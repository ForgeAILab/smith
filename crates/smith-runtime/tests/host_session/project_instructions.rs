use super::*;

#[tokio::test]
async fn terminal_and_headless_runs_leave_agent_metadata_outside_the_project() {
    const INSTRUCTION_MARKER: &str = "HOST_DISCOVERED_AGENTS_MARKER";

    let home = tempfile::tempdir().expect("a user root");
    let project = tempfile::tempdir().expect("a project root");
    let user_config_dir = home.path().join(".smith");
    std::fs::create_dir_all(&user_config_dir).expect("a user config directory");
    std::fs::write(user_config_dir.join("config.toml"), CONFIG).expect("a user config");
    std::fs::write(project.path().join("tracked.txt"), "project content\n")
        .expect("project fixture");
    std::fs::write(project.path().join("AGENTS.md"), INSTRUCTION_MARKER)
        .expect("project instructions");
    let before = project_tree(project.path());
    let mut instruction_identities = Vec::new();

    for surface in [HostSurface::Terminal, HostSurface::Headless] {
        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved user configuration")
            .config;
        let provider = Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "done".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])],
        ));
        let runtime = RuntimeRequest {
            provider: Some(provider.clone() as Arc<dyn Provider>),
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("a project workspace"),
            )),
            approval: Some(Arc::new(DenyAll)),
            ..RuntimeRequest::new(config, surface)
        };
        let host = start(
            HostSessionRequest::new(runtime, project.path())
                .checkpoint_keys(test_checkpoint_keys()),
        )
        .await
        .expect("a hosted session");
        let identity = host
            .runtime()
            .policy()
            .project_instructions
            .clone()
            .expect("root AGENTS.md is composition evidence");
        assert_eq!(identity.source, "AGENTS.md");
        instruction_identities.push(identity);
        host.session()
            .run(UserInput::text("answer without changing the checkout"))
            .await
            .expect("the turn completes");
        let wire =
            serde_json::to_string(&provider.requests()[0].messages).expect("a provider request");
        assert!(wire.contains(INSTRUCTION_MARKER), "{wire}");
        assert!(
            host.session()
                .history()
                .iter()
                .all(|message| !message.joined_text().contains(INSTRUCTION_MARKER)),
            "project instructions became canonical conversation history"
        );
        host.shutdown().await.expect("clean shutdown");
    }

    assert_eq!(instruction_identities[0], instruction_identities[1]);

    assert_eq!(project_tree(project.path()), before);
    for forbidden in [".smith", ".omo", "sessions", "timeline", "children"] {
        assert!(
            !project.path().join(forbidden).exists(),
            "Smith created project-local agent metadata `{forbidden}`"
        );
    }
    assert!(
        home.path().join(".smith/sessions").is_dir(),
        "durable state was not redirected to user storage"
    );
}

#[tokio::test]
async fn a_hosted_runtime_freezes_project_instructions_until_reconstruction() {
    let fixture = Fixture::new();
    let instructions = fixture.project.path().join("AGENTS.md");
    std::fs::write(&instructions, "FROZEN_INSTRUCTIONS_A").expect("first instructions");

    let first_provider = Arc::new(FakeProvider::text_reply("first done"));
    let mut first_request = fixture.request(HostSurface::Headless);
    first_request.runtime.provider = Some(first_provider.clone() as Arc<dyn Provider>);
    let first = start(first_request).await.expect("first hosted runtime");
    let first_identity = first
        .runtime()
        .policy()
        .project_instructions
        .clone()
        .expect("first instruction identity");

    std::fs::write(&instructions, "FROZEN_INSTRUCTIONS_B").expect("changed instructions");
    first
        .session()
        .run(UserInput::text("use the frozen instructions"))
        .await
        .expect("first turn");
    let first_wire =
        serde_json::to_string(&first_provider.requests()[0].messages).expect("first request");
    assert!(first_wire.contains("FROZEN_INSTRUCTIONS_A"), "{first_wire}");
    assert!(
        !first_wire.contains("FROZEN_INSTRUCTIONS_B"),
        "{first_wire}"
    );
    let first_cache_identity = first
        .session()
        .snapshot()
        .manifests
        .last()
        .expect("first manifest")
        .manifest
        .cache_plan_fingerprint
        .clone();
    first.shutdown().await.expect("first shutdown");

    let second_provider = Arc::new(FakeProvider::text_reply("second done"));
    let mut second_request = fixture.request(HostSurface::Headless);
    second_request.runtime.provider = Some(second_provider.clone() as Arc<dyn Provider>);
    let second = start(second_request).await.expect("second hosted runtime");
    let second_identity = second
        .runtime()
        .policy()
        .project_instructions
        .clone()
        .expect("second instruction identity");
    assert_ne!(first_identity.revision, second_identity.revision);

    second
        .session()
        .run(UserInput::text("use the reconstructed instructions"))
        .await
        .expect("second turn");
    let second_wire =
        serde_json::to_string(&second_provider.requests()[0].messages).expect("second request");
    assert!(
        second_wire.contains("FROZEN_INSTRUCTIONS_B"),
        "{second_wire}"
    );
    assert!(
        !second_wire.contains("FROZEN_INSTRUCTIONS_A"),
        "{second_wire}"
    );
    let second_cache_identity = second
        .session()
        .snapshot()
        .manifests
        .last()
        .expect("second manifest")
        .manifest
        .cache_plan_fingerprint
        .clone();
    assert_ne!(first_cache_identity, second_cache_identity);
    second.shutdown().await.expect("second shutdown");
}

#[tokio::test]
async fn invalid_project_instructions_fail_before_runtime_or_session_state() {
    let fixture = Fixture::new();
    std::fs::write(fixture.project.path().join("AGENTS.md"), [0xff, 0xfe])
        .expect("invalid UTF-8 instructions");
    let provider = Arc::new(FakeProvider::text_reply("must not run"));
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone() as Arc<dyn Provider>);

    let error = start(request)
        .await
        .expect_err("invalid project instructions fail startup");
    assert!(error.to_string().contains("not valid UTF-8"), "{error}");
    assert!(provider.requests().is_empty(), "provider was contacted");
    assert!(
        !fixture.home.path().join(".smith/sessions").exists(),
        "invalid instructions created session state"
    );
}

fn project_tree(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn visit(root: &std::path::Path, current: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let mut entries = std::fs::read_dir(current)
            .expect("read project tree")
            .collect::<Result<Vec<_>, _>>()
            .expect("read project entry");
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            out.push(
                path.strip_prefix(root)
                    .expect("project-relative path")
                    .to_owned(),
            );
            if path.is_dir() {
                visit(root, &path, out);
            }
        }
    }

    let mut paths = Vec::new();
    visit(root, root, &mut paths);
    paths
}
