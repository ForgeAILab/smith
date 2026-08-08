//! One real MCP server, end to end, over either transport. Opt-in, like every
//! other test that reaches outside the process.
//!
//! The hermetic suite proves the seams — admission, registration, namespacing,
//! failure isolation — against a fake connector. What it cannot prove is that
//! the transport, the protocol negotiation, and a real server's advertised
//! schemas survive the trip. That needs a real server, which is
//! developer-specific, so it is named by the environment and ignored by
//! default:
//!
//! ```text
//! SMITH_MCP_COMMAND=codegraph SMITH_MCP_ARGS='serve --mcp' \
//!   cargo test -p smith-runtime --test mcp_live -- --ignored --nocapture
//! ```
//!
//! A remote server is named by endpoint instead, and exercises the other
//! transport:
//!
//! ```text
//! SMITH_MCP_URL=http://127.0.0.1:3001/mcp \
//!   cargo test -p smith-runtime --test mcp_live -- --ignored --nocapture
//! ```
//!
//! Two things are worth proving separately: that a server connects and
//! advertises, and that one of the tools it advertised actually runs. A client
//! can list a server's catalogue perfectly and still fail every call.

use std::sync::Arc;
use std::time::Duration;

use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Deadline, SystemClock};
use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId};
use agent_runtime_core::tool::{InvocationContext, PreparationContext, Tool, ToolOutcome};
use agent_runtime_testkit::MemoryWorkspace;
use smith_config::resolve::{ResolveRequest, resolve};
use smith_config::trust::{TrustDecision, TrustStore};
use smith_runtime::mcp::{McpOptions, McpState, McpSupervisor};

const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;

/// The connected supervisor, plus the temporary roots it reads from — dropping
/// those deletes the config and the trust file out from under it, so the caller
/// holds them for as long as it holds the supervisor.
struct LiveServer {
    supervisor: Arc<McpSupervisor>,
    _home: tempfile::TempDir,
    _project: tempfile::TempDir,
}

