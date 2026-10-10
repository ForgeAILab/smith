use super::*;

use smith_client::commands::{CommandsAction, MenuRow, ParsedInput};
use smith_client::commands_report::{CommandsReport, render_plain};
use smith_client::file_commands::CommandState;
use smith_tui::app::{Overlay, SubmissionTarget};

use crate::local_command::file_commands::CommandContext;

struct CommandFixture {
    user: tempfile::TempDir,
    project: tempfile::TempDir,
    context: CommandContext,
}

impl CommandFixture {
    fn new() -> Self {
        let user = tempfile::tempdir().expect("user root");
        let project = tempfile::tempdir().expect("project root");
        let context = CommandContext::discover(user.path(), project.path()).expect("commands");
        Self {
            user,
            project,
            context,
        }
    }

    fn write(&self, project: bool, name: &str, body: &str) -> PathBuf {
        let root = if project {
            self.project.path().join(".smith")
        } else {
            self.user.path().to_path_buf()
        };
        std::fs::create_dir_all(root.join("commands")).expect("command directory");
        let path = root.join("commands").join(format!("{name}.md"));
        std::fs::write(&path, body).expect("command body");
        path
    }

    fn app(&self) -> App {
        let mut app = App::new("example-model", "project");
        app.set_command_catalog(self.context.catalog());
        app
    }
}

const AUDIT: &str = "---\ndescription: Review code for bugs\nargument-hint: [path]\n---\nReview $ARGUMENTS for bugs.\n";

fn enter() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

fn screen(app: &App) -> String {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 120)).expect("terminal");
    terminal
        .draw(|frame| {
            smith_tui::render::draw(
                frame,
                app,
                smith_tui::Theme::new().without_color().without_motion(),
            )
        })
        .expect("frame");
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn file_commands_slash_completion_and_palette_show_metadata_and_tab_only_completes() {
    let fixture = CommandFixture::new();
    fixture.write(false, "audit", AUDIT);
    fixture.context.reload().expect("reload");
    let mut app = fixture.app();
    for ch in "/aud".chars() {
        assert!(
            app.on_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .is_none()
        );
    }
    let visible = screen(&app);
    for value in ["/audit", "Review code for bugs", "user", "[path]"] {
        assert!(visible.contains(value), "{value}: {visible}");
    }
    assert!(
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .is_none()
    );
    assert_eq!(app.composer.text(), "/audit ");
    assert!(app.transcript.is_empty());
    assert!(app.overlay.is_none());

    app.composer.clear();
    app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(matches!(app.overlay, Some(Overlay::Palette { .. })));
    assert!(
        smith_client::commands::matches_with(app.composer.text(), &app.command_catalog)
            .iter()
            .any(|row| matches!(row, MenuRow::File(entry) if entry.command.name == "audit"))
    );
    app.composer.replace("/aud");
    assert!(
        matches!(app.on_key(enter()), Some(Action::FileCommand { typed, .. }) if typed == "/audit")
    );
    app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.composer.text(), "/audit");
}

#[test]
fn file_commands_enter_expands_at_the_host_and_keeps_typed_history() {
    let fixture = CommandFixture::new();
    fixture.write(false, "audit", AUDIT);
    fixture.context.reload().expect("reload");
    let mut app = fixture.app();
    app.composer.replace("/audit src/lib.rs");
    let Some(Action::FileCommand {
        typed,
        name,
        arguments,
        queue: false,
    }) = app.on_key(enter())
    else {
        panic!("host preparation action");
    };
    assert_eq!(typed, "/audit src/lib.rs");
    let submission = fixture
        .context
        .prepare(&app, typed, &name, &arguments)
        .expect("prepared prompt");
    assert_eq!(submission.display_text(), "/audit src/lib.rs");
    assert_eq!(submission.committed_text(), "Review src/lib.rs for bugs.");
    assert_eq!(
        submission.input_without_files(),
        UserInput::text("Review src/lib.rs for bugs.")
    );
    assert!(matches!(
        app.submit_prepared(submission),
        Action::Submit {
            target: SubmissionTarget::WholeTurn,
            ..
        }
    ));
    app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.composer.text(), "/audit src/lib.rs");
}

