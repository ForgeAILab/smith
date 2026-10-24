use super::*;

const FALLBACK_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[profiles.dev.fallback]
first_response_timeout_ms = 100
models = [{ provider = "backup", model = "secondary" }]

[providers.local]
kind = "fake"

[providers.backup]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[models."backup/secondary"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[approval]
mode = "allow-all"
"#;

#[derive(Debug)]
struct NeverRespond;

#[async_trait]
impl Provider for NeverRespond {
    fn describe(&self) -> Vec<ModelDescriptor> {
        Vec::new()
    }

    fn capabilities(&self, _model: &agent_runtime_core::provider::ModelId) -> Option<Capabilities> {
        Some(Capabilities::basic_streaming())
    }

    fn pre_output_replay_safe(&self, _request: &ProviderRequest) -> bool {
        true
    }

    async fn stream(
        &self,
        _request: ProviderRequest,
        _ctx: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        Ok(Box::pin(futures_util::stream::pending()))
    }
}

/// An inheriting child shares the parent's provider, which defers quota
/// rotation to the profile's chain, so the child must run that chain too.
#[tokio::test]
async fn an_inherited_child_falls_back_along_the_profile_chain() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.project.path().join(".smith/config.toml"),
        FALLBACK_CONFIG,
    )
    .expect("a fallback config");
    let backup = scripted(2, "the backup child's findings");
    let mut runtime_request = request(&fixture, Arc::new(NeverRespond));
    runtime_request
        .fallback_providers
        .insert(("backup".into(), "secondary".into()), backup.clone());
    let smith = Box::pin(factory::build_request(runtime_request))
        .await
        .expect("a runtime with a fallback chain");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    let _lifecycle = wire_delegation(&session, delegation)
        .await
        .expect("delegation wires once");
    let coordinator = delegation.coordinator().expect("a coordinator");

    let child = match coordinator
        .spawn(ChildSpec {
            task: UserInput::text("summarize the repository layout"),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(2),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("a spawn")
    {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        coordinator.wait_task_outcome(&child),
    )
    .await
    .expect("the child reaches its fallback before the test deadline")
    .expect("a protected child outcome");
    assert!(
        matches!(
            &outcome,
            ChildTaskOutcome::Completed { result, .. }
                if result.text == "the backup child's findings"
        ),
        "{outcome:?}"
    );
    assert_eq!(backup.requests()[0].model.as_str(), "secondary");

    session.shutdown().await.expect("a clean shutdown");
}
