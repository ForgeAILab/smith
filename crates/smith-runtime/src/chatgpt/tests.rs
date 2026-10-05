use super::*;
use agent_runtime::registry::{Fingerprint, RegistryRevision};
use agent_runtime_core::cache::{CacheEndpointIdentity, CacheIdentity};
use agent_runtime_core::content::ToolCall;
use agent_runtime_core::ids::{AttemptId, RequestId, ToolCallId};
use agent_runtime_core::provider::{ProviderAttemptPurpose, ToolSchema};
use agent_runtime_core::provider_credential::StaticProviderCredentialSource;
use agent_runtime_testkit::{
    CredentialLeaseFixture, RenewableProviderCredentialSource, ReplayTransport,
};
use smith_config::auth_file::{AuthFileBackend, AuthFileError};
use smith_config::credential::{CredentialEnrollmentBackend, KeychainError};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Default)]
struct MemoryEnrollment {
    value: Mutex<Option<Secret>>,
}

impl AuthFileBackend for MemoryEnrollment {
    fn read(&self, _entry: &str) -> Result<Option<Secret>, AuthFileError> {
        Ok(self.value.lock().expect("memory store").clone())
    }

    fn store(&self, _entry: &str, secret: &Secret) -> Result<(), AuthFileError> {
        *self.value.lock().expect("memory store") = Some(secret.clone());
        Ok(())
    }

    fn remove(&self, _entry: &str) -> Result<(), AuthFileError> {
        *self.value.lock().expect("memory store") = None;
        Ok(())
    }
}

#[derive(Debug)]
struct PanicKeychainEnrollment;

impl CredentialEnrollmentBackend for PanicKeychainEnrollment {
    fn prior(&self, _service: &str, _account: &str) -> Result<Option<Secret>, KeychainError> {
        panic!("ChatGPT must not query the keychain")
    }

    fn store(&self, _service: &str, _account: &str, _secret: &Secret) -> Result<(), KeychainError> {
        panic!("ChatGPT must not write the keychain")
    }

    fn remove(&self, _service: &str, _account: &str) -> Result<(), KeychainError> {
        panic!("ChatGPT must not remove a keychain entry")
    }
}

#[derive(Debug, Default)]
struct RotatingEndpoint {
    calls: AtomicUsize,
}

#[derive(Debug)]
struct AuthRejectingTransport;

#[async_trait]
impl HttpTransport for AuthRejectingTransport {
    async fn post_stream(
        &self,
        _request: HttpRequest,
    ) -> Result<agent_runtime::provider::transport::ByteStream, ProviderError> {
        Err(ProviderError::new(
            ProviderErrorKind::Auth,
            "transport-classified unauthorized",
        ))
    }
}

#[async_trait]
impl BundleRefresher<ChatGptTokenBundle> for RotatingEndpoint {
    async fn refresh(
        &self,
        bundle: &ChatGptTokenBundle,
        _now_ms: u64,
    ) -> Result<ChatGptTokenBundle, ChatGptAuthError> {
        assert_eq!(bundle.refresh_secret().expose(), "refresh-old");
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        Ok(ChatGptTokenBundle {
            access_token: "access-new".into(),
            refresh_token: "refresh-new".into(),
            expires_at_ms: SystemClock
                .now()
                .as_millis()
                .saturating_add(60 * 60 * 1_000),
            account_id: bundle.account_id().to_owned(),
        })
    }
}

fn jwt(claims: Value) -> String {
    format!(
        "e30.{}.signature",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("claims"))
    )
}

#[test]
fn account_claims_are_extracted_without_rendering_tokens() {
    let token = jwt(json!({
        "exp": 4_102_444_800_u64,
        "https://api.openai.com/auth": {"chatgpt_account_id": "acct_test-1"}
    }));
    assert_eq!(extract_account_id(&token).as_deref(), Some("acct_test-1"));
    let response = TokenResponse {
        id_token: None,
        access_token: token.clone(),
        refresh_token: Some("refresh-canary".into()),
        expires_in: None,
    };
    let bundle = bundle_from_response(&response, None, None).expect("bundle");
    let debug = format!("{bundle:?}");
    assert!(!debug.contains(&token));
    assert!(!debug.contains("refresh-canary"));
    assert_eq!(bundle.account_id(), "acct_test-1");
}