#[test]
fn file_commands_untrusted_invocation_is_a_local_refusal_without_a_submission() {
    let fixture = CommandFixture::new();
    fixture.write(true, "deploy", "Deploy $ARGUMENTS");
    fixture.context.reload().expect("reload");
    let mut app = fixture.app();
    app.composer.replace("/dep");
    app.open_overlay(Overlay::Palette {
        selected: 0,
        error: None,
        restore_on_escape: None,
    });
    assert!(screen(&app).contains("/commands trust deploy"));
    app.composer.replace("/deploy staging");
    let Some(Action::FileCommand {
        typed,
        name,
        arguments,
        ..
    }) = app.on_key(enter())
    else {
        panic!("only a host action, never a submission");
    };
    let refusal = fixture
        .context
        .prepare(&app, typed, &name, &arguments)
        .expect_err("withheld");
    app.transcript.push_error(refusal.clone());
    assert!(refusal.contains("/commands trust deploy"), "{refusal}");
    assert!(!app.has_pending_input());
    assert!(
        app.transcript
            .blocks()
            .iter()
            .all(|block| !matches!(block, Block::User { .. }))
    );
}

#[test]
fn file_commands_listing_includes_layers_states_shadowing_and_every_problem() {
    let fixture = CommandFixture::new();
    fixture.write(false, "audit", AUDIT);
    fixture.write(true, "audit", "Project audit");
    fixture.write(true, "deploy", "Deploy");
    fixture.write(false, "broken", "---\n");
    fixture.context.reload().expect("reload");
    let (_, digest) = fixture.context.confirmation("audit").expect("confirmation");
    fixture.context.trust("audit", &digest).expect("trust");
    let mut app = fixture.app();
    let CommandReport::Show(LocalResult::Commands(report)) =
        local_command::file_commands::command(&fixture.context, &mut app, CommandsAction::List)
    else {
        panic!("listing report");
    };
    let report = render_plain(&report);
    for value in [
        "project\n",
        "user\n",
        "/audit · runnable",
        "shadowed by a trusted project command",
        "/commands trust deploy",
        "not loaded",
        "broken",
        "frontmatter",
    ] {
        assert!(report.contains(value), "{value}: {report}");
    }
    assert_eq!(report.matches("/audit ·").count(), 2);
    assert!(
        render_plain(&CommandsReport::from_catalog(
            &CommandFixture::new().context.catalog()
        ))
        .contains("no file commands were found on disk")
    );
}

#[test]
fn file_commands_shared_confirmation_admits_exact_content_in_the_same_session() {
    let fixture = CommandFixture::new();
    let path = fixture.write(true, "deploy", "Deploy $ARGUMENTS");
    fixture.context.reload().expect("reload");
    let mut app = fixture.app();
    let report = local_command::file_commands::command(
        &fixture.context,
        &mut app,
        CommandsAction::Trust("deploy".to_owned()),
    );
    let CommandReport::CommandTrust {
        name,
        content,
        digest,
    } = report
    else {
        panic!("confirmation report");
    };
    assert!(content.contains(".smith/commands/deploy.md"), "{content}");
    assert!(content.contains(&format!("content {digest}")), "{content}");
    assert_eq!(
        fixture.context.catalog().resolve("deploy").unwrap().state,
        CommandState::NeedsTrust
    );
    app.confirm_command_trust(name, content, digest.clone());
    let Some(Overlay::Confirm(dialog)) = &app.overlay else {
        panic!("shared confirmation");
    };
    assert!(
        dialog
            .warning
            .as_ref()
            .unwrap()
            .0
            .contains("No action is selected by default")
    );
    let accepted = dialog.accept.as_ref();
    assert!(matches!(
        accepted,
        smith_tui::app::ConfirmOutcome::Action(Action::TrustCommand { .. })
    ));
    fixture
        .context
        .trust("deploy", &digest)
        .expect("record decision");
    app.set_command_catalog(fixture.context.catalog());
    let prompt = fixture
        .context
        .prepare(&app, "/deploy staging".to_owned(), "deploy", "staging")
        .expect("runs in same session");
    assert_eq!(prompt.committed_text(), "Deploy staging");
    assert!(
        fixture
            .context
            .confirmation("deploy")
            .unwrap_err()
            .contains("already trusted")
    );

    std::fs::write(&path, "Rewritten deploy").expect("later commit");
    let refusal = fixture
        .context
        .prepare(&app, "/deploy".to_owned(), "deploy", "")
        .expect_err("changed");
    assert!(
        refusal.contains("content changed") && refusal.contains("/commands trust deploy"),
        "{refusal}"
    );
    let (_, digest) = fixture
        .context
        .confirmation("deploy")
        .expect("re-approvable");
    fixture
        .context
        .trust("deploy", &digest)
        .expect("new decision");
    assert_eq!(
        fixture
            .context
            .prepare(&app, "/deploy".to_owned(), "deploy", "")
            .unwrap()
            .committed_text(),
        "Rewritten deploy"
    );
}

