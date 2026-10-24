use super::*;
use agent_runtime_core::event::{ModelFallbackReason, TurnFinish};
use agent_runtime_core::provider::{
    ProviderCallContext, ProviderError, ProviderRequest, ProviderStream,
};

const FALLBACK_CONFIG: &str = r#"
default_profile = "work"
[approval]
mode = "allow-all"
[providers.local]
kind = "fake"
[providers.backup]
kind = "fake"
[profiles.work]
provider = "local"
model = "primary"
instructions = "Preserve these profile instructions."
[profiles.work.fallback]
first_response_timeout_ms = 100
models = [{ provider = "backup", model = "secondary", max_output_tokens = 256 }]
[models."local/primary"]
context_tokens = 32768
max_input_tokens = 30000
max_output_tokens = 1024
[models."backup/secondary"]
context_tokens = 16384
max_input_tokens = 15000
max_output_tokens = 512
"#;

#[derive(Debug)]
struct NeverRespond;
#[async_trait]
impl Provider for NeverRespond {
    fn describe(&self) -> Vec<agent_runtime_core::provider::ModelDescriptor> {
        Vec::new()
    }
    fn capabilities(&self, _: &ModelId) -> Option<Capabilities> {
        Some(Capabilities::basic_streaming())
    }
    fn pre_output_replay_safe(&self, _: &ProviderRequest) -> bool {
        true
    }
    async fn stream(
        &self,
        _: ProviderRequest,
        _: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        Ok(Box::pin(futures_util::stream::pending()))
    }
}

#[tokio::test]
async fn profile_startup_timeout_switches_providers_with_model_controls_and_events() {
    let fixture = Fixture::new(FALLBACK_CONFIG);
    let backup = Arc::new(FakeProvider::text_reply("backup response"));
    let observer = RecordingObserver::shared();
    let mut req = request(&fixture, HostSurface::Headless);
    req.provider = Some(Arc::new(NeverRespond));
    req.fallback_providers
        .insert(("backup".into(), "secondary".into()), backup.clone());
    req.observers.push(observer.clone());
    let runtime = factory::build_request(req).await.unwrap();
    let session = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    session.run(UserInput::text("hello")).await.unwrap();
    let requests = backup.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model.as_str(), "secondary");
    assert_eq!(requests[0].max_output_tokens, Some(256));
    assert!(
        serde_json::to_string(&requests[0].messages)
            .unwrap()
            .contains("Preserve these profile instructions")
    );
    let events = observer.payloads();
    assert!(events.iter().any(|event| matches!(event, RuntimeEvent::ModelFallback { to_provider, reason: ModelFallbackReason::FirstResponseTimeout, .. } if to_provider == "backup")));
    assert!(events.iter().any(|event| matches!(
        event,
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            ..
        }
    )));
    assert_eq!(runtime.policy().agent_profile, "work");
    assert_eq!(
        runtime.policy().fallback_models[0].model.as_str(),
        "secondary"
    );
}

#[tokio::test]
async fn future_credential_is_not_read_until_fallback_is_selected() {
    let config = FALLBACK_CONFIG.replace("[providers.backup]\nkind = \"fake\"", "[providers.backup]\nkind = \"openai-compatible\"\nbase_url = \"https://never.example.test/v1\"\ncredential = \"env:UNUSED_FALLBACK_KEY\"");
    let fixture = Fixture::new(&config);
    let mut req = request(&fixture, HostSurface::Headless);
    req.credentials = Some(
        CredentialResolver::new("/unused").with_environment(Arc::new(PanicsIfCredentialResolved)),
    );
    req.provider = Some(Arc::new(FakeProvider::text_reply("primary response")));
    let runtime = factory::build_request(req).await.unwrap();
    let session = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    session.run(UserInput::text("hello")).await.unwrap();
    assert!(
        session
            .history()
            .iter()
            .any(|message| message.joined_text().contains("primary response"))
    );
}