#[test]
fn token_bundle_round_trips_only_through_a_secret() {
    let bundle = ChatGptTokenBundle {
        access_token: "access-canary".into(),
        refresh_token: "refresh-canary".into(),
        expires_at_ms: 9_999_999_999_999,
        account_id: "acct_test".into(),
    };
    let encoded = bundle.to_secret().expect("encoded");
    let decoded = ChatGptTokenBundle::from_secret(&encoded).expect("decoded");
    assert_eq!(decoded.account_id(), "acct_test");
    assert!(!format!("{decoded:?}").contains("canary"));
}

#[test]
fn codex_rate_limit_headers_parse_both_reset_shapes() {
    let headers = vec![
        ("x-codex-primary-used-percent".to_owned(), "12.5".to_owned()),
        (
            "x-codex-primary-window-minutes".to_owned(),
            "300".to_owned(),
        ),
        (
            "x-codex-primary-reset-at".to_owned(),
            "1704069000".to_owned(),
        ),
        ("x-codex-secondary-used-percent".to_owned(), "80".to_owned()),
        (
            "x-codex-secondary-reset-after-seconds".to_owned(),
            "3600".to_owned(),
        ),
    ];
    let snapshot = codex_rate_limit_snapshot(&headers);
    assert_eq!(snapshot.windows.len(), 2);
    assert_eq!(snapshot.windows[0].id.as_deref(), Some("primary"));
    assert_eq!(snapshot.windows[0].used_percent, Some(12.5));
    assert_eq!(snapshot.windows[0].window_seconds, Some(300 * 60));
    assert_eq!(snapshot.windows[0].resets_at_ms, Some(1_704_069_000_000));
    assert_eq!(snapshot.windows[1].id.as_deref(), Some("secondary"));
    assert_eq!(snapshot.windows[1].used_percent, Some(80.0));
    assert_eq!(snapshot.windows[1].resets_in_ms, Some(3_600_000));
}

#[test]
fn absent_rate_limit_headers_yield_an_empty_snapshot_not_zeroes() {
    let headers = vec![("content-type".to_owned(), "text/event-stream".to_owned())];
    assert!(codex_rate_limit_snapshot(&headers).is_empty());
}

#[test]
fn usage_payload_maps_windows_and_treats_zero_delay_as_filler() {
    let body = json!({
        "plan_type": "pro",
        "rate_limit": {
            "primary_window": {
                "used_percent": 42,
                "limit_window_seconds": 300,
                "reset_after_seconds": 0,
                "reset_at": 1704069000,
            },
            "secondary_window": {
                "used_percent": 84.5,
                "limit_window_seconds": 3600,
                "reset_after_seconds": 1800,
            },
        },
        "credits": {"has_credits": true, "unlimited": false},
    });
    let snapshot =
        usage_snapshot_from_json(&serde_json::to_vec(&body).expect("encodes")).expect("parses");
    assert_eq!(snapshot.windows.len(), 2);
    assert_eq!(snapshot.windows[0].id.as_deref(), Some("primary"));
    assert_eq!(snapshot.windows[0].used_percent, Some(42.0));
    assert_eq!(snapshot.windows[0].window_seconds, Some(300));
    assert_eq!(snapshot.windows[0].resets_at_ms, Some(1_704_069_000_000));
    // The zero delay beside an absolute reset is filler, not "resets now".
    assert_eq!(snapshot.windows[0].resets_in_ms, None);
    assert_eq!(snapshot.windows[1].used_percent, Some(84.5));
    assert_eq!(snapshot.windows[1].resets_in_ms, Some(1_800_000));
}

#[test]
fn usage_payload_without_rate_limit_reads_as_unmeasured() {
    let body = json!({"plan_type": "plus"});
    let snapshot =
        usage_snapshot_from_json(&serde_json::to_vec(&body).expect("encodes")).expect("parses");
    assert!(snapshot.is_empty());
}

