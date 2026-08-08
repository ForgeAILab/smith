//! One real stdio MCP server, end to end. Opt-in, like every other test that
//! reaches outside the process.
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

use std::sync::Arc;
use std::time::Duration;

use agent_runtime_core::tool::Tool;
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

#[tokio::test]
#[ignore = "requires a real MCP server named by SMITH_MCP_COMMAND"]
async fn a_real_stdio_server_connects_and_contributes_namespaced_tools() {
    let Ok(command) = std::env::var("SMITH_MCP_COMMAND") else {
        panic!("set SMITH_MCP_COMMAND to the server's program, e.g. `codegraph`");
    };
    let args: Vec<String> = std::env::var("SMITH_MCP_ARGS")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();

    let home = tempfile::tempdir().expect("a user root");
    let project = tempfile::tempdir().expect("a project root");
    std::fs::create_dir_all(project.path().join(".smith")).expect("a project `.smith`");
    let rendered_args = args
        .iter()
        .map(|arg| format!("\"{arg}\""))
        .collect::<Vec<_>>()
        .join(", ");
    std::fs::write(
        project.path().join(".smith/config.toml"),
        format!(
            "{CONFIG}\n[mcp.servers.live]\ncommand = \"{command}\"\nargs = [{rendered_args}]\n"
        ),
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
