use super::*;

#[tokio::test]
async fn explicit_skill_activation_precedes_the_first_request_and_survives_resume() {
    use smith_runtime::session::{FileSessionStore, ProjectId, SessionPaths};
    let fixture = Fixture::new(FAKE_CONFIG);
    let paths = SessionPaths::new(fixture.home.path(), &ProjectId::new("skill-test").unwrap());
    let store = Arc::new(FileSessionStore::new(paths));
    let provider = Arc::new(FakeProvider::text_reply("done"));
    let sources = SmithSkillSources::new()
        .with_user(Skill::inline(
            "inspection",
            "Examine boundaries",
            "EXPLICIT_SKILL_BODY_MARKER",
        ))
        .with_workspace(
            Skill::inline(
                "inspection",
                "Untrusted replacement",
                "WITHHELD_BODY_MARKER",
            ),
            TrustStatus::Untrusted,
        );
    let build = |selected| RuntimeRequest {
        provider: Some(provider.clone()),
        skills: sources.clone(),
        requested_skills: selected,
        session_store: Some(store.clone()),
        ..request(&fixture, HostSurface::Headless)
    };
    let original = factory::build_request(build(Vec::new())).await.unwrap();
    let session = original
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let id = session.id().clone();
    let (selected, instructions) = original
        .prepare_skill_invocation(&session, "inspection")
        .unwrap();
    assert_eq!(instructions, "EXPLICIT_SKILL_BODY_MARKER");
    assert_eq!(selected.layer, smith_runtime::skills::SmithSkillLayer::User);
    assert!(provider.requests().is_empty());
    session.shutdown().await.unwrap();

    let activated = factory::build_request(build(vec![selected.clone()]))
        .await
        .unwrap();
    let session = activated
        .runtime()
        .start_session(StartSession::resume(id.clone()))
        .await
        .unwrap();
    assert_eq!(session.id(), &id);
    assert!(provider.requests().is_empty());
    assert!(
        session
            .activation_epoch()
            .unwrap()
            .activated()
            .iter()
            .any(|(id, _)| id.qualified() == "skill:inspection")
    );
    assert_eq!(
        activated
            .prepare_skill_activation(&session, "inspection")
            .unwrap(),
        selected
    );
    session.run(UserInput::text("Hello.")).await.unwrap();
    let wire = serde_json::to_string(&provider.requests()[0].messages).unwrap();
    assert_eq!(wire.matches("EXPLICIT_SKILL_BODY_MARKER").count(), 1);
    assert!(!wire.contains("WITHHELD_BODY_MARKER"));
    assert!(
        !session
            .history()
            .iter()
            .any(|message| message.joined_text().contains("EXPLICIT_SKILL_BODY_MARKER"))
    );
    session.shutdown().await.unwrap();

    let resumed = factory::build_request(build(Vec::new())).await.unwrap();
    let session = resumed
        .runtime()
        .start_session(StartSession::resume(id.clone()))
        .await
        .unwrap();
    assert!(
        session
            .activation_epoch()
            .unwrap()
            .activated()
            .iter()
            .any(|(id, _)| id.qualified() == "skill:inspection")
    );
    session.shutdown().await.unwrap();

    let mut narrowed = build(Vec::new());
    narrowed.capability_denials = vec!["skill:inspection".to_owned()];
    let narrowed = factory::build_request(narrowed).await.unwrap();
    let session = narrowed
        .runtime()
        .start_session(StartSession::resume(id))
        .await
        .unwrap();
    assert!(
        !session
            .activation_epoch()
            .unwrap()
            .activated()
            .iter()
            .any(|(id, _)| id.qualified() == "skill:inspection")
    );
    assert!(
        narrowed
            .prepare_skill_activation(&session, "inspection")
            .is_err()
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn explicit_skill_refusals_leave_the_current_session_usable() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let path = fixture.project.path().join("skill.md");
    let body = b"PINNED_SKILL_BODY_MARKER";
    std::fs::write(&path, body).unwrap();
    let provider = Arc::new(FakeProvider::text_reply("still usable"));
    let sources = SmithSkillSources::new()
        .with_user(Skill::from_verified_file(
            "inspection",
            "Examine boundaries",
            &path,
            Sha256::digest(body).into(),
        ))
        .with_user(Skill::inline(
            "large",
            "Large manual guide",
            "x".repeat(200_000),
        ))
        .with_workspace(
            Skill::inline("withheld", "Needs trust", "WITHHELD_BODY_MARKER"),
            TrustStatus::Untrusted,
        );
    let mut input = request(&fixture, HostSurface::Headless);
    input.provider = Some(provider.clone());
    input.skills = sources;
    let smith = factory::build_request(input).await.unwrap();
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let selected = smith
        .prepare_skill_activation(&session, "inspection")
        .unwrap();
    for name in ["unknown", "withheld", "large"] {
        let error = smith.prepare_skill_invocation(&session, name).unwrap_err();
        assert!(error.to_string().contains(name), "{error}");
    }
    std::fs::write(&path, "CHANGED_BODY_MARKER").unwrap();
    assert!(
        smith
            .prepare_skill_invocation(&session, "inspection")
            .is_err()
    );
    assert!(provider.requests().is_empty());
    let mut changed = request(&fixture, HostSurface::Headless);
    changed.skills = SmithSkillSources::new().with_user(Skill::from_verified_file(
        "inspection",
        "Examine boundaries",
        &path,
        Sha256::digest(b"CHANGED_BODY_MARKER").into(),
    ));
    changed.requested_skills = vec![selected];
    assert!(
        factory::build_request(changed)
            .await
            .unwrap_err()
            .to_string()
            .contains("changed since selection")
    );
    session.run(UserInput::text("Hello.")).await.unwrap();
    assert_eq!(provider.requests().len(), 1);
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn explicit_skill_requests_cannot_bypass_profile_denials() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let sources = SmithSkillSources::new().with_user(Skill::inline(
        "inspection",
        "Examine boundaries",
        "Skill instructions",
    ));
    let mut input = request(&fixture, HostSurface::Headless);
    input.skills = sources.clone();
    let smith = factory::build_request(input).await.unwrap();
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let selected = smith
        .prepare_skill_activation(&session, "inspection")
        .unwrap();
    for profile_denial in [false, true] {
        let mut denied = request(&fixture, HostSurface::Headless);
        denied.skills = sources.clone();
        denied.requested_skills = vec![selected.clone()];
        if profile_denial {
            denied.config.agent.profile.capabilities.deny =
                Some(smith_config::resolve::Sourced::new(
                    vec!["skill:*".to_owned()],
                    smith_config::resolve::Source::built_in("test"),
                ));
        } else {
            denied.capability_denials = vec!["skill:*".to_owned()];
        }
        assert!(
            factory::build_request(denied)
                .await
                .unwrap_err()
                .to_string()
                .contains("denied")
        );
    }
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn explicit_skill_pins_still_enforce_the_runtime_instruction_budget() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let sources = SmithSkillSources::new().with_user(Skill::inline(
        "large",
        "Large manual guide",
        "x".repeat(200_000),
    ));
    let mut input = request(&fixture, HostSurface::Headless);
    input.skills = sources.clone();
    let smith = factory::build_request(input).await.unwrap();
    let entry = &smith.skill_index()[0];
    let selected = smith_runtime::skills::RequestedSkill {
        name: entry.name().to_owned(),
        layer: entry.layer,
        revision: entry.descriptor.content_revision().as_str().to_owned(),
    };
    let mut input = request(&fixture, HostSurface::Headless);
    input.skills = sources;
    input.requested_skills = vec![selected];
    let result = match factory::build_request(input).await {
        Err(error) => error.to_string(),
        Ok(smith) => smith
            .runtime()
            .start_session(StartSession::new())
            .await
            .unwrap_err()
            .to_string(),
    };
    assert!(result.contains("budget"), "{result}");
}

#[tokio::test]
async fn explicit_skill_activation_is_unavailable_on_installed_cli_backends() {
    use smith_config::resolve::{ResolvedHarness, Source, Sourced};
    let fixture = Fixture::new(FAKE_CONFIG);
    let mut input = request(&fixture, HostSurface::Headless);
    input.skills = SmithSkillSources::new().with_user(Skill::inline(
        "inspection",
        "Examine boundaries",
        "Skill instructions",
    ));
    input.config.harness = Some(ResolvedHarness {
        kind: Sourced::new("codex".to_owned(), Source::built_in("test")),
        model: Sourced::new("test-model".to_owned(), Source::built_in("test")),
        program: "codex".to_owned(),
        executable: Some(Sourced::new(
            "/bin/true".to_owned(),
            Source::built_in("test"),
        )),
        args: Vec::new(),
        env: Default::default(),
        allow_own_tools: Sourced::new(false, Source::built_in("test")),
    });
    let smith = factory::build_request(input).await.unwrap();
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let error = smith
        .prepare_skill_activation(&session, "inspection")
        .unwrap_err()
        .to_string();
    assert!(error.contains("installed CLI"), "{error}");
    assert!(error.contains("MCP bridge"), "{error}");
    session.shutdown().await.unwrap();
}

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
        let (skills, problems, _) = smith_runtime::skills::discover_into(
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
async fn agents_skills_compose_with_smith_copies_without_duplicate_names() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let user_root = fixture.home.path().join(".smith");
    let smith_body = "---\ndescription: Smith winner\n---\n\nSMITH_WINNER_MARKER\n";
    for (root, name) in [
        (fixture.home.path(), "rust-review"),
        (fixture.project.path(), "deploy"),
    ] {
        write_discovered_skill(&root.join(".smith"), name, smith_body);
        write_discovered_skill(&root.join(".agents"), name, DISCOVERED_SKILL);
    }
    write_discovered_skill(
        &fixture.home.path().join(".agents"),
        "shared-user",
        DISCOVERED_SKILL,
    );
    write_discovered_skill(
        &fixture.project.path().join(".agents"),
        "shared-project",
        DISCOVERED_SKILL,
    );
    let trust = smith_config::trust::TrustStore::open(&user_root).expect("a trust store");
    let mut runtime = request(&fixture, HostSurface::Headless);
    runtime.provider = Some(Arc::new(FakeProvider::text_reply("done")));
    let (skills, problems, _) = smith_runtime::skills::discover_into(
        std::mem::take(&mut runtime.skills),
        &user_root,
        fixture.project.path(),
        &trust,
    );
    assert_eq!(problems.len(), 2);
    assert!(
        problems
            .iter()
            .all(|problem| problem.reason.contains("shadowed"))
    );
    runtime.skills = skills;
    let smith = factory::build_request(runtime)
        .await
        .expect("overlapping locations compose without a duplicate-name error");
    for (name, layer, activatable, description) in [
        (
            "rust-review",
            smith_runtime::skills::SmithSkillLayer::User,
            true,
            "Smith winner",
        ),
        (
            "deploy",
            smith_runtime::skills::SmithSkillLayer::Workspace,
            false,
            "Smith winner",
        ),
        (
            "shared-user",
            smith_runtime::skills::SmithSkillLayer::User,
            true,
            "Review Rust implementation boundaries",
        ),
        (
            "shared-project",
            smith_runtime::skills::SmithSkillLayer::Workspace,
            false,
            "Review Rust implementation boundaries",
        ),
    ] {
        let entries = smith
            .skill_index()
            .iter()
            .filter(|entry| entry.name() == name)
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1, "{name}");
        assert_eq!(entries[0].layer, layer);
        assert_eq!(entries[0].activatable, activatable);
        assert_eq!(entries[0].description(), description);
    }
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("Hello."))
        .await
        .expect("the composed session runs");
    session.shutdown().await.expect("clean shutdown");
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
    let (skills, problems, _) = smith_runtime::skills::discover_into(
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
    let (skills, problems, _) = smith_runtime::skills::discover_into(
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