/// The `[mcp.servers.live]` table the environment asks for.
///
/// `SMITH_MCP_URL` selects the streamable-HTTP transport; otherwise
/// `SMITH_MCP_COMMAND` spawns a local server. The two transports share every
/// seam after the connection, so a run of either exercises the rest.
fn declared_server() -> String {
    if let Ok(url) = std::env::var("SMITH_MCP_URL") {
        return format!("[mcp.servers.live]\nurl = \"{url}\"\n");
    }
    let Ok(command) = std::env::var("SMITH_MCP_COMMAND") else {
        panic!("set SMITH_MCP_COMMAND to the server's program, or SMITH_MCP_URL to its endpoint");
    };
    let args = std::env::var("SMITH_MCP_ARGS").unwrap_or_default();
    let rendered_args = args
        .split_whitespace()
        .map(|arg| format!("\"{arg}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[mcp.servers.live]\ncommand = \"{command}\"\nargs = [{rendered_args}]\n")
}

/// Declares the server named by the environment, trusts it, and waits for it.
async fn connect_named_server() -> LiveServer {
    let home = tempfile::tempdir().expect("a user root");
    let project = tempfile::tempdir().expect("a project root");
    std::fs::create_dir_all(project.path().join(".smith")).expect("a project `.smith`");
    std::fs::write(
        project.path().join(".smith/config.toml"),
        format!("{CONFIG}\n{}", declared_server()),
    )
    .expect("a project config");

    let resolution = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("a resolved project");
    let server = &resolution.config.mcp.servers["live"];

    // The same decision `/mcp trust` records, made here without a terminal.
    let mut trust = TrustStore::open(home.path()).expect("an empty store");
    trust
        .record(
            project.path(),
            &smith_config::mcp::executable(server),
            TrustDecision::Allow,
        )
        .expect("a recorded decision");

    let supervisor = McpSupervisor::plan(
        &resolution.config.mcp,
        &trust,
        McpOptions::new(project.path()),
    )
    .expect("a planned supervisor");
    supervisor.connect();
    supervisor.settle(Duration::from_secs(60)).await;

    LiveServer {
        supervisor,
        _home: home,
        _project: project,
    }
}

#[tokio::test]
#[ignore = "requires a real MCP server named by SMITH_MCP_COMMAND or SMITH_MCP_URL"]
async fn a_real_server_connects_and_contributes_namespaced_tools() {
    let live = connect_named_server().await;
    let supervisor = &live.supervisor;

    let report = supervisor.report("live").expect("a reported server");
    let tools: Vec<String> = supervisor
        .tools()
        .iter()
        .map(|tool: &Arc<dyn Tool>| tool.spec().name)
        .collect();
    println!("state: {:?}", report.state);
    println!("tools: {tools:?}");
    println!("refused: {:?}", report.rejected);

    assert!(
        matches!(report.state, McpState::Connected { .. }),
        "the server did not connect: {:?}",
        report.state
    );
    assert!(!tools.is_empty(), "a connected server advertised nothing");
    assert!(
        tools.iter().all(|name| name.starts_with("mcp__live__")),
        "every tool is namespaced by its server: {tools:?}"
    );

    supervisor.shutdown().await;
}

/// Discovery proves the catalogue arrived; this proves a call in it runs.
///
/// The tool to call is named bare — the `mcp__live__` prefix is Smith's, not
/// the server's — and its arguments are JSON, defaulting to none:
///
/// ```text
/// SMITH_MCP_COMMAND=codegraph SMITH_MCP_ARGS='serve --mcp' \
///   SMITH_MCP_TOOL=codegraph_status SMITH_MCP_TOOL_ARGS='{}' \
///   cargo test -p smith-runtime --test mcp_live -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "requires a real MCP server named by SMITH_MCP_COMMAND or SMITH_MCP_URL"]
async fn a_real_server_answers_a_tool_call() {
    let Ok(tool_name) = std::env::var("SMITH_MCP_TOOL") else {
        panic!("set SMITH_MCP_TOOL to a tool the server advertises, e.g. `codegraph_status`");
    };
    let arguments: serde_json::Value =
        serde_json::from_str(&std::env::var("SMITH_MCP_TOOL_ARGS").unwrap_or_else(|_| "{}".into()))
            .expect("SMITH_MCP_TOOL_ARGS is JSON");

    let live = connect_named_server().await;
    let supervisor = &live.supervisor;

    let namespaced = format!("mcp__live__{tool_name}");
    let tools = supervisor.tools();
    let tool = tools
        .iter()
        .find(|tool: &&Arc<dyn Tool>| tool.spec().name == namespaced)
        .unwrap_or_else(|| panic!("the server advertises no `{namespaced}`"));

    let outcome = invoke(tool.as_ref(), arguments).await;
    println!("value: {}", outcome.value);
    println!("content: {:?}", outcome.content);

    assert!(
        !outcome.is_error,
        "the server reported a tool error: {}",
        outcome.value
    );
    assert!(
        !outcome.content.is_empty() || !outcome.value.is_null(),
        "a successful call returned nothing at all"
    );

    supervisor.shutdown().await;
}

/// Runs one tool the way the runtime does: prepare, then invoke exactly what
/// was prepared.
async fn invoke(tool: &dyn Tool, arguments: serde_json::Value) -> ToolOutcome {
    let session = SessionId::new("live-session");
    let call_id = ToolCallId::new("live-call");
    let request = RequestId::new("live-request");
    let workspace = Arc::new(MemoryWorkspace::new("/repo"));
    let clock = Arc::new(SystemClock);
    let cancel = Cancellation::new();

    let preparation = PreparationContext {
        session: session.clone(),
        turn: None,
        call_id: call_id.clone(),
        request: request.clone(),
        workspace: workspace.clone(),
        clock: clock.clone(),
        cancel: cancel.clone(),
        deadline: Deadline::never(),
    };
    let prepared = tool
        .prepare(arguments, &preparation)
        .await
        .expect("a prepared call");

    let ctx = InvocationContext {
        session,
        turn: None,
        call_id,
        request,
        workspace,
        clock,
        cancel,
        deadline: Deadline::never(),
        output_limit: 100_000,
    };
    tool.invoke(prepared, &ctx).await.expect("a tool outcome")
}
