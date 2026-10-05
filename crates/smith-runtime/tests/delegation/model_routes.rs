use super::*;

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

    let smith = factory::build_request(request)
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

/// A child profile that names an installed agent must run its turns on that
/// agent. The provider such a profile resolves exists only to supply model
/// identity and the limits the runtime plans against; asking it for the
/// `cli/...` model id is how this used to fail, with that provider's
/// rejection of an unknown model standing in for the agent that was never
/// launched.
#[tokio::test]
async fn a_child_profile_naming_an_installed_agent_runs_its_turn_on_that_agent() {
    let fixture = Fixture::new();
    let argv = fixture.project.path().join("argv.txt");
    let cli = fake_claude_cli(fixture.home.path(), &argv);
    let user = fixture.home.path().join(".smith");
    std::fs::create_dir_all(&user).expect("a user `.smith`");
    // The executable is owner-controlled, so it has to come from the user
    // layer: a project may select an installed agent but not declare what
    // Smith executes.
    std::fs::write(
        user.join("config.toml"),
        format!(
            "[harness.claude-code]\nexecutable = \"{}\"\nallow_own_tools = true\n",
            cli.display()
        ),
    )
    .expect("a user config");
    let config = r#"
default_profile = "dev"
profile_order = ["dev"]

[profiles.dev]
provider = "local"
model = "parent-model"
posture = "build"
use = ["main"]

[profiles.agent]
provider = "local"
model = "cli/claude-code/sonnet"
posture = "build"
use = ["child"]

[providers.local]
kind = "fake"

[models."local/parent-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

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
                profile: Some("agent".to_owned()),
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
        workspace: Some(Arc::new(
            ProjectWorkspace::new(fixture.project.path()).expect("a project workspace"),
        )),
        provider: Some(parent_provider.clone()),
        ..RuntimeRequest::new(root.config, HostSurface::Terminal)
    };
    request.child_profiles.push(ChildProfileRequest {
        config: child.config,
        catalog_sources: Vec::new(),
    });

    let smith = factory::build_request(request)
        .await
        .expect("root with an installed-agent child route");
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
            task: UserInput::text("introduce yourself"),
            model: ChildModelSelection::Explicit {
                provider: Some(route),
                model: agent_runtime_core::provider::ModelId::new("cli/claude-code/sonnet"),
            },
            limits: ChildLimits::turns(1),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("an installed-agent child spawn")
    {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected an installed-agent child, got {other:?}"),
    };
    let outcome = coordinator
        .wait_task_outcome(&child_id)
        .await
        .expect("installed-agent child outcome");
    let ChildTaskOutcome::Completed { result, .. } = outcome else {
        panic!("expected a completed installed-agent child, got {outcome:?}");
    };
    assert!(
        result.text.contains("ran on the installed agent"),
        "the child answered from somewhere other than the CLI: {}",
        result.text
    );
    let recorded = std::fs::read_to_string(&argv).expect("the CLI recorded its arguments");
    let recorded: Vec<&str> = recorded.lines().collect();
    assert!(
        recorded.contains(&"introduce yourself"),
        "the CLI did not receive the child's task: {recorded:?}"
    );
    let model = recorded
        .iter()
        .position(|arg| *arg == "--model")
        .expect("the CLI was told which model to run");
    assert_eq!(recorded[model + 1], "sonnet");
    // The spawn asked for read-only tools, so the CLI's own tools stay
    // withheld even though the owner enabled them for this agent.
    assert!(
        !recorded.contains(&"--permission-mode"),
        "a read-only child received the CLI's own tools: {recorded:?}"
    );
    assert!(
        parent_provider.requests().is_empty(),
        "the child fell back to the root provider/model route"
    );
    session.shutdown().await.expect("clean shutdown");
}

/// Writes a stand-in for the Claude CLI that records its arguments and emits
/// one turn in the stream-json dialect the adapter reads.
fn fake_claude_cli(dir: &std::path::Path, argv: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let path = dir.join("fake-claude");
    let script = format!(
        r#"#!/bin/sh
for arg in "$@"; do printf '%s\n' "$arg"; done > '{}'
echo '{{"type":"system","subtype":"init","session_id":"fake-session"}}'
echo '{{"type":"assistant","message":{{"content":[{{"type":"text","text":"ran on the installed agent"}}]}}}}'
echo '{{"type":"result","subtype":"success","is_error":false,"usage":{{"input_tokens":1,"output_tokens":1}}}}'
"#,
        argv.display()
    );
    std::fs::write(&path, script).expect("a CLI stand-in");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
        .expect("an executable stand-in");
    path
}