#[test]
fn responses_usage_is_disjoint_and_terminal() {
    let mut state = StreamState::default();
    let events = decode_event(
        &json!({
            "type": "response.completed",
            "response": {"usage": {
                "input_tokens": 100,
                "input_tokens_details": {"cached_tokens": 25},
                "output_tokens": 40,
                "output_tokens_details": {"reasoning_tokens": 10}
            }}
        })
        .to_string(),
        &mut state,
    )
    .expect("event");
    let usage = events
        .iter()
        .find_map(|event| match event {
            ProviderStreamEvent::Usage { delta } => Some(delta),
            _ => None,
        })
        .expect("usage");
    assert_eq!(usage.get(CounterKind::InputUncached), 75);
    assert_eq!(usage.get(CounterKind::InputCached), 25);
    assert_eq!(usage.get(CounterKind::Output), 30);
    assert_eq!(usage.get(CounterKind::Reasoning), 10);
    assert!(state.terminal);
}

#[test]
fn responses_cache_observation_preserves_presence_and_write_tokens() {
    let cases = [
        (
            "zero-read",
            json!({
                "input_tokens": 10,
                "input_tokens_details": {"cached_tokens": 0},
                "output_tokens": 2
            }),
            Some(0),
            None,
            10,
            0,
        ),
        (
            "omitted",
            json!({"input_tokens": 10, "output_tokens": 2}),
            None,
            None,
            10,
            0,
        ),
        (
            "write-only",
            json!({
                "input_tokens": 10,
                "input_tokens_details": {"cache_write_tokens": 3},
                "output_tokens": 2
            }),
            None,
            Some(3),
            7,
            3,
        ),
    ];

    for (name, usage, expected_read, expected_write, uncached, written) in cases {
        let mut state = StreamState::default();
        let events = decode_event(
            &json!({"type": "response.completed", "response": {"usage": usage}}).to_string(),
            &mut state,
        )
        .expect("event");
        let observations: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                ProviderStreamEvent::CacheObservation {
                    read_tokens,
                    write_tokens,
                } => Some((*read_tokens, *write_tokens)),
                _ => None,
            })
            .collect();
        if expected_read.is_none() && expected_write.is_none() {
            assert!(observations.is_empty(), "{name} must stay absent");
        } else {
            assert_eq!(
                observations,
                vec![(expected_read, expected_write)],
                "{name}"
            );
        }
        let delta = events.iter().find_map(|event| match event {
            ProviderStreamEvent::Usage { delta } => Some(delta),
            _ => None,
        });
        let delta = delta.expect("usage");
        assert_eq!(delta.get(CounterKind::InputUncached), uncached, "{name}");
        assert_eq!(delta.get(CounterKind::CacheWrite), written, "{name}");
    }
}

#[test]
fn browser_url_pins_pkce_state_scopes_and_smith_originator() {
    let url = browser_authorization_url(BrowserAuthorization {
        redirect_uri: "http://localhost:1455/auth/callback",
        code_challenge: "challenge",
        state: "state",
    });
    let parsed = reqwest::Url::parse(&url).expect("url");
    let query = parsed
        .query_pairs()
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        query.get("client_id").map(|value| value.as_ref()),
        Some(CHATGPT_CLIENT_ID)
    );
    assert_eq!(
        query
            .get("code_challenge_method")
            .map(|value| value.as_ref()),
        Some("S256")
    );
    assert_eq!(
        query.get("state").map(|value| value.as_ref()),
        Some("state")
    );
    assert_eq!(
        query.get("originator").map(|value| value.as_ref()),
        Some("smith")
    );
    assert_eq!(
        query.get("scope").map(|value| value.as_ref()),
        Some(CHATGPT_SCOPES)
    );
}

