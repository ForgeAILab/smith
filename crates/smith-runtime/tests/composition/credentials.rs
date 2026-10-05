use super::*;

/// A production-shaped profile whose endpoint is a closed loopback port.
const OPENAI_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "remote"
model = "example-model"

[providers.remote]
kind = "openai-compatible"
base_url = "http://127.0.0.1:1/v1"
credential = "env:ACME_API_KEY"

[models."remote/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[limits]
max_retries = 0

[approval]
mode = "allow-all"
"#;

#[derive(Debug)]
struct WaitingKeychain {
    gate: Arc<(Mutex<bool>, Condvar)>,
}

impl Keychain for WaitingKeychain {
    fn secret(&self, _service: &str, _account: &str) -> Result<Secret, KeychainError> {
        let (lock, ready) = &*self.gate;
        let mut released = lock.lock().expect("credential test gate");
        while !*released {
            released = ready.wait(released).expect("credential test gate wait");
        }
        Ok(Secret::new(TOKEN))
    }
}

#[tokio::test]
async fn a_credential_that_resolves_to_nothing_fails_before_the_provider_is_built() {
    let fixture = Fixture::new(OPENAI_CONFIG);
    let request = RuntimeRequest {
        credentials: Some(resolver(None)),
        ..request(&fixture, HostSurface::Headless)
    };

    let err = factory::build_request(request)
        .await
        .expect_err("no credential");
    assert!(matches!(err, FactoryError::Credential(_)), "{err}");
    // The locator is named; nothing else could be, because nothing was read.
    let rendered = format!("{err} {err:?}");
    assert!(rendered.contains("env:ACME_API_KEY"), "{rendered}");
}

#[tokio::test]
async fn a_platform_credential_prompt_cannot_hang_startup_forever() {
    let config = OPENAI_CONFIG.replace("env:ACME_API_KEY", "keychain:smith/remote");
    let fixture = Fixture::new(&config);
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let credentials = CredentialResolver::new("/nonexistent-user-state").with_keychain(Arc::new(
        WaitingKeychain {
            gate: Arc::clone(&gate),
        },
    ));
    let mut request = RuntimeRequest {
        credentials: Some(credentials),
        ..request(&fixture, HostSurface::Headless)
    };
    request.credential_timeout_ms = 10;

    let err = factory::build_request(request)
        .await
        .expect_err("blocked credential lookup");
    assert!(
        matches!(err, FactoryError::CredentialTimeout { timeout_ms: 10 }),
        "{err}"
    );
    let rendered = err.to_string();
    assert!(rendered.contains("env:<VAR>"), "{rendered}");
    assert!(!rendered.contains(TOKEN), "{rendered}");

    let (lock, ready) = &*gate;
    *lock.lock().expect("credential test gate") = true;
    ready.notify_all();
}

#[tokio::test]
async fn a_run_with_no_credential_resolver_says_so_rather_than_starting_unauthenticated() {
    let fixture = Fixture::new(OPENAI_CONFIG);

    let err = factory::build_request(request(&fixture, HostSurface::Headless))
        .await
        .expect_err("no resolver");
    assert!(
        matches!(
            err,
            FactoryError::MissingHostPolicy {
                what: "credential resolver",
                ..
            }
        ),
        "{err}"
    );
}

#[tokio::test]
async fn a_resolved_secret_reaches_no_event_snapshot_journal_or_error() {
    let fixture = Fixture::new(OPENAI_CONFIG);
    let state = tempfile::tempdir().expect("a state root");
    let journal_path = state.path().join("session.jsonl");
    // The observer exists before credential resolution, as it does in the
    // standard host. Its shared registry is populated by the one factory.
    let redactor = DefaultRedactor::new();
    let journal = Arc::new(
        EventJournal::open(
            &journal_path,
            JournalConfig::default(),
            Arc::new(redactor.clone()),
        )
        .await
        .expect("a journal"),
    );
    let recorder = RecordingObserver::shared();

    let request = RuntimeRequest {
        credentials: Some(resolver(Some(TOKEN))),
        persistence_redactor: Some(redactor.clone()),
        observers: vec![
            Arc::clone(&journal) as Arc<dyn EventObserver>,
            Arc::clone(&recorder) as Arc<dyn EventObserver>,
        ],
        ..request(&fixture, HostSurface::Headless)
    };
    let smith = factory::build_request(request).await.expect("a runtime");

    let mut reflected = serde_json::json!({"text": format!("reflected {TOKEN}")});
    redactor.redact(&mut reflected);
    let reflected = reflected.to_string();
    assert!(!reflected.contains(TOKEN), "{reflected}");
    assert!(reflected.contains("[redacted]"), "{reflected}");
    assert!(!format!("{redactor:?}").contains(TOKEN));

    // The composition record keeps the reference, never the value.
    assert_eq!(
        smith.policy().credential.as_deref(),
        Some("env:ACME_API_KEY")
    );
    let composition = format!(
        "{:?} {:?} {:?}",
        smith.policy(),
        smith.profile(),
        smith.runtime()
    );
    assert!(!composition.contains(TOKEN), "{composition}");

    // One turn against a closed port: the attempt fails in transport, which is
    // where an error that echoed its request would leak the authorization it
    // was sent with.
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");
    let snapshot = serde_json::to_string(&session.snapshot()).expect("a serialized snapshot");
    session.shutdown().await.expect("a clean shutdown");
    let stats = journal.shutdown().await.expect("a flushed journal");
    let journaled = std::fs::read_to_string(&journal_path).expect("a journal file");
    let events = format!("{:?}", recorder.events());

    assert!(stats.written > 0, "the journal recorded nothing to check");
    for (what, rendered) in [
        ("the snapshot", &snapshot),
        ("the journal", &journaled),
        ("the events", &events),
    ] {
        assert!(!rendered.contains(TOKEN), "{what} contains the credential");
    }
    // The failure itself was recorded, so the check above ran against a real
    // error path rather than an empty stream.
    assert!(
        events.contains("Error") || events.contains("error"),
        "{events}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn an_inline_user_key_bypasses_resolvers_and_reaches_no_runtime_surface() {
    let config = OPENAI_CONFIG.replace(
        "credential = \"env:ACME_API_KEY\"",
        &format!("api_key = \"{TOKEN}\""),
    );
    let fixture = Fixture::new_private_user(&config);
    let state = tempfile::tempdir().expect("a state root");
    let journal_path = state.path().join("inline-session.jsonl");
    let redactor = DefaultRedactor::new();
    let journal = Arc::new(
        EventJournal::open(
            &journal_path,
            JournalConfig::default(),
            Arc::new(redactor.clone()),
        )
        .await
        .expect("a journal"),
    );
    let recorder = RecordingObserver::shared();
    let credentials = CredentialResolver::new("/nonexistent-user-state")
        .with_keychain(Arc::new(PanicsIfCredentialResolved))
        .with_environment(Arc::new(PanicsIfCredentialResolved));
    let request = RuntimeRequest {
        credentials: Some(credentials),
        persistence_redactor: Some(redactor.clone()),
        observers: vec![
            Arc::clone(&journal) as Arc<dyn EventObserver>,
            Arc::clone(&recorder) as Arc<dyn EventObserver>,
        ],
        ..request(&fixture, HostSurface::Headless)
    };

    let smith = factory::build_request(request)
        .await
        .expect("inline-key runtime construction");
    assert!(
        smith.policy().credential.is_none(),
        "runtime policy must not invent a display-safe credential locator"
    );
    let mut reflected = serde_json::json!({"value": TOKEN});
    redactor.redact(&mut reflected);
    assert_eq!(reflected["value"], "[redacted]");

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");
    let snapshot = serde_json::to_string(&session.snapshot()).expect("a serialized snapshot");
    session.shutdown().await.expect("a clean shutdown");
    journal.shutdown().await.expect("a flushed journal");
    let journaled = std::fs::read_to_string(journal_path).expect("a journal file");
    let events = format!("{:?}", recorder.events());
    let composition = format!(
        "{:?} {:?} {:?}",
        smith.policy(),
        smith.profile(),
        smith.runtime()
    );

    for (what, rendered) in [
        ("snapshot", snapshot),
        ("journal", journaled),
        ("events", events),
        ("composition", composition),
        ("redactor", format!("{redactor:?}")),
    ] {
        assert!(!rendered.contains(TOKEN), "{what} contains the inline key");
    }
}
