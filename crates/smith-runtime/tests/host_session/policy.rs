use super::*;

#[tokio::test]
async fn project_configuration_cannot_silently_grant_tool_authority() {
    let home = tempfile::tempdir().expect("a user root");
    let project = tempfile::tempdir().expect("a project root");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a project config directory");
    std::fs::write(
        config_dir.join("config.toml"),
        format!("{CONFIG}\n[approval]\nmode = \"allow-all\"\n"),
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
async fn project_scoped_auto_approval_fails_preflight() {
    let home = tempfile::tempdir().expect("a user root");
    let project = tempfile::tempdir().expect("a project root");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a project config directory");
    std::fs::write(
        config_dir.join("config.toml"),
        format!(
            "{CONFIG}\n\
             [[approval.auto]]\n\
             revision = 1\n\
             tool = \"smith/edit\"\n\
             operations = [\"replace\"]\n\
             permissions = [\"fs.read\", \"fs.write\"]\n\
             max_risk = \"medium\"\n\
             mount = \"workspace\"\n\
             paths = [\"src/**\"]\n"
        ),
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
        .expect_err("project config must not grant scoped approval authority");
    assert!(
        matches!(
            error,
            HostSessionError::ProjectGrantedAuthority {
                setting: "approval.auto",
                ..
            }
        ),
        "{error}"
    );
    assert!(
        format!("{error}").contains("user configuration"),
        "the diagnostic says where the policy belongs: {error}"
    );
    assert!(
        !home.path().join(".smith/sessions").exists(),
        "authority failure created session state"
    );
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

    host.session()
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");
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
    let resumed = start(
        HostSessionRequest::new(resume.runtime, fixture.project.path())
            .checkpoint_keys(test_checkpoint_keys())
            .resume(session_id),
    )
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
