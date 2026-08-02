//! Direct-child delegation through the one factory (harness tasks 7.1–7.3).
//!
//! Children are composed by [`SmithChildFactory`] through the same policy as
//! the parent, managed root-only through the shared runtime's coordinator,
//! and their results are routed into the parent's safe-boundary inbox so the
//! model receives them only at a provider/tool boundary.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use agent_runtime::ability::descriptor::RiskLevel;
use agent_runtime::ability::{Ability, ToolAbility};
use agent_runtime::delegation::DELEGATION_PERMISSION;
use agent_runtime::delegation::{
    CHILD_CATALOG_NAMESPACE, ChildDurability, ChildState, ChildTaskOutcome, DurableChildCatalog,
    SpawnOutcome,
};
use agent_runtime::provider::fake::{
    FakeProvider, ScriptedStream, tool_call_fragments, usage_event,
};
use agent_runtime::registry::Permission;
use agent_runtime::runtime::StartSession;
use agent_runtime_core::artifact::{
    ArtifactError, ArtifactRead, ArtifactStore, MAX_ARTIFACT_READ_BYTES,
};
use agent_runtime_core::cancel::CancelReason;
use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Deadline, SystemClock};
use agent_runtime_core::content::UserInput;
use agent_runtime_core::delegation::{
    ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId};
use agent_runtime_core::provider::{
    Capabilities, FinishReason, ModelDescriptor, Provider, ProviderCallContext, ProviderError,
    ProviderRequest, ProviderStream, ProviderStreamEvent,
};
use agent_runtime_core::store::SessionStore;
use agent_runtime_core::tool::{InvocationContext, PreparationContext, Tool, ToolOutcome};
use agent_runtime_testkit::{InMemoryCheckpointStore, InMemorySessionStore, MemoryWorkspace};
use async_trait::async_trait;
use futures_util::StreamExt;
use smith_config::model::ProfileUse;
use smith_config::resolve::{Overrides, ResolveRequest, ResolvedConfig, resolve};
use smith_host::ProjectWorkspace;
use smith_runtime::artifact::SmithArtifactStore;
use smith_runtime::delegation::{AGENT_TOOL_NAME, AgentTool, profile_route_key, wire_delegation};
use smith_runtime::factory::{self, ChildProfileRequest, HostSurface, RuntimeRequest};
use smith_runtime::project_instructions::ProjectInstructionsSnapshot;

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

#[derive(Debug)]
struct CrashThenReplyProvider {
    calls: AtomicUsize,
    entered: tokio::sync::Notify,
    requests: Mutex<Vec<ProviderRequest>>,
}

impl CrashThenReplyProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            entered: tokio::sync::Notify::new(),
            requests: Mutex::new(Vec::new()),
        }
    }

    async fn wait_for_calls(&self, expected: usize) {
        while self.calls.load(Ordering::SeqCst) < expected {
            self.entered.notified().await;
        }
    }

    fn requests(&self) -> Vec<ProviderRequest> {
        self.requests
            .lock()
            .expect("provider requests poisoned")
            .clone()
    }
}

#[async_trait]
impl Provider for CrashThenReplyProvider {
    fn describe(&self) -> Vec<ModelDescriptor> {
        Vec::new()
    }

    fn capabilities(&self, _model: &agent_runtime_core::provider::ModelId) -> Option<Capabilities> {
        Some(Capabilities::basic_streaming())
    }

