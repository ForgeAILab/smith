//! Direct-child delegation through the one factory (harness tasks 7.1–7.3).
//!
//! Children are composed by [`SmithChildFactory`] through the same policy as
//! the parent, managed root-only through the shared runtime's coordinator,
//! and their results are routed into the parent's safe-boundary inbox so the
//! model receives them only at a provider/tool boundary.

use std::sync::{Arc, OnceLock};

use agent_runtime::ability::descriptor::RiskLevel;
use agent_runtime::ability::{Ability, ToolAbility};
use agent_runtime::delegation::DELEGATION_PERMISSION;
use agent_runtime::delegation::{ChildState, ChildTaskOutcome, SpawnOutcome};
use agent_runtime::provider::fake::{
    FakeProvider, ScriptedStream, tool_call_fragments, usage_event,
};
use agent_runtime::registry::Permission;
use agent_runtime::runtime::StartSession;
use agent_runtime_core::artifact::{
    ArtifactError, ArtifactRead, ArtifactStore, MAX_ARTIFACT_READ_BYTES,
};
use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Deadline, SystemClock};
use agent_runtime_core::content::UserInput;
use agent_runtime_core::delegation::{
    ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::{RequestId, ToolCallId};
use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
use agent_runtime_core::tool::{InvocationContext, PreparationContext, Tool, ToolOutcome};
use agent_runtime_testkit::MemoryWorkspace;
use smith_config::resolve::{ResolveRequest, ResolvedConfig, resolve};
use smith_host::ProjectWorkspace;
use smith_runtime::artifact::SmithArtifactStore;
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

fn questionnaire_script(
    call: &str,
    question: &str,
    prompt: &str,
    sensitivity: &str,
) -> ScriptedStream {
    let arguments = serde_json::json!({
        "questions": [{
            "id": question,
            "header": "Choice",
            "prompt": prompt,
            "choices": [
                {"id": "one", "label": "One"},
                {"id": "two", "label": "Two"}
            ]
        }],
        "sensitivity": sensitivity,
    })
    .to_string();
    let mut events = tool_call_fragments(0, call, "ask_user", &arguments);
    events.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    ScriptedStream::new(events)
}

fn request(fixture: &Fixture, provider: Arc<FakeProvider>) -> RuntimeRequest {
    RuntimeRequest {
        workspace: Some(Arc::new(MemoryWorkspace::new("/repo"))),
        provider: Some(provider),
        ..RuntimeRequest::new(fixture.config(), HostSurface::Terminal)
    }
}

async fn invoke_agent(
    tool: &AgentTool,
    arguments: serde_json::Value,
    ctx: &InvocationContext,
) -> Result<ToolOutcome, RuntimeError> {
    let preparation = PreparationContext {
        session: ctx.session.clone(),
        turn: ctx.turn.clone(),
        call_id: ctx.call_id.clone(),
        request: ctx.request.clone(),
        workspace: ctx.workspace.clone(),
        clock: ctx.clock.clone(),
        cancel: ctx.cancel.clone(),
        deadline: ctx.deadline,
    };
    let prepared = tool.prepare(arguments, &preparation).await?;
    tool.invoke(prepared, ctx).await
}

#[test]
fn the_agent_ability_advertises_its_host_defined_delegation_authority() {
    let tool = Arc::new(AgentTool::new(Arc::new(OnceLock::new())));
    let descriptor = ToolAbility::new(tool).descriptor();

    assert_eq!(descriptor.risk(), RiskLevel::High);
    assert!(
        descriptor
            .permissions()
            .contains(&Permission::other(DELEGATION_PERMISSION.to_owned()))
    );
    assert!(
        descriptor
            .affordances()
            .iter()
            .any(|affordance| affordance.as_str() == "host-defined-authority")
    );

    let description = AgentTool::new(Arc::new(OnceLock::new())).spec().description;
    assert!(description.contains("does not open user interface"));
    assert!(description.contains("root ask_user"));
    assert!(description.contains("explicit follow_up"));
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

/// The full root path: spawn through the coordinator and receive the protected
/// final outcome in the parent's next provider request even when a one-slot
/// presentation stream cannot retain the lifecycle burst.
#[tokio::test]
async fn a_spawned_child_completes_and_its_result_reaches_the_parent_model() {
    let fixture = Fixture::new();
    let provider = scripted(3, "the child's findings");
    let mut runtime_request = request(&fixture, provider.clone());
    runtime_request.event_buffer = 1;
    let smith = factory::build(runtime_request)
        .await
        .expect("a runtime with a one-event presentation buffer");

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    wire_delegation(&session, delegation).expect("delegation wires once");
    let coordinator = delegation.coordinator().expect("a coordinator");

    let outcome = coordinator
        .spawn(ChildSpec {
            task: UserInput::text("inspect and explain the Rust source files in this repository"),
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

    let outcome = coordinator
        .wait_task_outcome(&child)
        .await
        .expect("a protected child outcome");
    assert!(matches!(
        &outcome,
        ChildTaskOutcome::Completed { child: id, result }
            if id == &child
                && result.text == "the child's findings"
                && result.artifacts.is_empty()
    ));
    assert_eq!(
        coordinator.status(&child).expect("a status").state,
        ChildState::Idle
    );

    // The child keeps live descriptor routing after the coordinator narrows
    // the executable view: at least one relevant read tool plus the protected
    // discovery bootstrap, and never a mutation/delegation/question surface.
    let child_request = &provider.requests()[0];
    let names: Vec<&str> = child_request
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    assert!(
        names
            .iter()
            .any(|name| matches!(*name, "read" | "list" | "search")),
        "{names:?}"
    );
    assert!(
        names
            .iter()
            .all(|name| matches!(*name, "read" | "list" | "search" | "registry.search")),
        "a narrowed child exposed a broader or orphaned descriptor: {names:?}"
    );

    // The injection task runs concurrently; give it a moment to enqueue, then
    // run a parent turn and assert the result arrived at the safe boundary.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    session
        .run(UserInput::text("what did the child find?"))
        .await
        .expect("the parent turn runs");
    let parent_request = provider.requests()[1].clone();
    let deliveries = parent_request
        .messages
        .iter()
        .filter_map(|message| {
            let text = message.joined_text();
            text.contains(r#""type":"child_task_outcome""#)
                .then(|| serde_json::from_str::<serde_json::Value>(&text).expect("typed delivery"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        deliveries.len(),
        1,
        "the completed outcome is injected exactly once"
    );
    assert_eq!(deliveries[0]["child_id"], child.as_str());
    assert_eq!(deliveries[0]["outcome"]["kind"], "completed");
    assert_eq!(
        deliveries[0]["outcome"]["result"]["text"],
        "the child's findings"
    );
    assert_eq!(
        deliveries[0]["outcome"]["result"]["artifacts"],
        serde_json::json!([])
    );

    session
        .run(UserInput::text("continue after consuming the child result"))
        .await
        .expect("the later parent turn runs");
    assert_eq!(
        provider.requests()[2]
            .messages
            .iter()
            .filter(|message| {
                message
                    .joined_text()
                    .contains(r#""type":"child_task_outcome""#)
            })
            .count(),
        1,
        "the later request may retain canonical history but must not inject the result twice"
    );
    assert_eq!(
        coordinator
            .task_outcome(&child)
            .expect("known child")
            .expect("retained exact outcome"),
        outcome,
        "automatic delivery consumed the idempotent status outcome"
    );

    session.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn a_child_artifact_is_explicitly_transferred_without_widening_source_ownership() {
    let fixture = Fixture::new();
    let command = "yes 'child-owned artifact line' | head -c 262144";
    let mut shell = tool_call_fragments(
        0,
        "child-large-shell",
        "shell",
        &serde_json::json!({ "command": command }).to_string(),
    );
    shell.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(shell),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "child artifact ready".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "parent received the transferred artifact".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let paths = smith_runtime::host::paths(&fixture.config(), fixture.project.path())
        .expect("protected Smith paths");
    let store = Arc::new(SmithArtifactStore::new(paths));
    let mut runtime_request = request(&fixture, provider.clone());
    runtime_request.workspace = Some(Arc::new(
        ProjectWorkspace::new(fixture.project.path()).expect("a project workspace"),
    ));
    runtime_request.artifact_store = Some(store.clone());
    let smith = factory::build(runtime_request)
        .await
        .expect("a runtime with protected artifact transfer");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a parent session");
    let delegation = smith.delegation().expect("a delegation surface");
    wire_delegation(&session, delegation).expect("delegation wires");
    let coordinator = delegation.coordinator().expect("a coordinator");

    let spawned = coordinator
        .spawn(ChildSpec {
            task: UserInput::text(
                "Use shell to produce a large result so it is retained as an artifact.",
            ),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(1),
            tools: ToolViewScope::All,
            workspace: WorkspacePolicy::SharedProject,
        })
        .await
        .expect("the child spawns");
    let child = match spawned {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };
    let outcome = coordinator
        .wait_task_outcome(&child)
        .await
        .expect("a completed child outcome");
    let transferred = match &outcome {
        ChildTaskOutcome::Completed { result, .. } => {
            assert_eq!(result.text, "child artifact ready");
            assert_eq!(result.artifacts.len(), 1);
            result.artifacts[0].clone()
        }
        other => panic!("expected a completed child outcome, got {other:?}"),
    };
    assert_eq!(transferred.provenance.session, *session.id());
    let source = transferred
        .provenance
        .derived_from
        .clone()
        .expect("the parent reference retains child lineage");
    assert_ne!(source.session, *session.id());
    assert_eq!(source.digest, transferred.digest);

    assert_eq!(
        store
            .read(ArtifactRead {
                session: session.id().clone(),
                id: source.id,
                offset: 0,
                limit: MAX_ARTIFACT_READ_BYTES,
            })
            .await,
        Err(ArtifactError::AccessDenied),
        "the transferred reference must not grant access to its child-owned source"
    );
    let page = store
        .read(ArtifactRead {
            session: session.id().clone(),
            id: transferred.id.clone(),
            offset: 0,
            limit: MAX_ARTIFACT_READ_BYTES,
        })
        .await
        .expect("the explicit parent-owned copy is readable");
    assert!(
        String::from_utf8_lossy(&page.bytes).contains("child-owned artifact line"),
        "the transferred parent copy preserves exact child output"
    );

    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    session
        .run(UserInput::text("inspect the completed child result"))
        .await
        .expect("the parent consumes the typed outcome");
    let parent_request = &provider.requests()[2];
    let delivery = parent_request
        .messages
        .iter()
        .map(|message| message.joined_text())
        .find(|text| text.contains(r#""type":"child_task_outcome""#))
        .expect("the child outcome is delivered at the parent boundary");
    let delivery: serde_json::Value =
        serde_json::from_str(&delivery).expect("a typed child result");
    assert_eq!(
        delivery["outcome"]["result"]["text"],
        "child artifact ready"
    );
    assert_eq!(
        delivery["outcome"]["result"]["artifacts"][0]["id"],
        transferred.id.as_str()
    );
    assert_eq!(
        delivery["outcome"]["result"]["artifacts"][0]["provenance"]["session"],
        session.id().as_str()
    );

    session.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn concurrent_child_needs_input_is_lossless_ordered_and_never_opens_root_ui() {
    const PRIVATE_PROMPT: &str = "PRIVATE CHILD PROMPT MUST NOT ENTER PARENT";
    let fixture = Fixture::new();
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            questionnaire_script(
                "ask-public",
                "public-direction",
                "Choose the public direction",
                "public",
            ),
            questionnaire_script(
                "ask-sensitive",
                "private-direction",
                PRIVATE_PROMPT,
                "sensitive",
            ),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "parent handled both child requests".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let mut runtime_request = request(&fixture, provider.clone());
    runtime_request.event_buffer = 1;
    let smith = factory::build(runtime_request)
        .await
        .expect("a runtime with a one-event presentation buffer");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a delegation surface");
    wire_delegation(&session, delegation).expect("delegation wires");
    let coordinator = delegation.coordinator().expect("a coordinator");

    let spawn = |task: &str| ChildSpec {
        task: UserInput::text(task),
        model: ChildModelSelection::Inherit,
        limits: ChildLimits::turns(1),
        tools: ToolViewScope::ReadOnly,
        workspace: WorkspacePolicy::ReadOnlyView,
    };
    let first = coordinator
        .spawn(spawn(
            "Ask the user to choose a public implementation direction",
        ))
        .await
        .expect("first child spawns");
    let second = coordinator
        .spawn(spawn(
            "Ask the user for the sensitive missing implementation detail",
        ))
        .await
        .expect("second child spawns");
    let child_ids = [first, second].map(|outcome| match outcome {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a running child, got {other:?}"),
    });

    let (first_outcome, second_outcome) =
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(
                coordinator.wait_task_outcome(&child_ids[0]),
                coordinator.wait_task_outcome(&child_ids[1])
            )
        })
        .await
        .expect("children do not wait on a root UI broker");
    for outcome in [first_outcome, second_outcome] {
        assert!(matches!(
            outcome.expect("a typed child outcome"),
            ChildTaskOutcome::NeedsInput { .. }
        ));
    }
    assert_eq!(
        provider.requests().len(),
        2,
        "a returned questionnaire must finish each child after the paired metadata result"
    );

    // Give the lossless coordinator waiter a scheduling boundary. This does
    // not depend on the one-slot parent event broadcast, which has already
    // seen a burst of child lifecycle events.
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    session
        .run(UserInput::text("handle the returned child questions"))
        .await
        .expect("the parent turn runs");
    let parent_request = provider.requests().last().cloned().expect("parent request");
    let deliveries = parent_request
        .messages
        .iter()
        .filter_map(|message| {
            let text = message.joined_text();
            (text.contains(r#""type":"child_task_outcome""#)
                && text.contains(r#""kind":"needs_input""#))
            .then(|| serde_json::from_str::<serde_json::Value>(&text).expect("typed delivery"))
        })
        .collect::<Vec<_>>();
    assert_eq!(deliveries.len(), 2, "each child outcome is injected once");
    let delivered_children = deliveries
        .iter()
        .map(|delivery| {
            delivery["outcome"]["needs_input"]["child"]
                .as_str()
                .expect("child attribution")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        delivered_children,
        ["child-1", "child-2"],
        "keyed peers are canonicalized independently of completion order"
    );
    let parent_wire =
        serde_json::to_string(&parent_request.messages).expect("serializable parent messages");
    assert!(parent_wire.contains("Choose the public direction"));
    assert!(
        !parent_wire.contains(PRIVATE_PROMPT),
        "a sensitive child prompt entered ordinary parent context"
    );

    for child in &child_ids {
        assert!(
            matches!(
                coordinator
                    .task_outcome(child)
                    .expect("known child")
                    .expect("retained protected outcome"),
                ChildTaskOutcome::NeedsInput { .. }
            ),
            "automatic delivery consumed the exact protected request"
        );
    }

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
        session: session.id().clone(),
        turn: None,
        call_id: ToolCallId::new("call-1"),
        request: RequestId::new("req-1"),
        workspace: Arc::new(MemoryWorkspace::new("/repo")),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 100_000,
    };

    let spawned = invoke_agent(
        &tool,
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

    let waited = invoke_agent(
        &tool,
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

    let listed = invoke_agent(&tool, serde_json::json!({ "action": "list" }), &ctx)
        .await
        .expect("a list outcome");
    let listed = serde_json::to_string(&listed.into_result_block(
        ToolCallId::new("call-3"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("json");
    assert!(listed.contains("child-1"), "{listed}");

    let stopped = invoke_agent(
        &tool,
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
