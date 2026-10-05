use super::*;

#[tokio::test]
async fn a_session_is_saved_listed_and_resumed_with_its_canonical_history() {
    let fixture = Fixture::new();
    let host = start(fixture.request(HostSurface::Terminal))
        .await
        .expect("a new hosted session");
    let session_id = host.session().id().clone();
    let paths = host.paths().expect("persistent paths").clone();

    host.session()
        .run(UserInput::text("remember this"))
        .await
        .expect("the turn runs");
    host.session()
        .run(UserInput::text("and preserve every manifest"))
        .await
        .expect("the second turn runs");
    assert!(
        host.session()
            .history()
            .iter()
            .any(|message| message.joined_text().contains("remember this")),
        "the canonical history did not retain the user input"
    );
    assert_eq!(host.session().snapshot().manifests.len(), 2);
    assert!(
        paths
            .snapshot(&session_id)
            .expect("snapshot path")
            .is_file(),
        "a completed turn must be persisted before orderly shutdown"
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
    assert_eq!(
        resumed.session().snapshot().manifests.len(),
        2,
        "resume discarded historical manifests"
    );
    assert!(
        resumed
            .session()
            .history()
            .iter()
            .any(|message| message.joined_text().contains("remember this")),
        "resume discarded the prior canonical history"
    );
    resumed
        .session()
        .run(UserInput::text("append a third manifest"))
        .await
        .expect("the resumed turn runs");
    assert_eq!(
        resumed.session().snapshot().manifests.len(),
        3,
        "the resumed turn replaced historical manifests"
    );
    resumed.shutdown().await.expect("a clean resumed shutdown");

    let stored = FileSessionStore::new(paths.clone())
        .load(&session_id)
        .await
        .expect("saved snapshot")
        .expect("snapshot remains present");
    assert_eq!(
        stored.manifests.len(),
        3,
        "the resumed save lost historical manifests"
    );

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
    first
        .session()
        .run(UserInput::text("first turn"))
        .await
        .expect("the first turn runs");
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

    second
        .session()
        .run(UserInput::text("second turn"))
        .await
        .expect("the second turn runs");
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
    let paths = smith_runtime::host::paths(&fixture.config(), fixture.project.path())
        .expect("session paths");

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
    assert!(
        !paths.directory().exists(),
        "a missing resume identity created `{}`",
        paths.directory().display()
    );
}

#[tokio::test]
async fn cold_resume_selects_the_newer_canonical_capsule_after_runtime_startup() {
    let fixture = Fixture::new();
    let session = SessionId::new("session-cold-capsule-ordering");
    let turn = TurnId::new("turn-cold-capsule-ordering");
    let child = ChildId::new("running-before-restart");
    let input = UserInput::text("resume exact state after a cold restart");
    let paths = smith_runtime::host::paths(&fixture.config(), fixture.project.path()).unwrap();

    let canonical_capsule = ResumeCapsuleSlot::new(session.clone(), Timestamp(20));
    canonical_capsule.update(|capsule| {
        let mut children = std::collections::BTreeMap::new();
        children.insert(
            child.clone(),
            ChildResumeProjection {
                child: child.clone(),
                task_digest: None,
                state: ChildLifecycleState::Running,
                terminal_outcome: None,
                watermark: 20,
            },
        );
        assert!(capsule.commit_exact_state(
            ExactResumeState {
                watermark: 20,
                children,
                ..ExactResumeState::default()
            },
            Timestamp(20),
        ));
        capsule.cache.provider_warmth = ResumeCacheWarmth::WarmObserved;
        capsule.cache.guaranteed_until = Some(Timestamp(99));
    });

    let protected_capsule = ResumeCapsuleSlot::new(session.clone(), Timestamp(10));
    protected_capsule.update(|capsule| {
        let mut children = std::collections::BTreeMap::new();
        children.insert(
            child.clone(),
            ChildResumeProjection {
                child: child.clone(),
                task_digest: None,
                state: ChildLifecycleState::Completed,
                terminal_outcome: Some(ChildTerminalOutcome {
                    state: ChildLifecycleState::Completed,
                    result_digest: None,
                    watermark: 10,
                }),
                watermark: 10,
            },
        );
        assert!(capsule.commit_exact_state(
            ExactResumeState {
                watermark: 10,
                children,
                ..ExactResumeState::default()
            },
            Timestamp(10),
        ));
        capsule.cache.provider_warmth = ResumeCacheWarmth::MissObserved;
    });

    let mut canonical = SessionSnapshot {
        id: session.clone(),
        history: vec![input.clone().into_message()],
        usage: UsageLedger::new(),
        identity: SessionIdentityState {
            turn: 1,
            event: 20,
            event_seq: 20,
            ..SessionIdentityState::default()
        },
        manifests: Vec::new(),
        extension_state: Default::default(),
        updated: Timestamp(20),
    };
    canonical.extension_state.insert(
        RESUME_CAPSULE_STATE_NAMESPACE.to_owned(),
        canonical_capsule.versioned_state().unwrap(),
    );
    FileSessionStore::new(paths.clone())
        .save(&canonical)
        .await
        .expect("the newer canonical capsule saves");

    let mut protected = SessionSnapshot {
        id: session.clone(),
        history: vec![input.clone().into_message()],
        usage: UsageLedger::new(),
        identity: SessionIdentityState {
            turn: 1,
            event: 10,
            event_seq: 10,
            ..SessionIdentityState::default()
        },
        manifests: Vec::new(),
        extension_state: Default::default(),
        updated: Timestamp(10),
    };
    protected.extension_state.insert(
        RESUME_CAPSULE_STATE_NAMESPACE.to_owned(),
        protected_capsule.versioned_state().unwrap(),
    );
    let checkpoint_store = SmithCheckpointStore::initialize_with(paths, test_checkpoint_keys())
        .await
        .expect("a protected checkpoint store");
    let accepted = TurnCheckpoint::accepted(
        turn,
        input,
        protected.clone(),
        0,
        Deadline::never(),
        1,
        10,
        Timestamp(10),
    )
    .expect("accepted checkpoint");
    checkpoint_store
        .save(&accepted)
        .await
        .expect("accepted checkpoint saves");
    let completing = accepted
        .transition(
            TurnState::Completing {
                finish: TurnFinish::Completed,
                visible_output: false,
                provider_error_kind: None,
            },
            protected.clone(),
            11,
            Timestamp(11),
        )
        .expect("completing checkpoint");
    checkpoint_store
        .save(&completing)
        .await
        .expect("completing checkpoint saves");
    let publishing = completing
        .transition(
            TurnState::PublishingTerminal {
                finish: TurnFinish::Completed,
                visible_output: false,
            },
            protected.clone(),
            12,
            Timestamp(12),
        )
        .expect("publishing checkpoint");
    checkpoint_store
        .save(&publishing)
        .await
        .expect("publishing checkpoint saves");
    let terminal = publishing
        .transition(
            TurnState::Terminal {
                finish: TurnFinish::Completed,
                visible_output: false,
            },
            protected,
            13,
            Timestamp(13),
        )
        .expect("terminal checkpoint");
    checkpoint_store
        .save(&terminal)
        .await
        .expect("terminal checkpoint saves");

    let resumed = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session.clone()),
    )
    .await
    .expect("the cold session resumes");
    let capsule = resumed.resume_capsule().expect("resume capsule");
    assert_eq!(capsule.last_persisted_watermark, 20);
    assert_eq!(capsule.cache.provider_warmth, ResumeCacheWarmth::Unknown);
    assert_eq!(capsule.cache.guaranteed_until, None);
    assert_eq!(
        capsule.exact_state.children[&child].state,
        ChildLifecycleState::InterruptedByProcessExit
    );
    resumed.shutdown().await.expect("clean cold shutdown");
}