#[test]
fn response_tool_names_are_backend_safe_collision_free_and_reversible() {
    let tools = vec![
        ToolSchema {
            name: "smith_tool_1".into(),
            description: "Already valid".into(),
            input_schema: json!({"type": "object"}),
        },
        ToolSchema {
            name: "tool:registry.search".into(),
            description: "Needs a wire alias".into(),
            input_schema: json!({"type": "object"}),
        },
    ];
    let names = response_tool_names(&tools).expect("distinct tool names");
    let alias = response_wire_tool_name("tool:registry.search");
    assert_eq!(
        names.get("smith_tool_1").map(String::as_str),
        Some("smith_tool_1")
    );
    assert_eq!(
        names.get(&alias).map(String::as_str),
        Some("tool:registry.search")
    );
    assert!(names.keys().all(|name| valid_response_tool_name(name)));
    assert_eq!(
        response_tool_choice(&ToolChoice::Named("tool:registry.search".into()), &names,)
            .expect("named choice")
            .pointer("/name")
            .and_then(Value::as_str),
        Some(alias.as_str())
    );
}

#[test]
fn tool_call_history_keeps_its_wire_name_when_activation_grows() {
    let registry = ToolSchema {
        name: "registry.search".into(),
        description: "Search the tool registry".into(),
        input_schema: json!({"type": "object"}),
    };
    let initial = response_tool_names(std::slice::from_ref(&registry)).expect("initial tool names");
    let expanded = response_tool_names(&[
        ToolSchema {
            name: "artifact.read".into(),
            description: "Read an artifact".into(),
            input_schema: json!({"type": "object"}),
        },
        ToolSchema {
            name: "list".into(),
            description: "List files".into(),
            input_schema: json!({"type": "object"}),
        },
        registry,
    ])
    .expect("expanded tool names");
    let wire = response_wire_tool_name("registry.search");
    assert_eq!(initial.get(&wire), Some(&"registry.search".to_owned()));
    assert_eq!(expanded.get(&wire), Some(&"registry.search".to_owned()));

    let history = Message::assistant(vec![ContentPart::ToolCall(ToolCall {
        id: ToolCallId::new("call-1"),
        name: "registry.search".into(),
        arguments: json!({"query": "workspace repository inspection"}),
    })]);
    let replayed = response_items(&history, &expanded).expect("history encodes");
    assert_eq!(
        replayed[0].get("name").and_then(Value::as_str),
        Some(wire.as_str())
    );
    assert!(valid_response_tool_name(&wire));
}

#[tokio::test]
async fn context_only_cache_identity_reaches_the_chatgpt_routing_key() {
    let identity = CacheIdentity::builder(
        "chatgpt",
        ModelId::new("gpt-5.6-terra"),
        CacheEndpointIdentity::from_opaque("chatgpt-codex", RegistryRevision::new("endpoint-r1")),
        RegistryRevision::new("adapter-r1"),
        Fingerprint::of("profile-r1"),
    )
    .cache_control(PromptCacheControl::Implicit)
    .provider_key(Fingerprint::of("routing-key-r1"))
    .build();
    let target = ProviderCredentialTarget::new("chatgpt").expect("target");
    let provider = ChatGptProvider::new(
        ReplayTransport::single(
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
        ),
        ChatGptProviderConfig::new(
            "gpt-5.6-terra",
            Capabilities::basic_streaming(),
            "acct_test",
        )
        .expect("config"),
        target,
        Arc::new(StaticProviderCredentialSource::new(Secret::new(
            "access-token-canary",
        ))) as Arc<dyn ProviderCredentialSource>,
    );
    let ctx = ProviderCallContext {
        session: agent_runtime_core::ids::SessionId::new("session-must-not-be-used"),
        request_id: RequestId::new("request-cache-identity"),
        attempt_id: AttemptId::new("attempt-cache-identity"),
        cache_identity: Some(identity.clone()),
        purpose: ProviderAttemptPurpose::Ordinary,
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
    };
    provider
        .stream(
            ProviderRequest::new(
                ModelId::new("gpt-5.6-terra"),
                vec![Message::user("identity propagation")],
            ),
            ctx,
        )
        .await
        .expect("stream")
        .collect::<Vec<_>>()
        .await;

    let requests = provider.transport().requests();
    let payload: Value = serde_json::from_slice(&requests[0].body).expect("payload");
    assert_eq!(
        payload["prompt_cache_key"],
        identity.wire_cache_key().as_str()
    );
}