#[test]
fn file_commands_confirmation_cannot_admit_content_rewritten_while_visible() {
    let fixture = CommandFixture::new();
    let path = fixture.write(true, "deploy", "Deploy");
    fixture.context.reload().expect("reload");
    let (_, digest) = fixture
        .context
        .confirmation("deploy")
        .expect("confirmation");
    std::fs::write(path, "Different content").expect("edit");
    let error = fixture
        .context
        .trust("deploy", &digest)
        .expect_err("stale confirmation");
    assert!(error.contains("changed during confirmation"), "{error}");
    assert!(
        fixture
            .context
            .prepare(&fixture.app(), "/deploy".into(), "deploy", "")
            .is_err()
    );
}

#[test]
fn file_commands_trust_refuses_user_and_unknown_names() {
    let fixture = CommandFixture::new();
    fixture.write(false, "audit", AUDIT);
    fixture.context.reload().expect("reload");
    assert!(
        fixture
            .context
            .confirmation("audit")
            .unwrap_err()
            .contains("user command")
    );
    assert!(
        fixture
            .context
            .confirmation("missing")
            .unwrap_err()
            .contains("not a discovered project command")
    );
}

#[test]
fn file_commands_reload_refreshes_the_app_catalog_and_help_group() {
    let fixture = CommandFixture::new();
    let mut app = fixture.app();
    assert_eq!(
        smith_client::help_report::render_plain(&smith_client::commands::help_with(
            &app.command_catalog
        )),
        smith_client::help_report::render_plain(&smith_client::commands::help())
    );
    fixture.write(false, "audit", AUDIT);
    assert!(!smith_client::commands::has_exact_name_with(
        "/audit",
        &app.command_catalog
    ));
    let CommandReport::Show(LocalResult::Commands(report)) =
        local_command::file_commands::command(&fixture.context, &mut app, CommandsAction::Reload)
    else {
        panic!("reload report");
    };
    assert!(
        matches!(report.as_ref(), CommandsReport::Reloaded { runnable: 1, entries: 1, problems } if problems.is_empty())
    );
    assert!(smith_client::commands::has_exact_name_with(
        "/audit",
        &app.command_catalog
    ));
    app.composer.replace("/help");
    assert!(app.on_key(enter()).is_none());
    let help = app
        .transcript
        .blocks()
        .iter()
        .find_map(|block| match block {
            Block::Local(LocalResult::Help(report)) => Some(report),
            _ => None,
        })
        .expect("help report");
    let help = smith_client::help_report::render_plain(help);
    assert!(
        help.contains("File commands\n/audit [path] — user · Review code for bugs"),
        "{help}"
    );
    assert!(help.find("Advanced").unwrap() < help.find("File commands").unwrap());
    assert!(screen(&app).contains("File commands"));
    fixture.write(
        false,
        "audit",
        "---\ndescription: Edited description\nargument-hint: [files]\n---\nChanged",
    );
    local_command::file_commands::command(&fixture.context, &mut app, CommandsAction::Reload);
    let entry = app.command_catalog.resolve("audit").unwrap();
    assert_eq!(entry.command.description, "Edited description");
    assert_eq!(entry.command.argument_hint.as_deref(), Some("[files]"));
}

