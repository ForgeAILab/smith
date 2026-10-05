use super::*;

#[tokio::test]
async fn protected_checkpoint_availability_is_explicit_and_encrypted() {
    let fixture = Fixture::new();
    let host = start(fixture.request(HostSurface::Headless))
        .await
        .expect("a hosted session");
    assert_eq!(
        host.runtime().policy().mid_turn_durability,
        MidTurnDurability::Available
    );
    let session_id = host.session().id().clone();
    host.session()
        .run(UserInput::text("checkpoint secret marker"))
        .await
        .expect("the turn runs");
    let checkpoint = host
        .paths()
        .unwrap()
        .checkpoint(&session_id)
        .expect("checkpoint path");
    let bytes = tokio::fs::read(checkpoint)
        .await
        .expect("encrypted checkpoint");
    assert!(
        !bytes
            .windows("checkpoint secret marker".len())
            .any(|window| { window == b"checkpoint secret marker" })
    );
    host.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn persistence_does_not_switch_on_semantic_summarization() {
    const KEY: &str = "5151515151515151515151515151515151515151515151515151515151515151";
    let home = tempfile::tempdir().expect("a user root");
    let project = tempfile::tempdir().expect("a project root");
    let config_dir = home.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a user config directory");
    let config_path = config_dir.join("config.toml");
    std::fs::write(
        &config_path,
        format!("{CONFIG}\n[persistence]\nenabled = true\ncheckpoint_key = \"{KEY}\"\n"),
    )
    .expect("a private user config");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600))
            .expect("private config permissions");
    }
    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("the configured checkpoint key resolves")
        .config;
    assert!(config.persistence.enabled.value, "persistence is on");

    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a project workspace"),
        )),
        approval: Some(Arc::new(AllowAll)),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let request = HostSessionRequest::new(runtime, project.path());

    // Persistence says where a session is stored, not that a second model
    // route should summarize it. A caller that wants summarization still asks
    // for it explicitly.
    assert!(
        request.runtime.semantic_summary.is_none(),
        "enabling persistence must not install a semantic-summary coordinator"
    );
}