#[tokio::test]
async fn invalid_fallback_metadata_fails_before_primary_credentials() {
    let config = FALLBACK_CONFIG.replace("[providers.local]\nkind = \"fake\"", "[providers.local]\nkind = \"openai-compatible\"\nbase_url = \"https://never.example.test/v1\"\ncredential = \"env:PRIMARY_KEY\"")
        .replace("[models.\"backup/secondary\"]\ncontext_tokens = 16384\nmax_input_tokens = 15000\nmax_output_tokens = 512", "");
    let fixture = Fixture::new(&config);
    let mut req = request(&fixture, HostSurface::Headless);
    req.credentials = Some(
        CredentialResolver::new("/unused").with_environment(Arc::new(PanicsIfCredentialResolved)),
    );
    let error = factory::build_request(req).await.unwrap_err();
    assert!(error.to_string().contains("secondary"), "{error}");
}

#[tokio::test]
async fn buffered_reasoning_counts_as_a_started_response_for_latency_fallback() {
    let config = FALLBACK_CONFIG.replace("[providers.local]\nkind = \"fake\"", "[providers.local]\nkind = \"openai-compatible\"\nbase_url = \"https://never.example.test/v1\"\n[providers.local.response]\nreasoning_only = \"text\"")
        + "\n[limits]\nturn_time_limit_ms = 300\n";
    let fixture = Fixture::new(&config);
    let primary = Arc::new(FakeProvider::new(
        "primary",
        Capabilities::basic_streaming(),
        vec![ScriptedStream::blocking(vec![
            ProviderStreamEvent::ReasoningDelta {
                text: "working".into(),
                redacted: false,
                signature: None,
            },
        ])],
    ));
    let backup = Arc::new(FakeProvider::text_reply("must not run"));
    let observer = RecordingObserver::shared();
    let mut req = request(&fixture, HostSurface::Headless);
    req.provider = Some(primary);
    req.fallback_providers
        .insert(("backup".into(), "secondary".into()), backup.clone());
    req.observers.push(observer.clone());
    let runtime = factory::build_request(req).await.unwrap();
    let session = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    session.run(UserInput::text("hello")).await.unwrap();
    assert!(backup.requests().is_empty());
    assert!(
        !observer
            .payloads()
            .iter()
            .any(|event| matches!(event, RuntimeEvent::ModelFallback { .. }))
    );
}

#[tokio::test]
async fn unavailable_fallback_credential_source_advances_without_network_io() {
    let config = FALLBACK_CONFIG.replace("models = [{ provider = \"backup\", model = \"secondary\", max_output_tokens = 256 }]",
        "models = [{ provider = \"unavailable\", model = \"unused\" }, { provider = \"backup\", model = \"secondary\", max_output_tokens = 256 }]")
        + "\n[providers.unavailable]\nkind = \"openai-compatible\"\nbase_url = \"https://never.example.test/v1\"\ncredential = \"env:UNAVAILABLE_FALLBACK\"\n[models.\"unavailable/unused\"]\ncontext_tokens = 16384\nmax_input_tokens = 15000\nmax_output_tokens = 512\n";
    let fixture = Fixture::new(&config);
    let observer = RecordingObserver::shared();
    let backup = Arc::new(FakeProvider::text_reply("available backup"));
    let mut req = request(&fixture, HostSurface::Headless);
    req.provider = Some(Arc::new(NeverRespond));
    req.fallback_providers
        .insert(("backup".into(), "secondary".into()), backup.clone());
    req.observers.push(observer.clone());
    let runtime = factory::build_request(req).await.unwrap();
    let session = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    session.run(UserInput::text("hello")).await.unwrap();
    assert_eq!(backup.requests().len(), 1);
    assert!(observer.payloads().iter().any(|event| matches!(
        event,
        RuntimeEvent::ModelFallback {
            reason: ModelFallbackReason::Unavailable,
            ..
        }
    )));
}
