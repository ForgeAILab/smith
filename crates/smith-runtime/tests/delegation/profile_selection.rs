use super::*;

/// A root profile (`dev`, build posture) plus one child-enabled, read-only
/// (`review`) profile — the fixture the model-facing profile-selection tests
/// share.
const PROFILE_SELECTION_CONFIG: &str = r#"
default_profile = "dev"
profile_order = ["dev"]

[profiles.dev]
provider = "local"
model = "parent-model"
posture = "build"
use = ["main"]

[profiles.review]
provider = "local"
model = "review-model"
posture = "review"
use = ["child"]
instructions = "Review the requested scope; never write."

[providers.local]
kind = "fake"

[models."local/parent-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[models."local/review-model"]
context_tokens = 64000
max_input_tokens = 60000
max_output_tokens = 2048

[approval]
mode = "allow-all"
"#;

struct ProfileFixture {
    // Held only to keep its temporary directories alive for the test.
    _fixture: Fixture,
    root_config: ResolvedConfig,
    review_child_config: ResolvedConfig,
    review_option: AgentToolProfile,
}

fn profile_fixture() -> ProfileFixture {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.project.path().join(".smith/config.toml"),
        PROFILE_SELECTION_CONFIG,
    )
    .expect("a profile-selection config");
    let root =
        resolve(&ResolveRequest::new(fixture.project.path()).with_home_dir(fixture.home.path()))
            .expect("root config");
    let review = resolve(
        &ResolveRequest::new(fixture.project.path())
            .with_home_dir(fixture.home.path())
            .with_cli(Overrides {
                profile: Some("review".to_owned()),
                ..Overrides::default()
            })
            .with_profile_use(ProfileUse::Child),
    )
    .expect("review child config");
    let review_option = AgentToolProfile {
        name: review.config.agent.profile.name.clone(),
        revision: review.config.agent.profile.revision.clone(),
        provider: review.config.provider.name.value.clone(),
        model: agent_runtime_core::provider::ModelId::new(review.config.model.value.clone()),
    };
    ProfileFixture {
        _fixture: fixture,
        root_config: root.config,
        review_child_config: review.config,
        review_option,
    }
}

fn root_request_with_review_profile(
    pf: &ProfileFixture,
    provider: Arc<dyn Provider>,
) -> RuntimeRequest {
    let mut request = RuntimeRequest {
        workspace: Some(Arc::new(MemoryWorkspace::new("/repo"))),
        provider: Some(provider),
        ..RuntimeRequest::new(pf.root_config.clone(), HostSurface::Terminal)
    };
    request.child_profiles.push(ChildProfileRequest {
        config: pf.review_child_config.clone(),
        catalog_sources: Vec::new(),
    });
    request
}

fn invocation_context(session_id: agent_runtime_core::ids::SessionId) -> InvocationContext {
    InvocationContext {
        session: session_id,
        turn: None,
        call_id: ToolCallId::new("call-1"),
        request: RequestId::new("req-1"),
        workspace: Arc::new(MemoryWorkspace::new("/repo")),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 100_000,
    }
}

/// Spawns through the `agent` tool, waits for the child to finish, and
/// returns the tool names its provider request advertised. `SmithChildFactory`
/// is `pub(crate)`, so this — reading back what the model was actually
/// offered — is how an external integration test tells a write-capable child
/// view from a read-only one.
async fn spawn_and_collect_tool_names(
    tool: &AgentTool,
    ctx: &InvocationContext,
    coordinator: &agent_runtime::delegation::DelegationCoordinator,
    provider: &FakeProvider,
    arguments: serde_json::Value,
) -> Vec<String> {
    let spawned = invoke_agent(tool, arguments, ctx)
        .await
        .expect("a spawn outcome");
    assert!(!spawned.is_error, "{spawned:?}");
    let child_id = spawned.value["spawned"]
        .as_str()
        .expect("a spawned child id")
        .to_owned();
    let child = agent_runtime_core::ids::ChildId::new(child_id);
    coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the probe child completes");
    provider
        .requests()
        .last()
        .expect("at least one recorded provider request")
        .tools
        .iter()
        .map(|descriptor| descriptor.name.clone())
        .collect()
}