/// The wire pairs a bearer with the account that issued it. When the
/// source can name that account, its lease overrides the identity frozen
/// into the provider config at construction — otherwise a source that
/// changed accounts would send one account's token under another's header.
#[tokio::test]
async fn the_account_header_follows_the_lease_not_the_frozen_config() {
    let target = ProviderCredentialTarget::new("chatgpt").expect("target");
    let source = Arc::new(ChatGptCredentialSource::new(
        target.clone(),
        CredentialRef::parse("authfile:chatgpt").expect("reference"),
        ChatGptTokenBundle {
            access_token: "access-lease".into(),
            refresh_token: "refresh".into(),
            expires_at_ms: u64::MAX,
            account_id: "acct_lease".into(),
        },
        CredentialEnroller::with_backends(
            Arc::new(PanicKeychainEnrollment),
            Arc::new(MemoryEnrollment::default()),
        ),
        Arc::new(RotatingEndpoint::default()),
        None,
        Arc::new(SystemClock),
    )) as Arc<dyn ProviderCredentialSource>;
    let provider = ChatGptProvider::new(
        ReplayTransport::single(
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n",
        ),
        ChatGptProviderConfig::new(
            "gpt-5.6-terra",
            Capabilities::basic_streaming(),
            "acct_config",
        )
        .expect("config"),
        target,
        source,
    );
    let ctx = ProviderCallContext {
        session: agent_runtime_core::ids::SessionId::new("session-test"),
        request_id: RequestId::new("request-1"),
        attempt_id: AttemptId::new("attempt-1"),
        cache_identity: None,
        purpose: ProviderAttemptPurpose::Ordinary,
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
    };
    let _events = provider
        .stream(
            ProviderRequest::new(ModelId::new("gpt-5.6-terra"), vec![Message::user("hello")]),
            ctx,
        )
        .await
        .expect("stream")
        .collect::<Vec<_>>()
        .await;
    let sent = provider.transport().requests();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0]
            .headers
            .iter()
            .any(|(name, value)| name == "chatgpt-account-id" && value == "acct_lease")
    );
    assert!(
        !sent[0]
            .headers
            .iter()
            .any(|(_, value)| value == "acct_config")
    );
}

#[tokio::test]
async fn concurrent_expiry_refreshes_once_and_persists_rotated_bundle() {
    let backend = Arc::new(MemoryEnrollment::default());
    let endpoint = Arc::new(RotatingEndpoint::default());
    let source = Arc::new(ChatGptCredentialSource::new(
        ProviderCredentialTarget::new("chatgpt").expect("target"),
        CredentialRef::parse("authfile:chatgpt").expect("reference"),
        ChatGptTokenBundle {
            access_token: "access-old".into(),
            refresh_token: "refresh-old".into(),
            expires_at_ms: 1,
            account_id: "acct_test".into(),
        },
        CredentialEnroller::with_backends(Arc::new(PanicKeychainEnrollment), backend.clone()),
        endpoint.clone(),
        None,
        Arc::new(SystemClock),
    ));
    let target = ProviderCredentialTarget::new("chatgpt").expect("target");
    let cancel = Cancellation::new();
    let (left, right) = tokio::join!(
        source.acquire(&target, 30_000, &cancel, Deadline::never()),
        source.acquire(&target, 30_000, &cancel, Deadline::never()),
    );
    let left = left.expect("left lease");
    let right = right.expect("right lease");
    assert_eq!(left.secret().expose(), "access-new");
    assert_eq!(right.secret().expose(), "access-new");
    assert_eq!(endpoint.calls.load(Ordering::SeqCst), 1);
    let stored = backend
        .value
        .lock()
        .expect("memory store")
        .clone()
        .expect("stored bundle");
    let stored = ChatGptTokenBundle::from_secret(&stored).expect("rotated bundle");
    assert_eq!(stored.access_secret().expose(), "access-new");
    assert_eq!(stored.refresh_secret().expose(), "refresh-new");
}

