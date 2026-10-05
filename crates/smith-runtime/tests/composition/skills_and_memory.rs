use super::*;

#[tokio::test]
async fn trusted_workspace_skill_loads_only_after_factory_descriptor_resolution() {
    const BODY: &[u8] = b"TRUSTED_SKILL_BODY_MARKER: inspect unsafe boundaries first.";

    let fixture = Fixture::new(FAKE_CONFIG);
    let skill_path = fixture.project.path().join(".smith/review.SKILL.md");
    let provider = Arc::new(FakeProvider::text_reply("reviewed"));
    let mut runtime = request(&fixture, HostSurface::Headless);
    runtime.provider = Some(provider.clone());
    runtime.skills = SmithSkillSources::new().with_workspace(
        Skill::from_verified_file(
            "rust-review",
            "Review Rust implementation boundaries",
            &skill_path,
            Sha256::digest(BODY).into(),
        ),
        TrustStatus::Trusted,
    );

    let smith = factory::build_request(runtime)
        .await
        .expect("descriptor resolution succeeds before the file exists");
    assert!(
        !skill_path.exists(),
        "factory eagerly opened the skill body"
    );
    assert_eq!(smith.policy().skills, ["rust-review"]);
    assert!(smith.skill_index().iter().all(|entry| entry.activatable));

    std::fs::write(&skill_path, BODY).expect("publish reviewed bytes after descriptor indexing");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text(
            "Use the rust-review skill to review this Rust implementation.",
        ))
        .await
        .expect("the trusted skill turn runs");
    session.shutdown().await.expect("clean shutdown");

    let wire = serde_json::to_string(&provider.requests()[0].messages).unwrap();
    assert!(wire.contains("TRUSTED_SKILL_BODY_MARKER"), "{wire}");
}

#[tokio::test]
async fn untrusted_workspace_skill_is_indexed_but_never_activates_or_shadows_user_skill() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let provider = Arc::new(FakeProvider::text_reply("reviewed"));
    let mut runtime = request(&fixture, HostSurface::Headless);
    runtime.provider = Some(provider.clone());
    runtime.skills = SmithSkillSources::new()
        .with_user(Skill::inline(
            "rust-review",
            "Review Rust implementation boundaries",
            "USER_SKILL_BODY_MARKER",
        ))
        .with_workspace(
            Skill::inline(
                "rust-review",
                "Review Rust implementation boundaries",
                "UNTRUSTED_WORKSPACE_BODY_MARKER",
            ),
            TrustStatus::Untrusted,
        );

    let smith = factory::build_request(runtime).await.expect("a runtime");
    assert_eq!(smith.policy().skills, ["rust-review"]);
    assert!(smith.skill_index().iter().any(|entry| {
        entry.layer == smith_runtime::skills::SmithSkillLayer::Workspace && !entry.activatable
    }));
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text(
            "Use the rust-review skill to review this Rust implementation.",
        ))
        .await
        .expect("the trusted lower source remains usable");
    session.shutdown().await.expect("clean shutdown");

    let wire = serde_json::to_string(&provider.requests()[0].messages).unwrap();
    assert!(wire.contains("USER_SKILL_BODY_MARKER"), "{wire}");
    assert!(!wire.contains("UNTRUSTED_WORKSPACE_BODY_MARKER"), "{wire}");
}

#[tokio::test]
async fn smith_memory_is_relevant_sensitive_manifested_and_not_canonical_history() {
    const MEMORY: &str = "MEMORY_CONTEXT_MARKER: prefer deterministic fixtures.";

    let fixture = Fixture::new(FAKE_CONFIG);
    let provider = Arc::new(FakeProvider::text_reply("understood"));
    let source = Arc::new(
        SmithMemorySource::new(vec![
            SmithMemoryRecord::new(
                "test-preference",
                MEMORY,
                agent_runtime::context::Sensitivity::Sensitive,
            )
            .with_priority(1)
            .with_keywords(["deterministic", "fixtures"]),
        ])
        .unwrap(),
    );
    let mut runtime = request(&fixture, HostSurface::Headless);
    runtime.provider = Some(provider.clone());
    runtime.memory = Some(source);
    let smith = factory::build_request(runtime).await.expect("a runtime");
    assert!(smith.policy().memory_revision.is_some());
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text(
            "Explain how the deterministic fixtures should be verified.",
        ))
        .await
        .expect("the memory-backed turn runs");

    let wire = serde_json::to_string(&provider.requests()[0].messages).unwrap();
    assert!(wire.contains("MEMORY_CONTEXT_MARKER"), "{wire}");
    let snapshot = session.snapshot();
    assert!(
        snapshot
            .history
            .iter()
            .all(|message| !message.joined_text().contains("MEMORY_CONTEXT_MARKER")),
        "memory was copied into canonical conversation history"
    );
    let segment = snapshot
        .manifests
        .last()
        .expect("turn manifest")
        .manifest
        .segments
        .iter()
        .find(|segment| segment.id.as_str() == "harness:memory:smith:test-preference")
        .expect("memory segment");
    assert_eq!(
        segment.sensitivity,
        agent_runtime_core::manifest::SegmentSensitivity::Sensitive
    );
    assert!(segment.tokens > 0);
    session.shutdown().await.expect("clean shutdown");
}

