//! The lifecycle shared by the TUI and `smith -p`.
//!
//! These tests deliberately start above the runtime factory: configuration is
//! discovered from disk, the project workspace is real, and snapshots plus
//! canonical events are written below an injected user root. No test reaches
//! the network or the developer's home directory.

use std::sync::Arc;

use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
use agent_runtime_core::approval::AllowAll;
use agent_runtime_core::content::UserInput;
use agent_runtime_core::event::{RuntimeEvent, canonical_payloads};
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::provider::{Capabilities, FinishReason, Provider, ProviderStreamEvent};
use agent_runtime_testkit::RecordingObserver;
use futures_util::StreamExt;
use smith_config::resolve::{Overrides, ResolveRequest, ResolvedConfig, resolve};
use smith_host::ProjectWorkspace;
use smith_runtime::factory::{HostSurface, RuntimeRequest};
use smith_runtime::host::{HostSessionError, HostSessionRequest, list, start};
use smith_runtime::journal::{DefaultRedactor, read_journal};

const CONFIG: &str = r#"
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

fn edit_provider() -> Arc<dyn Provider> {
    let mut edit = tool_call_fragments(
        0,
        "edit-1",
        "edit",
        r#"{"path":"tracked.txt","old_string":"before\n","new_string":"after\n"}"#,
    );
    edit.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(edit),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "edited".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ))
}

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self::with_config(CONFIG)
    }

    fn with_config(config: &str) -> Self {
        let home = tempfile::tempdir().expect("a user root");
        let project = tempfile::tempdir().expect("a project root");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("a project config directory");
        std::fs::write(config_dir.join("config.toml"), config).expect("a project config");
        Self { home, project }
    }

    fn config(&self) -> ResolvedConfig {
        resolve(&ResolveRequest::new(self.project.path()).with_home_dir(self.home.path()))
            .expect("resolved configuration")
            .config
    }

    fn request(&self, surface: HostSurface) -> HostSessionRequest {
        self.request_with_config(self.config(), surface)
    }

    fn request_with_config(
        &self,
        config: ResolvedConfig,
        surface: HostSurface,
    ) -> HostSessionRequest {
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(self.project.path()).expect("a project workspace"),
            )),
            approval: Some(Arc::new(AllowAll)),
            ..RuntimeRequest::new(config, surface)
        };
        HostSessionRequest::new(runtime, self.project.path())
    }
}

#[tokio::test]
async fn project_configuration_cannot_silently_grant_tool_authority() {
    let home = tempfile::tempdir().expect("a user root");
    let project = tempfile::tempdir().expect("a project root");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a project config directory");
    std::fs::write(
        config_dir.join("config.toml"),
        format!("{CONFIG}\n[approval]\nmode = \"allow-all\"\nauto_approve = [\"edit\"]\n"),
    )
    .expect("a project config");
    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved configuration")
        .config;
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a workspace"),
        )),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };

    let error = start(HostSessionRequest::new(runtime, project.path()))
        .await
        .expect_err("project config must not grant authority");
    assert!(
        matches!(
            error,
            HostSessionError::ProjectGrantedAuthority {
                setting: "approval.mode",
                ..
            }
        ),
        "{error}"
    );
    assert!(
        !home.path().join(".smith/sessions").exists(),
        "authority failure created session state"
    );
}

#[tokio::test]
async fn a_session_is_saved_listed_and_resumed_with_its_canonical_history() {
    let fixture = Fixture::new();
    let host = start(fixture.request(HostSurface::Terminal))
        .await
        .expect("a new hosted session");
    let session_id = host.session().id().clone();
    let paths = host.paths().expect("persistent paths").clone();

    host.session().run(UserInput::text("remember this")).await;
    assert!(
        host.session()
            .history()
            .iter()
            .any(|message| message.joined_text().contains("remember this")),
        "the canonical history did not retain the user input"
    );
    host.shutdown().await.expect("a clean first shutdown");

    assert!(
        paths
            .snapshot(&session_id)
            .expect("snapshot path")
            .is_file()
    );
    assert!(paths.journal(&session_id).expect("journal path").is_file());
    let listings = list(&fixture.config(), fixture.project.path())
        .await
        .expect("session listings");
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].id, session_id);

    let resumed = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session_id.clone()),
    )
    .await
    .expect("a resumed hosted session");
    assert_eq!(resumed.session().id(), &session_id);
    assert!(
        resumed
            .session()
            .history()
            .iter()
            .any(|message| message.joined_text().contains("remember this")),
        "resume discarded the prior canonical history"
    );
    resumed.shutdown().await.expect("a clean resumed shutdown");

    let recovery = read_journal(paths.journal(&session_id).expect("journal path"))
        .await
        .expect("a readable journal");
    assert!(
        recovery.events().len() >= 4,
        "create and resume did not both append canonical lifecycle events"
    );
    assert!(
        recovery.truncated_tail.is_none(),
        "ordered shutdown left a partial journal record"
    );
}