#[tokio::test]
async fn file_commands_user_edits_apply_without_reload_and_references_materialize_normally() {
    let fixture = CommandFixture::new();
    let path = fixture.write(false, "audit", AUDIT);
    fixture.context.reload().expect("reload");
    let mut app = fixture.app();
    std::fs::write(path, "Inspect $ARGUMENTS").expect("edit user command");
    app.set_resources(RuntimeResources {
        files: vec![ResourceEntry::new(
            "file:src/lib.rs",
            "src/lib.rs",
            "workspace file",
        )],
        ..RuntimeResources::default()
    });
    std::fs::create_dir_all(fixture.project.path().join("src")).expect("src");
    std::fs::write(
        fixture.project.path().join("src/lib.rs"),
        "pub fn answer() {}\n",
    )
    .expect("source");
    let submission = fixture
        .context
        .prepare(&app, "/audit @src/lib.rs".into(), "audit", "@src/lib.rs")
        .expect("references");
    assert_eq!(submission.display_text(), "/audit @src/lib.rs");
    assert_eq!(submission.committed_text(), "Inspect @src/lib.rs");
    assert_eq!(submission.files(), &["src/lib.rs"]);
    let input = materialize_prepared_submission(fixture.project.path(), &submission)
        .await
        .expect("ordinary materialization");
    assert_eq!(input.parts.len(), 2);
}

#[test]
fn file_commands_busy_queue_keeps_typed_preview_and_enter_uses_steering() {
    let fixture = CommandFixture::new();
    fixture.write(false, "audit", AUDIT);
    fixture.context.reload().expect("reload");
    let mut app = fixture.app();
    app.status.activity = smith_tui::status::Activity::Working;
    app.composer.replace("/audit src/lib.rs");
    let Some(Action::FileCommand {
        typed,
        name,
        arguments,
        queue: true,
    }) = app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
    else {
        panic!("queue through the host");
    };
    let submission = fixture
        .context
        .prepare(&app, typed, &name, &arguments)
        .expect("prompt");
    app.queue_prepared(submission);
    assert_eq!(
        app.pending_input_previews()[0].entries,
        ["/audit src/lib.rs"]
    );
    let submission = fixture
        .context
        .prepare(&app, "/audit next".into(), "audit", "next")
        .expect("prompt");
    assert!(matches!(
        app.submit_prepared(submission),
        Action::Submit {
            target: SubmissionTarget::Steer { .. },
            ..
        }
    ));
}

#[test]
fn file_commands_reserved_names_and_literal_slash_keep_their_existing_routes() {
    let fixture = CommandFixture::new();
    fixture.write(false, "model", "This must never replace the built-in");
    fixture.write(false, "audit", AUDIT);
    fixture.context.reload().expect("reload");
    assert!(matches!(
        smith_client::commands::parse_with("/model", &fixture.context.catalog()),
        Ok(ParsedInput::BuiltIn(_))
    ));
    assert!(
        fixture
            .context
            .catalog()
            .problems()
            .iter()
            .any(|problem| problem.name == "model")
    );
    let mut app = fixture.app();
    for ch in "//audit src/lib.rs".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    let Some(Action::Submit { submission, .. }) = app.on_key(enter()) else {
        panic!("literal input");
    };
    assert_eq!(submission.committed_text(), "/audit src/lib.rs");
}

