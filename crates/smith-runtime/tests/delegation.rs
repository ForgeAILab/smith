//! Direct-child delegation through the one factory (harness tasks 7.1–7.3).
//!
//! Children are composed by [`SmithChildFactory`] through the same policy as
//! the parent, managed root-only through the shared runtime's coordinator,
//! and their results are routed into the parent's safe-boundary inbox so the
//! model receives them only at a provider/tool boundary.

use std::sync::{Arc, OnceLock};

use agent_runtime::delegation::{ChildState, SpawnOutcome};
use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, usage_event};
use agent_runtime::runtime::StartSession;
use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Deadline, SystemClock};
use agent_runtime_core::content::UserInput;
use agent_runtime_core::delegation::{
    ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
};
use agent_runtime_core::event::RuntimeEvent;
use agent_runtime_core::ids::{RequestId, ToolCallId};
use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
use agent_runtime_core::tool::{InvocationContext, Tool};
use agent_runtime_testkit::MemoryWorkspace;
use futures_util::StreamExt;
use smith_config::resolve::{ResolveRequest, ResolvedConfig, resolve};
use smith_runtime::delegation::{AGENT_TOOL_NAME, AgentTool, wire_delegation};
use smith_runtime::factory::{self, HostSurface, RuntimeRequest};

const FAKE_CONFIG: &str = r#"
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

[approval]
mode = "allow-all"
"#;

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("a user root");
        let project = tempfile::tempdir().expect("a project root");
        let dir = project.path().join(".smith");
        std::fs::create_dir_all(&dir).expect("a project `.smith`");
        std::fs::write(dir.join("config.toml"), FAKE_CONFIG).expect("a project config");
        Self { home, project }
    }

    fn config(&self) -> ResolvedConfig {
        resolve(&ResolveRequest::new(self.project.path()).with_home_dir(self.home.path()))
            .expect("a resolved configuration")
            .config
    }
}

/// A provider with `n` scripted text replies, shared by parent and children.
fn scripted(n: usize, text: &str) -> Arc<FakeProvider> {
    let scripts = (0..n)
        .map(|_| {
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta { text: text.into() },
                usage_event(5, 2),
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])
        })
        .collect();
    Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        scripts,
    ))
}

fn request(fixture: &Fixture, provider: Arc<FakeProvider>) -> RuntimeRequest {
    RuntimeRequest {
        workspace: Some(Arc::new(MemoryWorkspace::new("/repo"))),
        provider: Some(provider),
        ..RuntimeRequest::new(fixture.config(), HostSurface::Terminal)
    }
}

/// A child surface never composes the delegation tool.
#[tokio::test]
async fn a_child_surface_gets_no_delegation_tool() {
    let fixture = Fixture::new();
    let smith = factory::build(RuntimeRequest {
        workspace: Some(Arc::new(MemoryWorkspace::new("/repo"))),
        provider: Some(scripted(1, "child reply")),
        ..RuntimeRequest::new(fixture.config(), HostSurface::Child)
    })
    .await
    .expect("a child runtime");
    assert!(smith.delegation().is_none());
    assert!(
        !smith
            .policy()
            .tools
            .iter()
            .any(|name| name == AGENT_TOOL_NAME)
    );
}