#[tokio::test]
async fn a_protected_live_event_resolves_its_safe_display_from_canonical_history() {
    const OLD: &str = "TOP_SECRET_OLD_BODY";
    const NEW: &str = "TOP_SECRET_NEW_BODY";

    let fixture = Fixture::new();
    std::fs::write(fixture.project.path().join("tracked.txt"), OLD).expect("target file");
    let mut edit = tool_call_fragments(
        0,
        "edit-display-1",
        "edit",
        &serde_json::json!({
            "path": "tracked.txt",
            "old_string": OLD,
            "new_string": NEW
        })
        .to_string(),
    );
    edit.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(edit),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "done".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let mut request = fixture.request(HostSurface::Terminal);
    request.runtime.provider = Some(provider);
    let host = start(request).await.expect("host");
    let journal_path = host
        .paths()
        .expect("persistent paths")
        .journal(host.session().id())
        .expect("journal path");
    let mut events = host.session().subscribe();
    host.session()
        .send(UserInput::text("perform the reviewed edit"));

    let invocation = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut invocation = None;
        while let Some(event) = events.next().await {
            match &event.payload {
                RuntimeEvent::ToolCallRequested {
                    call, arguments, ..
                } => {
                    assert!(
                        arguments.is_none(),
                        "Smith must not opt raw arguments into canonical events"
                    );
                    invocation = host
                        .tool_call_display(call)
                        .map(|display| display.invocation());
                }
                RuntimeEvent::TurnCompleted { .. } => break,
                _ => {}
            }
        }
        invocation
    })
    .await
    .expect("turn completed")
    .expect("safe display was available when the protected event arrived");

    assert_eq!(invocation, "Edit(tracked.txt)");
    host.shutdown().await.expect("clean shutdown");
    let journal = std::fs::read_to_string(journal_path).expect("event journal");
    assert!(!journal.contains(OLD), "{journal}");
    assert!(!journal.contains(NEW), "{journal}");
}

#[tokio::test]
async fn an_edit_turn_is_attributed_and_undoable_through_the_host() {
    let fixture = Fixture::new();
    std::fs::write(fixture.project.path().join("tracked.txt"), "before\n").expect("file");
    let mut request = fixture.request(HostSurface::Terminal);
    request.runtime.provider = Some(edit_provider());
    let host = start(request).await.expect("host");

    host.session().run(UserInput::text("edit the file")).await;
    let set = host.changes().latest().expect("change set");
    assert!(set.is_fully_attributable());
    assert!(
        host.changes()
            .undo_preview()
            .expect("preview")
            .contains("-after")
    );
    host.changes().undo_latest().expect("undo");
    assert_eq!(
        std::fs::read_to_string(fixture.project.path().join("tracked.txt")).expect("read"),
        "before\n"
    );
    host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn registered_credentials_are_removed_from_persisted_history_and_events() {
    const SECRET: &str = "sk-live-persistence-secret";

    let fixture = Fixture::new();
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(Arc::new(FakeProvider::text_reply(format!(
        "reflected {SECRET}"
    ))) as Arc<dyn Provider>);
    request.runtime.persistence_redactor = Some(DefaultRedactor::new().with_secret(SECRET));
    let host = start(request).await.expect("a hosted session");
    let paths = host.paths().expect("persistent paths").clone();
    let session_id = host.session().id().clone();

    host.session().run(UserInput::text("hello")).await;
    host.shutdown().await.expect("a clean shutdown");

    let snapshot = std::fs::read_to_string(
        paths
            .snapshot(&session_id)
            .expect("a persisted snapshot path"),
    )
    .expect("a persisted snapshot");
    let journal = std::fs::read_to_string(
        paths
            .journal(&session_id)
            .expect("a persisted journal path"),
    )
    .expect("a persisted journal");
    for (name, persisted) in [("snapshot", snapshot), ("journal", journal)] {
        assert!(!persisted.contains(SECRET), "{name} leaked the credential");
        assert!(
            persisted.contains("[redacted]"),
            "{name} did not retain an explicit redaction marker"
        );
    }

    let mut resume = fixture.request(HostSurface::Headless);
    resume.runtime.persistence_redactor = Some(DefaultRedactor::new().with_secret(SECRET));
    let resumed =
        start(HostSessionRequest::new(resume.runtime, fixture.project.path()).resume(session_id))
            .await
            .expect("a redacted session still resumes");
    let history = resumed
        .session()
        .history()
        .iter()
        .map(|message| message.joined_text())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!history.contains(SECRET), "{history}");
    assert!(history.contains("[redacted]"), "{history}");
    resumed.shutdown().await.expect("a clean resumed shutdown");
}