    async fn stream(
        &self,
        request: ProviderRequest,
        _ctx: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        self.requests
            .lock()
            .expect("provider requests poisoned")
            .push(request);
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_waiters();
        if call == 0 {
            Ok(Box::pin(futures_util::stream::pending()))
        } else {
            Ok(Box::pin(futures_util::stream::iter(vec![
                ProviderStreamEvent::TextDelta {
                    text: "resumed exact child".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])))
        }
    }
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

fn request(fixture: &Fixture, provider: Arc<dyn Provider>) -> RuntimeRequest {
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

#[tokio::test]
async fn a_child_enabled_profile_uses_its_preflighted_alternate_model_route() {
    let fixture = Fixture::new();
    let config = r#"
default_profile = "dev"
profile_order = ["dev"]

[profiles.dev]
provider = "local"
model = "parent-model"
posture = "build"
use = ["main"]

[profiles.audit]
provider = "local"
model = "audit-model"
posture = "review"
use = ["child"]
instructions = "Audit the requested scope and report evidence."

[providers.local]
kind = "fake"

[models."local/parent-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[models."local/audit-model"]
context_tokens = 64000
max_input_tokens = 60000
max_output_tokens = 2048

[approval]
mode = "allow-all"
"#;
    std::fs::write(fixture.project.path().join(".smith/config.toml"), config)
        .expect("profile config");
    let root =
        resolve(&ResolveRequest::new(fixture.project.path()).with_home_dir(fixture.home.path()))
            .expect("root profile");
    let child = resolve(
        &ResolveRequest::new(fixture.project.path())
            .with_home_dir(fixture.home.path())
            .with_cli(Overrides {
                profile: Some("audit".to_owned()),
                ..Overrides::default()
            })
            .with_profile_use(ProfileUse::Child),
    )
    .expect("child profile");
    let route = profile_route_key(
        &child.config.agent.profile.name,
        &child.config.agent.profile.revision,
    );
    let parent_provider = scripted(1, "root fallback must not run");
    let mut request = RuntimeRequest {
        workspace: Some(Arc::new(MemoryWorkspace::new("/repo"))),
        provider: Some(parent_provider.clone()),
        ..RuntimeRequest::new(root.config, HostSurface::Terminal)
    };
    request.child_profiles.push(ChildProfileRequest {
        config: child.config,
        catalog_sources: Vec::new(),
    });

    let smith = factory::build(request)
        .await
        .expect("root with alternate child route");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("root session");
    let delegation = smith.delegation().expect("delegation surface");
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wiring");
    let coordinator = delegation.coordinator().expect("coordinator");
    let child_id = match coordinator
        .spawn(ChildSpec {
            task: UserInput::text("audit the parser"),
            model: ChildModelSelection::Explicit {
                provider: Some(route),
                model: agent_runtime_core::provider::ModelId::new("audit-model"),
            },
            limits: ChildLimits::turns(1),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("alternate-profile child spawn")
    {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected an alternate-profile child, got {other:?}"),
    };
    assert!(matches!(
        coordinator
            .wait_task_outcome(&child_id)
            .await
            .expect("alternate-profile child outcome"),
        ChildTaskOutcome::Completed { .. }
    ));
    assert!(
        parent_provider.requests().is_empty(),
        "the child silently fell back to the root provider/model route"
    );
    session.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_durable_child_accepts_a_follow_up_after_parent_restart_with_prior_history() {
    let fixture = Fixture::new();
    let provider = scripted(2, "same specialist");
    let sessions = Arc::new(InMemorySessionStore::new());
    let checkpoints = Arc::new(InMemoryCheckpointStore::new());
    let parent_id = SessionId::new("durable-parent");

    let build = || {
        let mut request = request(&fixture, provider.clone());
        request.session_store = Some(sessions.clone());
        request.checkpoint_store = Some(checkpoints.clone());
        request
    };

    let first = factory::build(build())
        .await
        .expect("the first Smith runtime");
    let first_session = first
        .runtime()
        .start_session(StartSession::new().with_id(parent_id.clone()))
        .await
        .expect("the first parent session");
    let first_delegation = first.delegation().expect("a root delegation surface");
    wire_delegation(&first_session, first_delegation)
        .await
        .expect("first delegation wiring");
    let first_coordinator = first_delegation.coordinator().expect("first coordinator");
    let child = match first_coordinator
        .spawn(ChildSpec {
            task: UserInput::text("Inspect the parser and remember the important constraints."),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(3),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the durable child spawns")
    {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };
    first_coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the first child task completes");
    let before = first_coordinator
        .status(&child)
        .expect("first child status");
    assert_eq!(before.durability, ChildDurability::Durable);
    first_coordinator
        .flush()
        .await
        .expect("the child catalog flushes");
    first_session
        .shutdown()
        .await
        .expect("the first parent shuts down");
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    let second = factory::build(build())
        .await
        .expect("the resumed Smith runtime");
    let second_session = second
        .runtime()
        .start_session(StartSession::new().with_id(parent_id))
        .await
        .expect("the parent snapshot resumes");
    let second_delegation = second.delegation().expect("a resumed delegation surface");
    wire_delegation(&second_session, second_delegation)
        .await
        .expect("resumed delegation wiring");
    let second_coordinator = second_delegation
        .coordinator()
        .expect("resumed coordinator");
    let recovered = second_coordinator
        .status(&child)
        .expect("the same child is restored");
    assert_eq!(recovered.child, before.child);
    assert_eq!(recovered.session, before.session);
    assert_eq!(recovered.state, ChildState::Idle);
    assert_eq!(recovered.turns_used, 1);

    second_coordinator
        .follow_up(
            &child,
            UserInput::text("Now identify the highest-risk regression based on that review."),
        )
        .await
        .expect("the recovered child accepts a new turn");
    second_coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the follow-up completes");
    let after = second_coordinator.status(&child).expect("follow-up status");
    assert_eq!(after.session, before.session);
    assert_eq!(after.turns_used, 2);

    let follow_up_request = provider.requests()[1].clone();
    let wire = serde_json::to_string(&follow_up_request.messages).expect("provider messages");
    assert!(wire.contains("Inspect the parser"), "{wire}");
    assert!(wire.contains("same specialist"), "{wire}");
    assert!(wire.contains("highest-risk regression"), "{wire}");

    second_coordinator
        .flush()
        .await
        .expect("the resumed catalog flushes");
    second_session
        .shutdown()
        .await
        .expect("the resumed parent shuts down");
}

#[tokio::test]
async fn a_returned_child_question_survives_smith_restart_without_provider_work() {
    let fixture = Fixture::new();
    let text_script = |text: &str| {
        ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta { text: text.into() },
            usage_event(5, 2),
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])
    };
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            questionnaire_script(
                "ask-after-restart",
                "direction",
                "Choose the implementation direction",
                "public",
            ),
            text_script("parent surfaced the recovered question"),
            text_script("child continued with the answer"),
        ],
    ));
    let sessions = Arc::new(InMemorySessionStore::new());
    let checkpoints = Arc::new(InMemoryCheckpointStore::new());
    let parent_id = SessionId::new("question-parent");
    let build = || {
        let mut request = request(&fixture, provider.clone());
        request.session_store = Some(sessions.clone());
        request.checkpoint_store = Some(checkpoints.clone());
        request
    };

    let first = factory::build(build())
        .await
        .expect("the first Smith runtime");
    let first_session = first
        .runtime()
        .start_session(StartSession::new().with_id(parent_id.clone()))
        .await
        .expect("the first parent session");
    let first_delegation = first.delegation().expect("a root delegation surface");
    wire_delegation(&first_session, first_delegation)
        .await
        .expect("first delegation wiring");
    let first_coordinator = first_delegation.coordinator().expect("first coordinator");
    let (child, child_session) = match first_coordinator
        .spawn(ChildSpec {
            task: UserInput::text("Ask which implementation direction to use."),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(3),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the durable child spawns")
    {
        SpawnOutcome::Spawned { child, handle } => (child, handle.id().clone()),
        other => panic!("expected a spawned child, got {other:?}"),
    };
    assert!(matches!(
        first_coordinator
            .wait_task_outcome(&child)
            .await
            .expect("the child returns its question"),
        ChildTaskOutcome::NeedsInput { .. }
    ));
    first_coordinator.flush().await.expect("catalog flushes");
    first_session.shutdown().await.expect("first parent stops");
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    let second = factory::build(build())
        .await
        .expect("the resumed Smith runtime");
    let second_session = second
        .runtime()
        .start_session(StartSession::new().with_id(parent_id))
        .await
        .expect("the parent resumes");
    let second_delegation = second.delegation().expect("a resumed delegation surface");
    wire_delegation(&second_session, second_delegation)
        .await
        .expect("protected child question recovers");
    let second_coordinator = second_delegation
        .coordinator()
        .expect("resumed coordinator");
    assert_eq!(
        provider.requests().len(),
        1,
        "Smith startup recovered the question without a provider call"
    );
    assert!(matches!(
        second_coordinator
            .task_outcome(&child)
            .expect("known recovered child"),
        Some(ChildTaskOutcome::NeedsInput { .. })
    ));

    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    second_session
        .run(UserInput::text("surface the recovered child question"))
        .await
        .expect("the root consumes the recovered outcome");
    let root_wire = serde_json::to_string(&provider.requests()[1].messages).expect("messages");
    assert!(
        root_wire.contains(r#"\"kind\":\"needs_input\""#),
        "{root_wire}"
    );
    assert!(
        root_wire.contains("Choose the implementation direction"),
        "{root_wire}"
    );

    second_coordinator
        .follow_up(&child, UserInput::text("Use direction one."))
        .await
        .expect("the answer starts a new turn on the same child");
    second_coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the child follow-up completes");
    let status = second_coordinator.status(&child).expect("child status");
    assert_eq!(status.session, child_session);
    assert_eq!(status.turns_used, 2);
    assert_eq!(
        status.last_result.as_deref(),
        Some("child continued with the answer")
    );
    assert_eq!(provider.requests().len(), 3);
    second_session.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn an_interrupted_smith_child_resumes_exactly_once_and_never_on_startup() {
    let fixture = Fixture::new();
    let provider = Arc::new(CrashThenReplyProvider::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let crash_checkpoints = Arc::new(InMemoryCheckpointStore::new());
    let parent_id = SessionId::new("crashed-parent");
    let build = |checkpoint_store: Arc<InMemoryCheckpointStore>| {
        let mut request = request(&fixture, provider.clone());
        request.session_store = Some(sessions.clone());
        request.checkpoint_store = Some(checkpoint_store);
        request
    };

    let first = factory::build(build(crash_checkpoints.clone()))
        .await
        .expect("the first Smith runtime");
    let first_session = first
        .runtime()
        .start_session(StartSession::new().with_id(parent_id.clone()))
        .await
        .expect("the first parent");
    let first_delegation = first.delegation().expect("the first delegation surface");
    wire_delegation(&first_session, first_delegation)
        .await
        .expect("first delegation wiring");
    let first_coordinator = first_delegation.coordinator().expect("first coordinator");
    let (child, abandoned_handle) = match first_coordinator
        .spawn(ChildSpec {
            task: UserInput::text("Keep analyzing until the process interruption."),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(2),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the first child starts")
    {
        SpawnOutcome::Spawned { child, handle } => (child, handle),
        other => panic!("expected a spawned child, got {other:?}"),
    };
    provider.wait_for_calls(1).await;
    first_coordinator
        .flush()
        .await
        .expect("the running catalog and checkpoint flush");
    let before = first_coordinator
        .status(&child)
        .expect("running child status");
    assert_eq!(before.state, ChildState::Running);
    assert_eq!(before.turns_used, 1);

    // The first protected boundary is Accepted, before provider I/O. Model a
    // crash at exactly that safe point by retaining it in a fresh protected
    // store and aligning the parent-owned watermark. The later CallingModel
    // boundary is deliberately non-resumable because its provider outcome is
    // indeterminate and replay could double-spend.
    let accepted = crash_checkpoints
        .history(&before.session)
        .into_iter()
        .find(|checkpoint| {
            matches!(
                checkpoint.state,
                agent_runtime_core::checkpoint::TurnState::Accepted { .. }
            )
        })
        .expect("an accepted child checkpoint precedes provider I/O");
    let resume_checkpoints = Arc::new(InMemoryCheckpointStore::new());
    resume_checkpoints
        .seed(accepted.clone())
        .expect("the accepted crash boundary is retained");
    let mut parent_snapshot = sessions
        .load(&parent_id)
        .await
        .expect("the parent snapshot loads")
        .expect("the child catalog was persisted on the parent");
    let catalog_state = parent_snapshot
        .extension_state
        .get_mut(CHILD_CATALOG_NAMESPACE)
        .expect("the durable child catalog exists");
    let mut catalog: DurableChildCatalog =
        serde_json::from_value(catalog_state.value.clone()).expect("a valid child catalog");
    let record = catalog
        .children
        .first_mut()
        .expect("the child record exists");
    assert!(
        !record.checkpoint_resumable,
        "an indeterminate in-flight provider call was advertised as resumable"
    );
    record.checkpoint_watermark = Some(accepted.watermark);
    record.checkpoint_resumable = true;
    catalog_state.value = serde_json::to_value(catalog).expect("the fixture catalog serializes");
    sessions
        .save(&parent_snapshot)
        .await
        .expect("the accepted crash catalog is committed");

    // Starting a new owner over the persisted records models process loss:
    // no orderly child cancellation or terminal checkpoint occurred.
    let second = factory::build(build(resume_checkpoints))
        .await
        .expect("the replacement Smith runtime");
    let second_session = second
        .runtime()
        .start_session(StartSession::new().with_id(parent_id))
        .await
        .expect("the replacement parent");
    let second_delegation = second.delegation().expect("replacement delegation surface");
    wire_delegation(&second_session, second_delegation)
        .await
        .expect("replacement delegation wiring");
    let second_coordinator = second_delegation
        .coordinator()
        .expect("replacement coordinator");
    let recovered = second_coordinator
        .status(&child)
        .expect("interrupted child restored");
    assert!(matches!(
        recovered.state,
        ChildState::Interrupted { resumable: true }
    ));
    assert_eq!(recovered.session, before.session);
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "startup spent tokens"
    );

    let slot = Arc::new(OnceLock::new());
    slot.set(second_coordinator.clone())
        .expect("an empty agent slot");
    let tool = AgentTool::new(slot);
    let mut resumed_events = second_session.subscribe();
    let ctx = InvocationContext {
        session: second_session.id().clone(),
        turn: None,
        call_id: ToolCallId::new("resume-call"),
        request: RequestId::new("resume-request"),
        workspace: Arc::new(MemoryWorkspace::new("/repo")),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 100_000,
    };
    let resumed = invoke_agent(
        &tool,
        serde_json::json!({ "action": "resume", "child_id": child.as_str() }),
        &ctx,
    )
    .await
    .expect("the explicit resume operation returns");
    let resumed_wire = serde_json::to_string(&resumed.into_result_block(
        ToolCallId::new("resume-call"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("resume JSON");
    assert!(resumed_wire.contains("exact_checkpoint"), "{resumed_wire}");

    let outcome = match second_coordinator.wait_task_outcome(&child).await {
        Ok(outcome) => outcome,
        Err(error) => {
            let mut observed = Vec::new();
            while let Ok(Some(event)) =
                tokio::time::timeout(std::time::Duration::from_millis(10), resumed_events.next())
                    .await
            {
                observed.push(format!("{:?}", event.payload));
            }
            panic!(
                "the exact resumed turn completes: {error:?}; status={:?}; events={observed:?}",
                second_coordinator.status(&child)
            );
        }
    };
    assert!(matches!(
        outcome,
        ChildTaskOutcome::Completed { ref result, .. }
            if result.text == "resumed exact child"
    ));
    let after = second_coordinator.status(&child).expect("resumed status");
    assert_eq!(after.turns_used, 1, "resume consumed a new task slot");
    assert_eq!(after.session, before.session);
    assert_eq!(provider.requests().len(), 2);

    let duplicate = invoke_agent(
        &tool,
        serde_json::json!({ "action": "resume", "child_id": child.as_str() }),
        &ctx,
    )
    .await
    .expect("duplicate resume returns a structured tool error");
    let duplicate_wire = serde_json::to_string(&duplicate.into_result_block(
        ToolCallId::new("duplicate-resume"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("duplicate resume JSON");
    assert!(duplicate_wire.contains("no compatible interrupted checkpoint"));
    assert_eq!(
        provider.requests().len(),
        2,
        "duplicate resume called the provider"
    );

    second_coordinator
        .flush()
        .await
        .expect("replacement catalog flushes");
    second_session
        .shutdown()
        .await
        .expect("replacement parent shuts down");
    abandoned_handle.cancel(CancelReason::Shutdown);
    let _ = abandoned_handle.shutdown().await;
    let _ = first_session.shutdown().await;
}

/// The full root path: spawn through the coordinator and receive the protected
/// final outcome in the parent's next provider request even when a one-slot
/// presentation stream cannot retain the lifecycle burst.
#[tokio::test]
async fn a_spawned_child_completes_and_its_result_reaches_the_parent_model() {
    let fixture = Fixture::new();
    let provider = scripted(3, "the child's findings");
    let mut runtime_request = request(&fixture, provider.clone());
    let project_instructions =
        ProjectInstructionsSnapshot::from_body("SHARED_PARENT_CHILD_INSTRUCTIONS")
            .expect("bounded project instructions");
    runtime_request.project_instructions = Some(project_instructions.clone());
    runtime_request.event_buffer = 1;
    let smith = factory::build(runtime_request)
        .await
        .expect("a runtime with a one-event presentation buffer");
    assert_eq!(
        smith.policy().project_instructions.as_ref(),
        Some(&project_instructions.identity())
    );

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wires once");
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
    let child_wire = serde_json::to_string(&child_request.messages).expect("child messages");
    assert!(
        child_wire.contains("SHARED_PARENT_CHILD_INSTRUCTIONS"),
        "{child_wire}"
    );
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
    let parent_wire = serde_json::to_string(&parent_request.messages).expect("parent messages");
    assert!(
        parent_wire.contains("SHARED_PARENT_CHILD_INSTRUCTIONS"),
        "{parent_wire}"
    );
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
    let large_fixture = "child-owned artifact line\n".repeat(10_000);
    std::fs::write(
        fixture.project.path().join("child-artifact.txt"),
        large_fixture,
    )
    .expect("large read-only child fixture");
    let mut read = tool_call_fragments(
        0,
        "child-large-read",
        "read",
        &serde_json::json!({ "path": "child-artifact.txt", "limit": 10_000 }).to_string(),
    );
    read.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(read),
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
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wires");
    let coordinator = delegation.coordinator().expect("a coordinator");

    let spawned = coordinator
        .spawn(ChildSpec {
            task: UserInput::text(
                "Read the large fixture so its bounded result is retained as an artifact.",
            ),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(1),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
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
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wires");
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
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wires once");

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