/// `agent spawn` may name a registered child-enabled profile and have it
/// resolve through the exact preflighted route `/agent <preset>` uses —
/// `SmithChildFactory::route_for` via `profile_route_key` — rather than
/// falling back to the parent's own inherited route.
#[tokio::test]
async fn agent_tool_spawn_resolves_a_named_profile_to_its_preflighted_route() {
    let pf = profile_fixture();
    let parent_provider = scripted(1, "root fallback must not run");
    let smith = factory::build_request(root_request_with_review_profile(
        &pf,
        parent_provider.clone(),
    ))
    .await
    .expect("a runtime with a review child profile");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wiring");
    let coordinator = delegation.coordinator().expect("a coordinator").clone();

    let slot = Arc::new(OnceLock::new());
    slot.set(coordinator.clone()).expect("an empty slot");
    let tool = AgentTool::new(slot).with_profiles(vec![pf.review_option.clone()]);

    // The schema and the description name the profile the model may choose.
    let spec = tool.spec();
    assert!(
        spec.input_schema["properties"]["profile"]["enum"]
            .as_array()
            .expect("a profile enum")
            .contains(&serde_json::json!("review")),
        "{}",
        spec.input_schema
    );
    assert!(spec.description.contains("review"), "{}", spec.description);

    let ctx = invocation_context(session.id().clone());
    let spawned = invoke_agent(
        &tool,
        serde_json::json!({
            "action": "spawn",
            "task": "review the change",
            "profile": "review",
        }),
        &ctx,
    )
    .await
    .expect("a spawn outcome");
    assert!(!spawned.is_error, "{spawned:?}");
    let child_id = spawned.value["spawned"]
        .as_str()
        .expect("a spawned child id")
        .to_owned();
    let child = agent_runtime_core::ids::ChildId::new(child_id);
    let outcome = coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the profile-routed child completes");
    assert!(
        matches!(outcome, ChildTaskOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert!(
        parent_provider.requests().is_empty(),
        "the profile-selected spawn fell back to the parent's inherited route"
    );

    session.shutdown().await.expect("a clean shutdown");
}

/// An unknown, non-child-enabled, or unrouted profile fails the spawn with a
/// tool error naming the available profiles, and creates no child — checked
/// before the coordinator is ever asked to spawn anything.
#[tokio::test]
async fn agent_tool_refuses_an_unavailable_profile_and_creates_no_child() {
    let pf = profile_fixture();
    let parent_provider = scripted(1, "root fallback must not run");
    let smith = factory::build_request(root_request_with_review_profile(&pf, parent_provider))
        .await
        .expect("a runtime with a review child profile");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wiring");
    let coordinator = delegation.coordinator().expect("a coordinator").clone();
    let ctx = invocation_context(session.id().clone());

    let slot = Arc::new(OnceLock::new());
    slot.set(coordinator.clone()).expect("an empty slot");
    let tool = AgentTool::new(slot).with_profiles(vec![pf.review_option.clone()]);
    let refused = invoke_agent(
        &tool,
        serde_json::json!({
            "action": "spawn",
            "task": "do a thing",
            "profile": "ghost",
        }),
        &ctx,
    )
    .await
    .expect("a structured tool error, not a hard failure");
    assert!(refused.is_error, "{refused:?}");
    let message = refused.value.as_str().expect("an error message");
    assert!(message.contains("ghost"), "{message}");
    assert!(message.contains("review"), "{message}");

    // No profile registered at all: the schema drops the `profile` property
    // entirely rather than advertising an empty enum, and the refusal says so.
    let bare_slot = Arc::new(OnceLock::new());
    bare_slot.set(coordinator.clone()).expect("an empty slot");
    let bare_tool = AgentTool::new(bare_slot);
    assert!(
        bare_tool.spec().input_schema["properties"]
            .get("profile")
            .is_none(),
        "{}",
        bare_tool.spec().input_schema
    );
    let bare_refused = invoke_agent(
        &bare_tool,
        serde_json::json!({
            "action": "spawn",
            "task": "do a thing",
            "profile": "anything",
        }),
        &ctx,
    )
    .await
    .expect("a structured tool error, not a hard failure");
    assert!(bare_refused.is_error, "{bare_refused:?}");
    assert!(
        bare_refused
            .value
            .as_str()
            .expect("an error message")
            .contains("none are registered"),
        "{bare_refused:?}"
    );

    assert!(
        coordinator.list().is_empty(),
        "a refused profile spawn must create no child and no lifecycle event"
    );

    session.shutdown().await.expect("a clean shutdown");
}

/// A spawn that names no profile keeps behaving exactly as it did before
/// profile selection existed, even once a directory of selectable profiles is
/// registered on the tool: it inherits the parent's own route.
#[tokio::test]
async fn agent_tool_spawn_without_a_profile_still_inherits_the_parents_route() {
    let pf = profile_fixture();
    let parent_provider = scripted(1, "root handled the inherited spawn");
    let smith = factory::build_request(root_request_with_review_profile(
        &pf,
        parent_provider.clone(),
    ))
    .await
    .expect("a runtime with a review child profile");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wiring");
    let coordinator = delegation.coordinator().expect("a coordinator").clone();
    let ctx = invocation_context(session.id().clone());

    let slot = Arc::new(OnceLock::new());
    slot.set(coordinator.clone()).expect("an empty slot");
    let tool = AgentTool::new(slot).with_profiles(vec![pf.review_option.clone()]);

    let spawned = invoke_agent(
        &tool,
        serde_json::json!({ "action": "spawn", "task": "inherit the parent's route" }),
        &ctx,
    )
    .await
    .expect("a spawn outcome");
    assert!(!spawned.is_error, "{spawned:?}");
    let child_id = spawned.value["spawned"]
        .as_str()
        .expect("a spawned child id")
        .to_owned();
    let child = agent_runtime_core::ids::ChildId::new(child_id);
    let outcome = coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the inherited child completes");
    assert!(
        matches!(
            &outcome,
            ChildTaskOutcome::Completed { result, .. }
                if result.text == "root handled the inherited spawn"
        ),
        "{outcome:?}"
    );
    assert_eq!(
        parent_provider.requests().len(),
        1,
        "an absent profile argument must still consume the parent's own inherited route"
    );

    session.shutdown().await.expect("a clean shutdown");
}

/// The posture/scope/workspace matrix for a build-posture route: write tools
/// reach a child only when a full tool scope and a non-read-only workspace
/// are both declared; either one missing leaves it read-only.
#[tokio::test]
async fn child_write_access_needs_full_scope_and_workspace_together() {
    let fixture = Fixture::new();
    let provider = scripted(3, "write-access probe");
    let smith = factory::build_request(request(&fixture, provider.clone()))
        .await
        .expect("a build-posture runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wiring");
    let coordinator = delegation.coordinator().expect("a coordinator").clone();
    let ctx = invocation_context(session.id().clone());

    let slot = Arc::new(OnceLock::new());
    slot.set(coordinator.clone()).expect("an empty slot");
    let tool = AgentTool::new(slot);

    // (build posture, tools: "all", workspace: "shared") -> write tools present.
    // Progressive capability discovery activates only a relevance-ranked
    // subset of the registered tools per turn (`agent_runtime::capability`),
    // so this checks that a matching write tool is reachable rather than
    // that every registered write tool is advertised on this one turn.
    let full_scope_shared = spawn_and_collect_tool_names(
        &tool,
        &ctx,
        &coordinator,
        &provider,
        serde_json::json!({
            "action": "spawn",
            "task": "edit something",
            "tools": "all",
            "workspace": "shared",
        }),
    )
    .await;
    assert!(
        full_scope_shared.contains(&"edit".to_owned()),
        "{full_scope_shared:?}"
    );

    // (build posture, default read-only tool scope, workspace: "shared") -> no write tools.
    let default_scope_shared = spawn_and_collect_tool_names(
        &tool,
        &ctx,
        &coordinator,
        &provider,
        serde_json::json!({
            "action": "spawn",
            "task": "look something up",
            "workspace": "shared",
        }),
    )
    .await;
    assert!(
        !default_scope_shared.contains(&"edit".to_owned())
            && !default_scope_shared.contains(&"shell".to_owned()),
        "{default_scope_shared:?}"
    );

    // (build posture, tools: "all", default (read-only) workspace) -> no write
    // tools. This is the dangerous default: a spawn that asks for "all" but
    // never names a workspace must not silently become write-capable.
    let full_scope_default_workspace = spawn_and_collect_tool_names(
        &tool,
        &ctx,
        &coordinator,
        &provider,
        serde_json::json!({
            "action": "spawn",
            "task": "edit something without naming a workspace",
            "tools": "all",
        }),
    )
    .await;
    assert!(
        !full_scope_default_workspace.contains(&"edit".to_owned())
            && !full_scope_default_workspace.contains(&"shell".to_owned()),
        "{full_scope_default_workspace:?}"
    );

    session.shutdown().await.expect("a clean shutdown");
}

/// A root config with a review (read-only) posture — no explicit `use`
/// declaration, so it is the profile every spawn inherits by default.
const READ_ONLY_POSTURE_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"
posture = "review"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[approval]
mode = "allow-all"
"#;

/// A read-only-posture route stays read-only even when a spawn declares a
/// full tool scope and a non-read-only workspace: the declared scope cannot
/// widen what the posture withheld.
#[tokio::test]
async fn child_write_access_stays_read_only_under_a_read_only_posture() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.project.path().join(".smith/config.toml"),
        READ_ONLY_POSTURE_CONFIG,
    )
    .expect("a read-only-posture config");
    let provider = scripted(1, "write-access probe");
    let smith = factory::build_request(request(&fixture, provider.clone()))
        .await
        .expect("a read-only-posture runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wiring");
    let coordinator = delegation.coordinator().expect("a coordinator").clone();
    let ctx = invocation_context(session.id().clone());

    let slot = Arc::new(OnceLock::new());
    slot.set(coordinator.clone()).expect("an empty slot");
    let tool = AgentTool::new(slot);

    let names = spawn_and_collect_tool_names(
        &tool,
        &ctx,
        &coordinator,
        &provider,
        serde_json::json!({
            "action": "spawn",
            "task": "edit something",
            "tools": "all",
            "workspace": "shared",
        }),
    )
    .await;
    assert!(
        !names.contains(&"edit".to_owned()) && !names.contains(&"shell".to_owned()),
        "{names:?}"
    );

    session.shutdown().await.expect("a clean shutdown");
}