#[tokio::test]
async fn pre_stream_auth_rejection_invalidates_exact_revision_for_one_runtime_replay() {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let source = Arc::new(RenewableProviderCredentialSource::new(
        clock,
        CredentialLeaseFixture::non_expiring("access-old", "revision-1").expect("lease"),
        [CredentialLeaseFixture::non_expiring("access-new", "revision-2").expect("lease")],
    ));
    let provider = ChatGptProvider::new(
        AuthRejectingTransport,
        ChatGptProviderConfig::new(
            "gpt-5.6-terra",
            Capabilities::basic_streaming(),
            "acct_test",
        )
        .expect("config"),
        ProviderCredentialTarget::new("chatgpt").expect("target"),
        source.clone(),
    );
    let result = provider
        .stream(
            ProviderRequest::new(ModelId::new("gpt-5.6-terra"), vec![Message::user("hello")]),
            ProviderCallContext {
                session: agent_runtime_core::ids::SessionId::new("session-test"),
                request_id: RequestId::new("request-1"),
                attempt_id: AttemptId::new("attempt-1"),
                cache_identity: None,
                purpose: ProviderAttemptPurpose::Ordinary,
                cancel: Cancellation::new(),
                deadline: Deadline::never(),
            },
        )
        .await;
    let error = match result {
        Ok(_) => panic!("401 classification must fail before a stream is accepted"),
        Err(error) => error,
    };
    assert_eq!(error.kind, ProviderErrorKind::Auth);
    assert_eq!(
        error.credential_recovery,
        Some(ProviderCredentialRecovery::RetryWithRenewedCredential)
    );
    assert_eq!(source.invalidations().len(), 1);
}

#[tokio::test]
async fn direct_adapter_maps_tools_usage_headers_and_terminal_without_codex() {
    let wire = response_wire_tool_name("tool:registry.search");
    let sse = [
        "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"call_id\":\"call-1\",\"name\":\"smith_tool_0\"}}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"delta\":\"{\\\"path\\\":\\\"README.md\\\"}\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":12,\"input_tokens_details\":{\"cached_tokens\":2},\"output_tokens\":5,\"output_tokens_details\":{\"reasoning_tokens\":1}}}}\n\n",
    ]
    .concat()
    .replace("smith_tool_0", &wire);
    let target = ProviderCredentialTarget::new("chatgpt").expect("target");
    let source = Arc::new(StaticProviderCredentialSource::new(Secret::new(
        "access-token-canary",
    ))) as Arc<dyn ProviderCredentialSource>;
    let provider = ChatGptProvider::new(
        ReplayTransport::single(sse),
        ChatGptProviderConfig::new(
            "gpt-5.6-terra",
            Capabilities::basic_streaming(),
            "acct_test",
        )
        .expect("config"),
        target,
        source,
    );
    let mut request = ProviderRequest::new(
        ModelId::new("gpt-5.6-terra"),
        vec![Message::system("Be concise"), Message::user("Read it")],
    );
    request.max_output_tokens = Some(16);
    request.tools.push(ToolSchema {
        name: "tool:registry.search".into(),
        description: "Search the tool registry".into(),
        input_schema: json!({"type": "object"}),
    });
    let ctx = ProviderCallContext {
        session: agent_runtime_core::ids::SessionId::new("session-test"),
        request_id: RequestId::new("request-1"),
        attempt_id: AttemptId::new("attempt-1"),
        cache_identity: None,
        purpose: ProviderAttemptPurpose::Ordinary,
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
    };
    let events = provider
        .stream(request, ctx)
        .await
        .expect("stream")
        .collect::<Vec<_>>()
        .await;
    assert!(events.iter().any(|event| matches!(
        event,
        ProviderStreamEvent::ToolCallDelta {
            id: Some(id),
            name: Some(name),
            ..
        } if id == "call-1" && name == "tool:registry.search"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls
        }
    )));
    let usage = events
        .iter()
        .find_map(|event| match event {
            ProviderStreamEvent::Usage { delta } => Some(delta),
            _ => None,
        })
        .expect("usage");
    assert_eq!(usage.total(), 17);

    let sent = provider.transport().requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].url, CHATGPT_RESPONSES_ENDPOINT);
    assert!(
        sent[0].headers.iter().any(|(name, value)| {
            name == "authorization" && value == "Bearer access-token-canary"
        })
    );
    assert!(
        sent[0]
            .headers
            .iter()
            .any(|(name, value)| { name == "chatgpt-account-id" && value == "acct_test" })
    );
    assert!(!format!("{:?}", sent[0]).contains("access-token-canary"));
    let body: Value = serde_json::from_slice(&sent[0].body).expect("body");
    assert_eq!(body.get("store"), Some(&Value::Bool(false)));
    assert_eq!(body.get("max_output_tokens"), None);
    assert_eq!(
        body.get("instructions").and_then(Value::as_str),
        Some("Be concise")
    );
    assert_eq!(
        body.pointer("/tools/0/name").and_then(Value::as_str),
        Some(wire.as_str())
    );
}

