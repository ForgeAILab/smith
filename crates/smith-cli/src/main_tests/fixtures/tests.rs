use super::*;

#[test]
fn usage_fixture_builders_seed_one_user_turn() {
    for planned in [false, true] {
        let app = fixture_local_app(planned, true);
        let usage = app.status.session_usage();
        assert_eq!(usage.turns, 1);
        assert!(usage.render().expect("usage").starts_with("1 turn ·"));
    }
    assert_eq!(
        fixture_local_app(true, false).status.session_usage().turns,
        0
    );
}

#[tokio::test]
async fn fixtures_local_help() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[("help", "/help"), ("details-shown", "/details")])).await;
    let mut details = fixture_local_app(false, false);
    details.work_details = true;
    Box::pin(fixture.command("details-hidden", "/details", details)).await;
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_pickers() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("model-picker-empty", "/model"),
        ("model-missing", "/model missing"),
        ("provider-picker-empty", "/provider"),
        ("provider-missing", "/provider missing"),
        ("profile-picker-empty", "/profile"),
        ("profile-missing", "/profile missing"),
        ("resume-picker-empty", "/resume"),
        ("resume-missing", "/resume missing"),
        ("think-picker-empty", "/think"),
        ("think-unavailable", "/think on"),
        ("effort-picker-empty", "/effort"),
        ("effort-unavailable", "/effort high"),
    ]))
    .await;
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_status_diagnostics() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("status-fresh", "/status"),
        ("diagnostics-fresh", "/diagnostics"),
    ]))
    .await;
    for (name, command, reported) in [
        ("status-planned", "/status", false),
        ("diagnostics-planned", "/diagnostics", false),
        ("status-usage", "/status", true),
        ("diagnostics-usage", "/diagnostics", true),
    ] {
        Box::pin(fixture.command(name, command, fixture_local_app(true, reported))).await;
    }
    let mut priced = fixture_local_app(true, true);
    priced.status.set_price(Some(PriceReference {
        provider: "local".into(),
        model: "example-model".into(),
        table: PriceTable {
            input: Some(2_000_000),
            output: Some(8_000_000),
            cache_read: Some(200_000),
            cache_write: None,
        },
    }));
    Box::pin(fixture.command("status-priced", "/status", priced)).await;
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_context() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("context-fresh", "/context"),
        ("context-argument", "/context 256k"),
    ]))
    .await;
    for (name, reported) in [("context-planned", false), ("context-usage", true)] {
        Box::pin(fixture.command(name, "/context", fixture_local_app(true, reported))).await;
    }
    for (name, compacted) in [("context-exact", false), ("context-compacted", true)] {
        let mut app = fixture_local_app(true, false);
        let plan = app.status.context_plan.as_mut().expect("context plan");
        plan.confidence = EstimationConfidence::Exact;
        if compacted {
            plan.totals.insert("history".into(), 1_000);
            plan.totals.insert("summary".into(), 300);
            plan.segment_count = 4;
        }
        Box::pin(fixture.command(name, "/context", app)).await;
    }
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_goal() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("goal-empty", "/goal"),
        ("goal-edit-empty", "/goal edit revised objective"),
        ("goal-budget-empty", "/goal budget 100"),
        ("goal-pause-empty", "/goal pause"),
        ("goal-resume-empty", "/goal resume"),
        ("goal-clear-empty", "/goal clear"),
    ]))
    .await;
    Box::pin(fixture_local_goal_turn(&fixture, true)).await;
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_timeline() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[("timeline-empty", "/timeline")])).await;
    // Keep the original goal event history and usage-bearing turn: it is part
    // of the timeline-turn recording, including any journal-read error.
    Box::pin(fixture_local_goal_turn(&fixture, false)).await;
    Box::pin(fixture.commands(&[("timeline-turn", "/timeline")])).await;
    Box::pin(fixture.shutdown()).await;

    let fixture = Box::pin(fixture_local_populated()).await;
    fixture_local_git_setup(&fixture);
    Box::pin(fixture_local_git_edit(&fixture)).await;
    Box::pin(fixture_local_git_dirty(&fixture, FixtureGitGroup::Timeline)).await;
    Box::pin(fixture.commands(&[("timeline-recovery", "/timeline")])).await;
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_agent() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("agent-empty", "/agent"),
        ("agent-next-empty", "/agent next"),
        ("agent-previous-empty", "/agent previous"),
        ("agent-parent", "/agent parent"),
        ("agent-missing", "/agent missing"),
        ("agent-resume-missing", "/agent resume missing"),
    ]))
    .await;
    // Consume the root stream before spawning, so the child still receives
    // the original "child done" stream with the same counters and turn IDs.
    Box::pin(fixture_local_goal_turn(&fixture, false)).await;
    let coordinator = fixture
        .host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .expect("delegation");
    let spawned = Box::pin(coordinator.spawn(ChildSpec {
        task: UserInput::text("inspect the fixture"),
        model: ChildModelSelection::Inherit,
        limits: ChildLimits::turns(1),
        tools: ToolViewScope::ReadOnly,
        workspace: WorkspacePolicy::ReadOnlyView,
    }))
    .await
    .expect("spawn child");
    let agent_runtime::delegation::SpawnOutcome::Spawned { child, .. } = spawned else {
        panic!("fixture child was not spawned");
    };
    let child_status =
        tokio::time::timeout(Duration::from_secs(60), Box::pin(coordinator.wait(&child)))
            .await
            .expect("child watchdog")
            .expect("completed child");
    for (name, command) in [
        ("agent-populated", "/agent".to_owned()),
        ("agent-inspector", format!("/agent {child}")),
        ("agent-next", "/agent next".to_owned()),
        ("agent-previous", "/agent previous".to_owned()),
    ] {
        let mut app = fixture_local_app(false, false);
        app.children.insert(
            child.as_str().to_owned(),
            smith_tui::app::ChildSummary {
                state: smith_tui::app::ChildState::Completed,
                detail: child_status.last_result.clone(),
                profile: None,
            },
        );
        Box::pin(fixture.command(name, &command, app)).await;
    }
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_mcp_skills() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("mcp-empty", "/mcp"),
        ("mcp-trust-empty", "/mcp trust missing"),
        ("skills-empty", "/skills"),
        ("skills-trust-missing", "/skills trust missing"),
    ]))
    .await;
    Box::pin(fixture.shutdown()).await;

    let fixture = Box::pin(fixture_local_populated()).await;
    let mcp = mcp_context(fixture.home.path(), fixture.project.path());
    for (name, command) in [
        ("mcp-declared", "/mcp"),
        ("mcp-trust", "/mcp trust docs"),
        ("mcp-trust-missing", "/mcp trust missing"),
        ("skills-populated", "/skills"),
        ("skills-trust", "/skills trust deploy"),
    ] {
        Box::pin(fixture_command(
            name,
            command,
            fixture_local_app(false, false),
            &fixture.host,
            fixture.project.path(),
            Some(&mcp),
            &fixture.skills,
        ))
        .await;
    }
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_accounts_connections() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("account-picker-empty", "/account"),
        ("account-invalid", "/account nope"),
        ("account-missing", "/account 2"),
        ("connect-picker-empty", "/connect"),
        ("connect-missing", "/connect missing"),
        ("disconnect-picker-empty", "/disconnect"),
        ("disconnect-missing", "/disconnect missing"),
    ]))
    .await;
    Box::pin(fixture.shutdown()).await;

    let fixture = Box::pin(fixture_local_populated()).await;
    let mut accounts = fixture_local_app(false, false);
    accounts.set_accounts(vec![
        smith_tui::picker::ResourceEntry::new("0", "1", "env:FIRST · 25% used")
            .description("25% used")
            .active(true),
        smith_tui::picker::ResourceEntry::new("1", "2", "env:SECOND · 75% used")
            .description("75% used"),
    ]);
    Box::pin(fixture.command("account-picker", "/account", accounts)).await;
    let mut active = fixture_local_app(false, false);
    active.set_accounts(vec![
        smith_tui::picker::ResourceEntry::new("0", "1", "env:FIRST").active(true),
    ]);
    Box::pin(fixture.command("account-active", "/account 1", active)).await;
    for (name, command, connected) in [
        ("connect-picker", "/connect", false),
        ("disconnect-picker", "/disconnect", true),
    ] {
        let mut app = fixture_local_app(false, false);
        let entry =
            smith_tui::picker::ResourceEntry::new("local", "Local provider", "configured fixture");
        if connected {
            app.resources.disconnections.push(entry);
        } else {
            app.resources.connections.push(entry);
        }
        Box::pin(fixture.command(name, command, app)).await;
    }
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_diff_review() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("diff-no-git", "/diff"),
        ("diff-last-turn-empty", "/diff last-turn"),
        ("review-no-git", "/review"),
    ]))
    .await;
    Box::pin(fixture.shutdown()).await;

    let fixture = Box::pin(fixture_local_populated()).await;
    fixture_local_git_setup(&fixture);
    Box::pin(fixture.commands(&[("diff-clean", "/diff"), ("review-clean", "/review")])).await;
    Box::pin(fixture_local_git_edit(&fixture)).await;
    Box::pin(fixture_local_git_dirty(
        &fixture,
        FixtureGitGroup::DiffReview,
    ))
    .await;
    fixture
        .host
        .changes()
        .undo_latest()
        .expect("undo exact edit");
    Box::pin(fixture.replay("/undo")).await;
    Box::pin(fixture.replay("/redo")).await;
    fixture
        .host
        .changes()
        .redo_latest()
        .expect("redo exact edit");
    git(fixture.project.path(), &["add", "tracked.txt"]);
    Box::pin(fixture.commands(&[("diff-staged", "/diff staged")])).await;
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_undo_redo_revert() {
    let fixture = Box::pin(fixture_local_base()).await;
    Box::pin(fixture.commands(&[
        ("undo-empty", "/undo"),
        ("redo-empty", "/redo"),
        ("revert-no-scope", "/revert"),
        ("revert-no-git", "/revert tracked.txt"),
    ]))
    .await;
    Box::pin(fixture.shutdown()).await;

    let fixture = Box::pin(fixture_local_populated()).await;
    fixture_local_git_setup(&fixture);
    Box::pin(fixture_local_git_edit(&fixture)).await;
    Box::pin(fixture_local_git_dirty(&fixture, FixtureGitGroup::Recovery)).await;
    fixture
        .host
        .changes()
        .undo_latest()
        .expect("undo exact edit");
    Box::pin(fixture.commands(&[("undo-already-undone", "/undo"), ("redo-preview", "/redo")]))
        .await;
    fixture
        .host
        .changes()
        .redo_latest()
        .expect("redo exact edit");
    Box::pin(fixture.shutdown()).await;
}

