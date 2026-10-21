use super::*;

use agent_runtime_core::provider::ProviderErrorKind;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn private_endpoint_validates_reasoning_after_the_request_is_sent() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        for accepted in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let request = loop {
                    let mut chunk = [0; 4096];
                    let read = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(read, 0, "the client closed before sending its request");
                    bytes.extend_from_slice(&chunk[..read]);
                    let Some(boundary) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
                        continue;
                    };
                    let headers = std::str::from_utf8(&bytes[..boundary]).unwrap();
                    let length: usize = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    }).expect("a content length");
                    let start = boundary + 4;
                    if bytes.len() >= start + length {
                        break serde_json::from_slice::<serde_json::Value>(&bytes[start..start + length]).unwrap();
                    }
                };
                let (status, content_type, body) = if accepted {
                    ("200 OK", "text/event-stream", "data: {\"choices\":[{\"delta\":{\"content\":\"accepted\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
                } else {
                    ("400 Bad Request", "application/json", r#"{"error":{"message":"reasoning_effort ultra is not supported"}}"#)
                };
                socket.write_all(format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                ).as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
                request
            });
            let config = format!("{FAKE_CONFIG}\n[profiles.dev.reasoning]\neffort = \"ultra\"\n")
                .replace("kind = \"fake\"", &format!("kind = \"openai-compatible\"\nbase_url = \"http://{address}/v1\""));
            let fixture = Fixture::new(&config);
            let recorder = RecordingObserver::shared();
            let request = RuntimeRequest {
                observers: vec![recorder.clone()],
                ..request(&fixture, HostSurface::Headless)
            };
            factory::preflight(&request).await.expect("metadata does not block setup");
            let smith = factory::build_request(request).await.expect("a compatible runtime");
            let session = smith.runtime().start_session(StartSession::new()).await.unwrap();
            session.run(UserInput::text("hello")).await.unwrap();
            let wire = server.await.unwrap();
            assert_eq!(wire["reasoning_effort"], "ultra");
            let failures: Vec<_> = recorder.events().into_iter().filter_map(|event| match event.payload {
                RuntimeEvent::ProviderAttemptFinished { error, .. } => error,
                _ => None,
            }).collect();
            if accepted {
                assert!(failures.is_empty(), "{failures:?}");
                assert!(session.history().iter().any(|message| message.joined_text() == "accepted"));
            } else {
                assert_eq!(failures.len(), 1);
                assert_eq!(failures[0].kind, ProviderErrorKind::BadRequest);
            }
            session.shutdown().await.unwrap();
        }
    }).await.expect("the endpoint fixtures finish promptly");
}

/// A profile selecting an adapter this build of Agent Runtime does not ship.
const UNAVAILABLE_ADAPTER_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "remote"
model = "example-model"

[providers.remote]
kind = "grpc-frontier"
base_url = "https://api.example.test/v1"

[models."remote/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[approval]
mode = "allow-all"
"#;

#[derive(Debug)]
struct PanicsIfRegistered;

#[async_trait]
impl Tool for PanicsIfRegistered {
    fn spec(&self) -> ToolSpec {
        panic!("setup preflight attempted to construct a tool registry")
    }

    async fn invoke(
        &self,
        _prepared: PreparedToolCall,
        _ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        unreachable!("the preflight does not run tools")
    }
}

#[tokio::test]
async fn setup_preflight_uses_factory_derivation_without_constructing_runtime_state() {
    let config = FAKE_CONFIG.replace("mode = \"allow-all\"", "mode = \"ask\"");
    let fixture = Fixture::new(&config);
    let mut request = request(&fixture, HostSurface::Terminal);
    request
        .trusted_native
        .add_tool(Arc::new(PanicsIfRegistered));

    let checked = factory::preflight(&request)
        .await
        .expect("configuration-only factory preflight");
    assert_eq!(checked.provider_name, "local");
    assert_eq!(checked.model, ModelId::new("example-model"));
    assert_eq!(
        checked.model_profile.limits,
        ModelLimits::new(128_000, 124_000, 4_096)
    );

    // Harness resolution intentionally inventories trusted-native descriptors
    // before the factory. Remove the sentinel after proving setup preflight
    // itself did not touch it, then exercise the factory's host-policy order.
    request.trusted_native = Default::default();
    let error = factory::build_request(request)
        .await
        .expect_err("normal construction still requires an approval surface");
    assert!(matches!(
        error,
        FactoryError::MissingHostPolicy {
            what: "approval surface",
            ..
        }
    ));
}

#[tokio::test]
async fn missing_model_limits_fail_with_a_model_profile_diagnostic_and_no_provider_request() {
    let fixture = Fixture::new(NO_LIMITS_CONFIG);
    let provider = Arc::new(FakeProvider::text_reply("never asked for"));
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        ..request(&fixture, HostSurface::Headless)
    };

    let err = factory::build_request(request)
        .await
        .expect_err("a model-profile failure");
    assert!(matches!(err, FactoryError::ModelProfile { .. }), "{err}");
    let rendered = err.to_string();
    assert!(rendered.contains("example-model"), "{rendered}");
    // The diagnostic names what to write, rather than substituting a window.
    assert!(rendered.contains("context_tokens"), "{rendered}");
    assert!(
        provider.requests().is_empty(),
        "startup must fail before any provider request"
    );
}