/// The full root path: spawn through the coordinator, watch the attributed
/// lifecycle on the parent stream, and receive the child's result in the
/// parent's next provider request via the safe-boundary inbox.
#[tokio::test]
async fn a_spawned_child_completes_and_its_result_reaches_the_parent_model() {
    let fixture = Fixture::new();
    let provider = scripted(2, "the child's findings");
    let smith = factory::build(request(&fixture, provider.clone()))
        .await
        .expect("a runtime");

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    wire_delegation(&session, delegation).expect("delegation wires once");
    let coordinator = delegation.coordinator().expect("a coordinator");

    let mut events = session.subscribe();
    let outcome = coordinator
        .spawn(ChildSpec {
            task: UserInput::text("summarize the repo"),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(2),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("a spawn");
    let child = match outcome {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };

    // The attributed lifecycle reaches the parent stream, completed last.
    let mut saw_spawned = false;
    while let Some(envelope) = events.next().await {
        match envelope.payload {
            RuntimeEvent::ChildSpawned { child: id, .. } => {
                assert_eq!(id, child);
                saw_spawned = true;
            }
            RuntimeEvent::ChildCompleted { child: id, result } => {
                assert_eq!(id, child);
                assert_eq!(result, "the child's findings");
                break;
            }
            _ => {}
        }
    }
    assert!(saw_spawned);

    let status = coordinator.wait(&child).await.expect("a status");
    assert_eq!(status.state, ChildState::Idle);

    // The child's scoped view: read-only built-ins, and never the agent tool.
    let child_request = &provider.requests()[0];
    let names: Vec<&str> = child_request
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(names, ["read", "list", "search"]);

    // The injection task runs concurrently; give it a moment to enqueue, then
    // run a parent turn and assert the result arrived at the safe boundary.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    session
        .run(UserInput::text("what did the child find?"))
        .await;
    let parent_request = &provider.requests()[1];
    let injected = parent_request.messages.iter().any(|message| {
        let text = message.joined_text();
        text.contains("Sub-agent") && text.contains("the child's findings")
    });
    assert!(
        injected,
        "the child result must reach the parent model through the inbox"
    );

    session.shutdown().await.expect("a clean shutdown");
}

/// The model-facing `agent` tool drives the same coordinator: spawn, wait,
/// and list answer with structured JSON, and stop resolves a terminal state.
#[tokio::test]
async fn the_agent_tool_spawns_waits_and_lists() {
    let fixture = Fixture::new();
    let provider = scripted(1, "done");
    let smith = factory::build(request(&fixture, provider))
        .await
        .expect("a runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    wire_delegation(&session, delegation).expect("delegation wires once");

    let slot = Arc::new(OnceLock::new());
    slot.set(delegation.coordinator().expect("a coordinator").clone())
        .expect("an empty slot");
    let tool = AgentTool::new(slot);
    let ctx = InvocationContext {
        call_id: ToolCallId::new("call-1"),
        request: RequestId::new("req-1"),
        workspace: Arc::new(MemoryWorkspace::new("/repo")),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 100_000,
    };

    let spawned = tool
        .invoke(
            serde_json::json!({ "action": "spawn", "task": "do a thing" }),
            &ctx,
        )
        .await
        .expect("a spawn outcome");
    let spawned = serde_json::to_string(&spawned.into_result_block(
        ToolCallId::new("call-1"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("json");
    assert!(spawned.contains("child-1"), "{spawned}");

    let waited = tool
        .invoke(
            serde_json::json!({ "action": "wait", "child_id": "child-1" }),
            &ctx,
        )
        .await
        .expect("a wait outcome");
    let waited = serde_json::to_string(&waited.into_result_block(
        ToolCallId::new("call-2"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("json");
    assert!(waited.contains("idle"), "{waited}");
    assert!(waited.contains("done"), "{waited}");

    let listed = tool
        .invoke(serde_json::json!({ "action": "list" }), &ctx)
        .await
        .expect("a list outcome");
    let listed = serde_json::to_string(&listed.into_result_block(
        ToolCallId::new("call-3"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("json");
    assert!(listed.contains("child-1"), "{listed}");

    let stopped = tool
        .invoke(
            serde_json::json!({ "action": "stop", "child_id": "child-1" }),
            &ctx,
        )
        .await
        .expect("a stop outcome");
    let stopped = serde_json::to_string(&stopped.into_result_block(
        ToolCallId::new("call-4"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("json");
    assert!(stopped.contains("stopped"), "{stopped}");

    session.shutdown().await.expect("a clean shutdown");
}