#[tokio::test]
async fn provider_switch_rebuilds_and_resumes_the_same_canonical_session() {
    const SWITCH_CONFIG: &str = r#"
default_profile = "first"

[profiles.first]
provider = "alpha"
model = "model-a"

[profiles.second]
provider = "beta"
model = "model-b"

[providers.alpha]
kind = "fake"

[providers.beta]
kind = "fake"

[models."alpha/model-a"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[models."beta/model-b"]
context_tokens = 64000
max_input_tokens = 60000
max_output_tokens = 2048
"#;

    let fixture = Fixture::with_config(SWITCH_CONFIG);
    let first = start(fixture.request(HostSurface::Terminal))
        .await
        .expect("the first runtime");
    let session_id = first.session().id().clone();
    first.session().run(UserInput::text("first turn")).await;
    first.shutdown().await.expect("the first runtime saved");

    let switched = resolve(
        &ResolveRequest::new(fixture.project.path())
            .with_home_dir(fixture.home.path())
            .with_cli(Overrides {
                profile: Some("second".into()),
                ..Overrides::default()
            }),
    )
    .expect("the second profile resolves")
    .config;
    let second = start(
        fixture
            .request_with_config(switched, HostSurface::Terminal)
            .resume(session_id.clone()),
    )
    .await
    .expect("the rebuilt runtime resumes");
    assert_eq!(second.session().id(), &session_id);
    assert_eq!(second.runtime().policy().provider_name, "beta");
    assert_eq!(second.runtime().policy().model.as_str(), "model-b");
    assert!(
        second
            .session()
            .history()
            .iter()
            .any(|message| message.joined_text().contains("first turn")),
        "the switch discarded canonical history"
    );

    second.session().run(UserInput::text("second turn")).await;
    let snapshot = second.session().snapshot();
    assert_eq!(
        snapshot
            .manifests
            .last()
            .expect("a second manifest")
            .manifest
            .model
            .provider,
        "beta"
    );
    second.shutdown().await.expect("the switched runtime saved");
}

#[tokio::test]
async fn resume_refuses_an_unknown_identity_instead_of_creating_it() {
    let fixture = Fixture::new();
    let missing = SessionId::new("session-does-not-exist");

    let error = start(
        fixture
            .request(HostSurface::Terminal)
            .resume(missing.clone()),
    )
    .await
    .expect_err("resume must not create a missing session");
    assert!(
        matches!(
            error,
            HostSessionError::SessionNotFound { ref session } if session == &missing
        ),
        "{error}"
    );
    assert!(
        list(&fixture.config(), fixture.project.path())
            .await
            .expect("session listings")
            .is_empty(),
        "a failed resume left session state behind"
    );
}

#[tokio::test]
async fn terminal_and_headless_hosts_emit_the_same_canonical_turn() {
    let fixture = Fixture::new();
    let mut runs = Vec::new();

    for surface in [HostSurface::Terminal, HostSurface::Headless] {
        let observer = RecordingObserver::shared();
        let mut request = fixture.request(surface);
        request.runtime.observers.push(observer.clone());
        let host = start(request).await.expect("a hosted session");
        let policy = host.runtime().policy().clone();
        host.session().run(UserInput::text("same input")).await;
        host.shutdown().await.expect("a clean shutdown");
        runs.push((policy, observer.events()));
    }

    assert_eq!(runs[0].0, runs[1].0, "surface changed runtime policy");
    assert_eq!(
        canonical_payloads(&runs[0].1),
        canonical_payloads(&runs[1].1),
        "surface changed canonical behavior"
    );
}

#[tokio::test]
async fn preflight_failure_does_not_create_a_journal_or_session_directory() {
    let fixture = Fixture::new();
    let config = fixture.config();
    let sessions_dir = config.persistence.sessions_dir.value.clone();
    let runtime = RuntimeRequest::new(config, HostSurface::Headless);

    let error = start(HostSessionRequest::new(runtime, fixture.project.path()))
        .await
        .expect_err("a missing workspace must fail preflight");
    assert!(
        matches!(error, HostSessionError::Factory(_)),
        "unexpected error: {error}"
    );
    assert!(
        !sessions_dir.exists(),
        "failed preflight created persistence state at {}",
        sessions_dir.display()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_project_paths_keep_distinct_session_partitions() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let root = tempfile::tempdir().expect("a root");
    let first = root.path().join(OsString::from_vec(vec![b'p', 0x80]));
    let second = root.path().join(OsString::from_vec(vec![b'p', 0x81]));
    std::fs::create_dir(&first).expect("first project");
    std::fs::create_dir(&second).expect("second project");

    assert_ne!(
        smith_runtime::host::project_id(first).expect("first id"),
        smith_runtime::host::project_id(second).expect("second id"),
        "lossy path conversion merged distinct projects"
    );
}