#[tokio::test]
async fn configured_clear_checkpoint_key_makes_children_durable_without_keychain_fallback() {
    const KEY: &str = "5151515151515151515151515151515151515151515151515151515151515151";
    let home = tempfile::tempdir().expect("a user root");
    let project = tempfile::tempdir().expect("a project root");
    let config_dir = home.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a user config directory");
    let config_path = config_dir.join("config.toml");
    std::fs::write(
        &config_path,
        format!("{CONFIG}\n[persistence]\nenabled = true\ncheckpoint_key = \"{KEY}\"\n"),
    )
    .expect("a private user config");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600))
            .expect("private config permissions");
    }
    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("the configured checkpoint key resolves")
        .config;
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta {
                text: "durable child".to_owned(),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])],
    ));
    let runtime = RuntimeRequest {
        provider: Some(provider),
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a project workspace"),
        )),
        approval: Some(Arc::new(AllowAll)),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    // Deliberately do not call HostSessionRequest::checkpoint_keys: startup
    // must select the resolved inline key before considering the platform
    // credential service.
    let host = start(HostSessionRequest::new(runtime, project.path()))
        .await
        .expect("the configured non-prompt key initializes persistence");
    assert_eq!(
        host.runtime().policy().mid_turn_durability,
        MidTurnDurability::Available
    );
    let coordinator = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .expect("a delegation coordinator");
    let child = match coordinator
        .spawn(ChildSpec {
            task: UserInput::text("verify configured child durability"),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(1),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the child starts")
    {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };
    coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the child completes");
    assert_eq!(
        coordinator.status(&child).expect("child status").durability,
        ChildDurability::Durable
    );
    host.shutdown().await.expect("the host shuts down");
}

#[tokio::test]
async fn unavailable_checkpoint_key_keeps_completed_turn_persistence_honest() {
    let fixture = Fixture::new();
    let mut request = fixture.request(HostSurface::Headless);
    request.checkpoint_keys = Some(Arc::new(UnavailableCheckpointKeys));
    let host = start(request)
        .await
        .expect("completed-turn persistence remains");
    assert_eq!(
        host.runtime().policy().mid_turn_durability,
        MidTurnDurability::Unavailable
    );
    let session_id = host.session().id().clone();
    host.session()
        .run(UserInput::text("completed turn"))
        .await
        .expect("a turn without false checkpoint durability");
    let paths = host.paths().unwrap().clone();
    host.shutdown().await.expect("clean shutdown");
    assert!(paths.snapshot(&session_id).unwrap().is_file());
    assert!(!paths.checkpoint(&session_id).unwrap().exists());
}

#[tokio::test]
async fn terminal_resume_merges_protected_sensitive_state_over_the_plaintext_snapshot() {
    const SECRET: &str = "protected-memory-state-8f31";

    let fixture = Fixture::new();
    let session = SessionId::new("session-terminal-extension-state");
    let turn = TurnId::new("turn-1");
    let input = UserInput::text("retain protected component state");
    let paths = smith_runtime::host::paths(&fixture.config(), fixture.project.path()).unwrap();
    let mut snapshot = SessionSnapshot {
        id: session.clone(),
        history: vec![input.clone().into_message()],
        usage: UsageLedger::new(),
        identity: SessionIdentityState {
            turn: 1,
            event: 10,
            event_seq: 11,
            ..SessionIdentityState::default()
        },
        manifests: Vec::new(),
        extension_state: Default::default(),
        updated: Timestamp(4),
    };
    snapshot.extension_state.insert(
        "smith.todo".into(),
        VersionedSessionState::new(
            RegistryRevision::new("todo-state-1"),
            serde_json::json!({"pending": 1}),
        )
        .redaction_safe(),
    );
    snapshot.extension_state.insert(
        "smith.memory".into(),
        VersionedSessionState::new(
            RegistryRevision::new("memory-state-1"),
            serde_json::json!({"content": SECRET}),
        ),
    );

    let session_store = FileSessionStore::new(paths.clone());
    session_store
        .save(&snapshot)
        .await
        .expect("the ordinary snapshot saves");
    let ordinary = session_store
        .load(&session)
        .await
        .expect("the ordinary snapshot loads")
        .expect("an ordinary snapshot exists");
    assert!(ordinary.extension_state.contains_key("smith.todo"));
    assert!(!ordinary.extension_state.contains_key("smith.memory"));
    let ordinary_bytes = tokio::fs::read(paths.snapshot(&session).expect("ordinary snapshot path"))
        .await
        .expect("ordinary snapshot bytes");
    assert!(
        !ordinary_bytes
            .windows(SECRET.len())
            .any(|window| window == SECRET.as_bytes())
    );

    let checkpoint_store =
        SmithCheckpointStore::initialize_with(paths.clone(), test_checkpoint_keys())
            .await
            .expect("a protected checkpoint store");
    let accepted = TurnCheckpoint::accepted(
        turn,
        input,
        snapshot.clone(),
        0,
        Deadline::never(),
        1,
        7,
        Timestamp(1),
    )
    .expect("an accepted checkpoint");
    checkpoint_store
        .save(&accepted)
        .await
        .expect("accepted checkpoint");
    let completing = accepted
        .transition(
            TurnState::Completing {
                finish: TurnFinish::Completed,
                visible_output: false,
                provider_error_kind: None,
            },
            snapshot.clone(),
            8,
            Timestamp(2),
        )
        .expect("a completing checkpoint");
    checkpoint_store
        .save(&completing)
        .await
        .expect("completing checkpoint");
    let publishing = completing
        .transition(
            TurnState::PublishingTerminal {
                finish: TurnFinish::Completed,
                visible_output: false,
            },
            snapshot.clone(),
            9,
            Timestamp(3),
        )
        .expect("a publishing checkpoint");
    checkpoint_store
        .save(&publishing)
        .await
        .expect("publishing checkpoint");
    let terminal = publishing
        .transition(
            TurnState::Terminal {
                finish: TurnFinish::Completed,
                visible_output: false,
            },
            snapshot,
            10,
            Timestamp(4),
        )
        .expect("a terminal checkpoint");
    checkpoint_store
        .save(&terminal)
        .await
        .expect("terminal checkpoint");

    let resumed = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session.clone()),
    )
    .await
    .expect("the terminal session resumes");
    let restored = resumed.session().snapshot();
    assert_eq!(
        restored.extension_state["smith.memory"].value,
        serde_json::json!({"content": SECRET})
    );
    assert_eq!(
        restored.extension_state["smith.todo"].value,
        serde_json::json!({"pending": 1})
    );
    resumed.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_checkpoint_only_first_turn_is_resumable_without_a_completed_snapshot() {
    let fixture = Fixture::new();
    let session = SessionId::new("session-first-turn-crash");
    let turn = TurnId::new("turn-1");
    let input = UserInput::text("resume the first accepted turn");
    let paths = smith_runtime::host::paths(&fixture.config(), fixture.project.path()).unwrap();
    let mut snapshot = SessionSnapshot {
        id: session.clone(),
        history: vec![input.clone().into_message()],
        usage: UsageLedger::new(),
        identity: SessionIdentityState {
            turn: 1,
            event: 7,
            event_seq: 7,
            ..SessionIdentityState::default()
        },
        manifests: Vec::new(),
        extension_state: Default::default(),
        updated: Timestamp::ZERO,
    };
    let checkpoint = TurnCheckpoint::accepted(
        turn,
        input,
        snapshot.clone(),
        0,
        Deadline::never(),
        1,
        7,
        Timestamp::ZERO,
    )
    .unwrap();
    let checkpoint_store =
        SmithCheckpointStore::initialize_with(paths.clone(), test_checkpoint_keys())
            .await
            .unwrap();
    checkpoint_store.save(&checkpoint).await.unwrap();
    assert!(
        !paths.snapshot(&session).unwrap().exists(),
        "the fixture must model a crash before the first completed snapshot"
    );

    let resumed = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session.clone()),
    )
    .await
    .expect("the protected checkpoint proves the session exists");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            snapshot = resumed.session().snapshot();
            if snapshot
                .history
                .iter()
                .any(|message| message.role == agent_runtime_core::content::Role::Assistant)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the accepted turn resumed");
    resumed.shutdown().await.expect("clean shutdown");
    assert!(paths.snapshot(&session).unwrap().is_file());
}

#[tokio::test]
async fn only_one_host_can_own_a_persistent_session_lifecycle() {
    let fixture = Fixture::new();
    let first = start(fixture.request(HostSurface::Headless))
        .await
        .expect("the first host");
    first
        .session()
        .run(UserInput::text("establish a resumable session"))
        .await
        .expect("a completed turn");
    let session = first.session().id().clone();

    let error = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session.clone()),
    )
    .await
    .expect_err("a second active owner must fail instead of waiting");
    assert!(
        matches!(
            error,
            HostSessionError::Runtime(ref error)
                if error.kind == agent_runtime_core::error::ErrorKind::Conflict
                    && error.message.contains("already active")
        ),
        "{error}"
    );

    first.shutdown().await.expect("release the lifecycle lease");
    let second = start(fixture.request(HostSurface::Headless).resume(session))
        .await
        .expect("the lease is recoverable after shutdown");
    second.shutdown().await.expect("clean second shutdown");
}