#[tokio::test]
async fn an_adapter_the_pinned_runtime_does_not_ship_is_reported_as_unavailable() {
    let fixture = Fixture::new(UNAVAILABLE_ADAPTER_CONFIG);

    let err = factory::build_request(request(&fixture, HostSurface::Terminal))
        .await
        .expect_err("an unavailable adapter");
    assert!(
        matches!(err, FactoryError::AdapterUnavailable { .. }),
        "{err}"
    );
    let rendered = err.to_string();
    assert!(rendered.contains("grpc-frontier"), "{rendered}");
    // It says which adapters exist rather than quietly picking one.
    assert!(rendered.contains("openai-compatible"), "{rendered}");
    assert!(rendered.contains("anthropic-messages"), "{rendered}");
    assert!(rendered.contains("openai-responses"), "{rendered}");
}

/// A stateless Responses endpoint — xAI's Grok is the first verified one.
const XAI_CONFIG: &str = r#"
default_profile = "grok"

[profiles.grok]
provider = "xai"
model = "grok-4.5"

[providers.xai]
kind = "openai-responses"
base_url = "https://api.x.ai/v1"

[models."xai/grok-4.5"]
context_tokens = 500000
max_input_tokens = 480000
max_output_tokens = 32768

[approval]
mode = "allow-all"
"#;

#[tokio::test]
async fn a_responses_provider_composes_from_configuration_alone() {
    // The adapter is generic over the Responses protocol and takes its
    // endpoint from `base_url`, so this proves the kind resolves and the
    // credential reaches it without naming a vendor anywhere in the factory.
    let fixture = Fixture::new(XAI_CONFIG);
    let smith = factory::build_request(request(&fixture, HostSurface::Terminal))
        .await
        .expect("a Responses runtime");

    let policy = smith.policy();
    assert_eq!(policy.provider_name, "xai");
    assert_eq!(policy.provider_kind, "openai-responses");
    assert_eq!(policy.model, ModelId::new("grok-4.5"));
    assert_eq!(
        policy.model_profile.limits,
        ModelLimits::new(500_000, 480_000, 32_768)
    );
    // Generic over the protocol: nothing in the factory names a vendor, only
    // the configured endpoint does.
    assert_eq!(policy.endpoint.as_deref(), Some("https://api.x.ai/v1"));
}

#[tokio::test]
async fn host_policy_a_run_cannot_do_without_fails_before_anything_else() {
    // No workspace: the shared runtime would otherwise deny every tool call
    // with nothing to point the user at.
    let fixture = Fixture::new(FAKE_CONFIG);
    let err = factory::build_request(RuntimeRequest::new(fixture.config(), HostSurface::Terminal))
        .await
        .expect_err("no workspace");
    assert!(
        matches!(
            err,
            FactoryError::MissingHostPolicy {
                what: "workspace",
                ..
            }
        ),
        "{err}"
    );

    // `approval.mode = "ask"` with nothing to ask: a headless run must fail
    // closed rather than hang on a question nobody receives.
    const ASK_CONFIG: &str = r#"
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
    let fixture = Fixture::new(ASK_CONFIG);
    let err = factory::build_request(request(&fixture, HostSurface::Headless))
        .await
        .expect_err("no approval surface");
    assert!(
        matches!(
            err,
            FactoryError::MissingHostPolicy {
                what: "approval surface",
                ..
            }
        ),
        "{err}"
    );
    assert!(err.to_string().contains("allow-all"), "{err}");
}