#[tokio::test]
async fn truncated_responses_stream_fails_malformed_instead_of_inventing_finish() {
    let target = ProviderCredentialTarget::new("chatgpt").expect("target");
    let source = Arc::new(StaticProviderCredentialSource::new(Secret::new("token")))
        as Arc<dyn ProviderCredentialSource>;
    let provider = ChatGptProvider::new(
        ReplayTransport::single(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
        ),
        ChatGptProviderConfig::new(
            "gpt-5.6-terra",
            Capabilities::basic_streaming(),
            "acct_test",
        )
        .expect("config"),
        target,
        source,
    );
    let ctx = ProviderCallContext {
        session: agent_runtime_core::ids::SessionId::new("session-test"),
        request_id: RequestId::new("request-1"),
        attempt_id: AttemptId::new("attempt-1"),
        cache_identity: None,
        purpose: ProviderAttemptPurpose::Ordinary,
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
    };
    let events = provider
        .stream(
            ProviderRequest::new(ModelId::new("gpt-5.6-terra"), vec![Message::user("hello")]),
            ctx,
        )
        .await
        .expect("stream")
        .collect::<Vec<_>>()
        .await;
    assert!(
        matches!(events.last(), Some(ProviderStreamEvent::Error { error })
        if error.kind == ProviderErrorKind::MalformedStream)
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProviderStreamEvent::Finish { .. }))
    );
}

#[tokio::test]
#[ignore = "requires explicitly injected ChatGPT test credentials and spends a bounded live request"]
async fn live_chatgpt_responses() {
    let access_token = Secret::new(
        std::env::var("SMITH_CHATGPT_TEST_ACCESS_TOKEN")
            .expect("inject SMITH_CHATGPT_TEST_ACCESS_TOKEN explicitly"),
    );
    let account_id = std::env::var("SMITH_CHATGPT_TEST_ACCOUNT_ID")
        .expect("inject SMITH_CHATGPT_TEST_ACCOUNT_ID explicitly");
    let target = ProviderCredentialTarget::new("chatgpt").expect("target");
    let source = Arc::new(StaticProviderCredentialSource::new(access_token))
        as Arc<dyn ProviderCredentialSource>;
    let provider = ChatGptProvider::new(
        crate::transport::ReqwestTransport::new(Default::default()).expect("transport"),
        ChatGptProviderConfig::new("gpt-5.6-terra", Capabilities::basic_streaming(), account_id)
            .expect("config"),
        target,
        source,
    );
    let clock = SystemClock;
    let ctx = ProviderCallContext {
        session: agent_runtime_core::ids::SessionId::new("session-test"),
        request_id: RequestId::new("smith-live-chatgpt"),
        attempt_id: AttemptId::new("smith-live-chatgpt-1"),
        cache_identity: None,
        purpose: ProviderAttemptPurpose::Ordinary,
        cancel: Cancellation::new(),
        deadline: Deadline::after(&clock, 180_000),
    };
    let mut request = ProviderRequest::new(
        ModelId::new("gpt-5.6-terra"),
        vec![Message::user("Reply with exactly: SMITH_OK")],
    );
    request.max_output_tokens = Some(16);
    let events = provider
        .stream(request, ctx)
        .await
        .expect("live stream")
        .collect::<Vec<_>>()
        .await;
    let text = events
        .iter()
        .filter_map(|event| match event {
            ProviderStreamEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(
        text.contains("SMITH_OK"),
        "live response did not contain the canary"
    );
    assert!(events.iter().any(|event| matches!(
        event,
        ProviderStreamEvent::Finish {
            reason: FinishReason::Stop
        }
    )));
}