#[tokio::test]
async fn file_commands_production_loop_sends_expansion_and_never_sends_withheld_commands() {
    use crate::tui_driver::{InteractiveExit, InteractiveResources, run_scripted_tui};
    use agent_runtime::provider::fake::{FakeProvider, ScriptedStream};
    use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
    use crossterm::event::Event;
    use futures_util::StreamExt;

    for project_command in [false, true] {
        let fixture = CommandFixture::new();
        fixture.write(project_command, "audit", AUDIT);
        fixture.context.reload().expect("reload");
        std::fs::create_dir_all(fixture.project.path().join(".smith")).expect("config directory");
        std::fs::write(
            fixture.project.path().join(".smith/config.toml"),
            LOCAL_COMMAND_CONFIG,
        )
        .expect("config");
        let resolution = resolve(
            &ResolveRequest::new(fixture.project.path()).with_home_dir(fixture.user.path()),
        )
        .expect("resolution");
        let catalog = Arc::new(
            serde_json::from_str::<smith_config::catalog::CatalogSnapshot>(
                smith_runtime::model_catalog::EMBEDDED_MODELS_DEV_SEED,
            )
            .expect("model catalog"),
        );
        let inventory =
            local_inventory_with_catalog(&resolution, AVAILABLE_ADAPTER_KINDS, Some(&catalog))
                .expect("inventory");
        let agents = resolution.config.agent.clone();
        let (sources, skills) = crate::skills::SkillContext::compose(
            smith_runtime::skills::SmithSkillSources::new(),
            fixture.user.path(),
            fixture.project.path(),
        )
        .expect("skills");
        let provider = Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "checked".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])],
        ));
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(fixture.project.path()).expect("workspace"),
            )),
            approval: Some(Arc::new(agent_runtime_core::approval::DenyAll)),
            provider: Some(provider.clone()),
            skills: sources,
            ..RuntimeRequest::new(resolution.config, HostSurface::Terminal)
        };
        let host = Box::pin(smith_runtime::host::start(
            HostSessionRequest::new(runtime, fixture.project.path())
                .checkpoint_keys(Arc::new(TestCheckpointKeys)),
        ))
        .await
        .expect("host");
        host.set_goal_continuation_enabled(false);
        let resources = InteractiveResources {
            inventory,
            agents,
            catalog,
            sessions: Vec::new(),
            credential_pool: None,
            mcp: None,
            skills: Arc::new(skills),
            commands: Arc::new(
                CommandContext::discover(fixture.user.path(), fixture.project.path())
                    .expect("commands"),
            ),
            capability_denials: Vec::new(),
        };
        let mut app = fixture.app();
        app.composer.replace("/audit src/lib.rs");
        let wait_provider = provider.clone();
        let delayed_quit = futures_util::stream::once(async move {
            if !project_command {
                tokio::time::timeout(Duration::from_secs(2), async {
                    while wait_provider.requests().is_empty() {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("ordinary dispatch reaches the fake provider");
            }
            Ok(Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
            )))
        });
        let mut keys = Box::pin(
            futures_util::stream::iter([Ok(Event::Key(enter()))])
                .chain(delayed_quit)
                .chain(futures_util::stream::iter([Ok(Event::Key(KeyEvent::new(
                    KeyCode::Char('c'),
                    KeyModifiers::CONTROL,
                )))])),
        );
        let (exit, app) = Box::pin(run_scripted_tui(
            &host,
            fixture.project.path(),
            &resources,
            app,
            &mut keys,
            false,
        ))
        .await
        .expect("production loop");
        assert!(matches!(exit, InteractiveExit::Quit(..)));
        if project_command {
            assert!(
                provider.requests().is_empty(),
                "withheld commands never reach the provider"
            );
            assert!(host.session().history().is_empty());
            assert!(app.transcript.blocks().iter().any(|block| matches!(block,
                Block::Error { message } if message.contains("/commands trust audit"))));
        } else {
            let requests = provider.requests();
            assert_eq!(requests.len(), 1);
            let sent = serde_json::to_string(&requests[0].messages).expect("request");
            assert!(sent.contains("Review src/lib.rs for bugs."), "{sent}");
            assert!(!sent.contains("/audit src/lib.rs"), "{sent}");
            assert!(app.transcript.blocks().iter().any(|block| matches!(block,
                Block::User { text } if text == "Review src/lib.rs for bugs.")));
        }
        Box::pin(host.shutdown()).await.expect("shutdown");
    }
}