#[tokio::test]
async fn missing_resume_summary_artifact_does_not_abort_exact_cold_resume() {
    let fixture = Fixture::new();
    let session = SessionId::new("session-missing-summary-artifact");
    let paths = smith_runtime::host::paths(&fixture.config(), fixture.project.path()).unwrap();
    let artifacts = SmithArtifactStore::new(paths.clone());
    let reference = artifacts
        .put(agent_runtime_core::artifact::ArtifactWrite {
            bytes: b"summary body".to_vec(),
            media_type: smith_runtime::resume_capsule::RESUME_SUMMARY_MEDIA_TYPE.to_owned(),
            sensitivity: agent_runtime_core::artifact::ArtifactSensitivity::Sensitive,
            retention: agent_runtime_core::artifact::ArtifactRetention::Session,
            provenance: agent_runtime_core::artifact::ArtifactProvenance::new(
                session.clone(),
                smith_runtime::resume_capsule::RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
            ),
            idempotency_key: "missing-summary-host-r1".to_owned(),
        })
        .await
        .expect("summary artifact writes");
    let capsule = ResumeCapsuleSlot::new(session.clone(), Timestamp(1));
    capsule
        .update(|capsule| {
            capsule.record_ordinary_summary(
                "provider",
                "summary-model",
                RegistryRevision::new("summary-r1"),
                Timestamp(2),
                "summary body",
                vec![],
            )?;
            capsule.attach_summary_artifact(reference)
        })
        .expect("summary metadata attaches");

    let mut snapshot = SessionSnapshot {
        id: session.clone(),
        history: Vec::new(),
        usage: UsageLedger::new(),
        identity: SessionIdentityState::default(),
        manifests: Vec::new(),
        extension_state: Default::default(),
        updated: Timestamp(1),
    };
    snapshot.extension_state.insert(
        RESUME_CAPSULE_STATE_NAMESPACE.to_owned(),
        capsule.versioned_state().expect("versioned capsule"),
    );
    FileSessionStore::new(paths)
        .save(&snapshot)
        .await
        .expect("canonical snapshot saves");
    std::fs::remove_dir_all(artifacts.directory()).expect("remove optional summary artifact");

    let resumed = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session.clone()),
    )
    .await
    .expect("missing optional artifact must not abort resume");
    let capsule = resumed.resume_capsule().expect("resume capsule");
    let summary = capsule.semantic_summary.expect("summary metadata");
    assert_eq!(
        summary.provenance.outcome,
        smith_runtime::resume_capsule::ResumeSummaryOutcome::Missing
    );
    assert!(summary.provenance.summary_artifact.is_none());
    resumed.shutdown().await.expect("clean shutdown");
}