/// Writes `<root>/skills/<name>/SKILL.md`.
fn write_discovered_skill(root: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let directory = root.join("skills").join(name);
    std::fs::create_dir_all(&directory).expect("a skill directory");
    let path = directory.join("SKILL.md");
    std::fs::write(&path, body).expect("a skill body");
    path
}

const DISCOVERED_SKILL: &str =
    "---\ndescription: Review Rust implementation boundaries\n---\n\nDISCOVERED_BODY_MARKER\n";

#[tokio::test]
async fn tui_and_headless_discover_the_same_catalog() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let user_root = fixture.home.path().join(".smith");
    write_discovered_skill(&user_root, "rust-review", DISCOVERED_SKILL);
    write_discovered_skill(
        &fixture.project.path().join(".smith"),
        "deploy",
        DISCOVERED_SKILL,
    );
    let trust = smith_config::trust::TrustStore::open(fixture.home.path()).expect("a trust store");

    let mut indexes = Vec::new();
    for surface in [HostSurface::Terminal, HostSurface::Headless] {
        let mut runtime = request(&fixture, surface);
        runtime.provider = Some(Arc::new(FakeProvider::text_reply("unused")));
        let (skills, problems) = smith_runtime::skills::discover_into(
            std::mem::take(&mut runtime.skills),
            &user_root,
            fixture.project.path(),
            &trust,
        );
        assert!(problems.is_empty(), "{problems:?}");
        runtime.skills = skills;
        let smith = factory::build_request(runtime).await.expect("a runtime");
        indexes.push(
            smith
                .skill_index()
                .iter()
                .map(|entry| (entry.name().to_owned(), entry.layer, entry.activatable))
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(indexes[0], indexes[1]);
    assert!(
        indexes[0].iter().any(|(name, layer, activatable)| {
            name == "rust-review"
                && *layer == smith_runtime::skills::SmithSkillLayer::User
                && *activatable
        }),
        "{:?}",
        indexes[0]
    );
}

#[tokio::test]
async fn a_malformed_skill_does_not_stop_a_session_starting() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let user_root = fixture.home.path().join(".smith");
    write_discovered_skill(&user_root, "rust-review", DISCOVERED_SKILL);
    write_discovered_skill(&user_root, "half-written", "---\nname: half-written\n");
    let trust = smith_config::trust::TrustStore::open(fixture.home.path()).expect("a trust store");

    let mut runtime = request(&fixture, HostSurface::Terminal);
    runtime.provider = Some(Arc::new(FakeProvider::text_reply("unused")));
    let (skills, problems) = smith_runtime::skills::discover_into(
        std::mem::take(&mut runtime.skills),
        &user_root,
        fixture.project.path(),
        &trust,
    );
    runtime.skills = skills;
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].name, "half-written");

    let smith = factory::build_request(runtime)
        .await
        .expect("a broken skill file is not a reason to refuse to start");
    assert!(
        smith
            .policy()
            .skills
            .iter()
            .any(|name| name == "rust-review")
    );
    assert!(
        !smith
            .policy()
            .skills
            .iter()
            .any(|name| name == "half-written")
    );
}

#[tokio::test]
async fn headless_indexes_an_untrusted_project_skill_without_asking_anything() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let user_root = fixture.home.path().join(".smith");
    write_discovered_skill(
        &fixture.project.path().join(".smith"),
        "deploy",
        DISCOVERED_SKILL,
    );
    let trust = smith_config::trust::TrustStore::open(fixture.home.path()).expect("a trust store");

    let provider = Arc::new(FakeProvider::text_reply("done"));
    let mut runtime = request(&fixture, HostSurface::Headless);
    runtime.provider = Some(provider.clone());
    // No approval broker: an unattended run has no surface that could answer,
    // so a discovery path that asked would deadlock or fail the run.
    let (skills, problems) = smith_runtime::skills::discover_into(
        std::mem::take(&mut runtime.skills),
        &user_root,
        fixture.project.path(),
        &trust,
    );
    assert!(problems.is_empty(), "{problems:?}");
    runtime.skills = skills;

    let smith = factory::build_request(runtime).await.expect("a runtime");
    assert!(!smith.policy().skills.iter().any(|name| name == "deploy"));
    assert!(smith.skill_index().iter().any(|entry| {
        entry.name() == "deploy"
            && entry.layer == smith_runtime::skills::SmithSkillLayer::Workspace
            && !entry.activatable
    }));

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("Use the deploy skill."))
        .await
        .expect("the turn runs");
    session.shutdown().await.expect("clean shutdown");
    let wire = serde_json::to_string(&provider.requests()[0].messages).unwrap();
    assert!(!wire.contains("DISCOVERED_BODY_MARKER"), "{wire}");
}