#[tokio::test]
async fn fixtures_local_ephemeral() {
    use agent_runtime::provider::fake::FakeProvider;
    let (home, project) = mcp_project("");
    // The host rejects persistence policy from either project config layer.
    // ResolveRequest::with_home_dir reads this isolated user's .smith/config.toml.
    std::fs::write(
        home.path().join(".smith/config.toml"),
        "[persistence]\nenabled = false\n",
    )
    .expect("ephemeral user config");
    let ephemeral_config = LOCAL_COMMAND_CONFIG.replace(
        "model = \"example-model\"\n",
        "model = \"example-model\"\ndelegation = false\n",
    );
    std::fs::write(project.path().join(".smith/config.toml"), ephemeral_config)
        .expect("ephemeral project config");
    let (sources, skills) = crate::skills::SkillContext::compose(
        smith_runtime::skills::SmithSkillSources::new(),
        &home.path().join(".smith"),
        project.path(),
    )
    .expect("ephemeral skills");
    let host = Box::pin(fixture_local_host(
        home.path(),
        project.path(),
        Arc::new(FakeProvider::text_reply("unused")),
        sources,
    ))
    .await;
    assert!(host.paths().is_none(), "fixture host must be ephemeral");
    assert!(
        !host.runtime().policy().agent_delegation,
        "fixture delegation must be disabled"
    );
    let fixture = FixtureLocal {
        home,
        project,
        host,
        skills,
    };
    Box::pin(fixture.direct_commands(&[
        ("agent-unavailable", "/agent"),
        ("goal-ephemeral", "/goal"),
        ("goal-create-ephemeral", "/goal an unavailable goal"),
    ]))
    .await;
    Box::pin(fixture.shutdown()).await;
}

#[test]
fn fixture_feedback_capture_keeps_the_kind_and_hint_placement() {
    let mut app = App::new("model", "project");
    app.push_notice(
        smith_client::NoticeKind::AccountUnchanged,
        "already using that account",
    );
    let (raw, view) = fixture_raw_and_view(&app, &mut fixture_support::Normalizer::default());
    assert_eq!(
        raw,
        "title: account\nstate: Feedback\nbody:\nalready using that account\n"
    );
    assert!(view.transcript.is_empty());
    assert_eq!(view.feedback_notice(), app.feedback_notice());
    let screen = fixture_screen(&view, 100);
    assert!(
        screen
            .lines()
            .last()
            .unwrap()
            .contains("already using that account"),
        "{screen}"
    );
}
