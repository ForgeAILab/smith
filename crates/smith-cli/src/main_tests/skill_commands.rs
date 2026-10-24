use super::*;

use agent_runtime::ability::Skill;
use agent_runtime::provider::fake::FakeProvider;
use agent_runtime_core::approval::AllowAll;
use crossterm::event::Event;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use smith_runtime::skills::SmithSkillSources;

use crate::tui_driver::{
    InteractiveExit, InteractiveResources, PresentationOptions, prepare_interactive_app,
    run_scripted_tui,
};

#[tokio::test]
async fn skill_shortcuts_submit_the_body_through_the_production_loop() {
    const BODY: &str = "MANUAL_SKILL_CONTEXT_MARKER\nKeep @file:guide.md and @@literal unchanged.";
    for (command, task_in_command, already_active) in [
        ("/inspect", false, false),
        ("/inspect", true, false),
        ("/skills use inspect", false, false),
        ("/skills use inspect", true, false),
        ("/inspect", false, true),
        ("/inspect", true, true),
        ("/skills use inspect", false, true),
        ("/skills use inspect", true, true),
    ] {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join(".smith")).unwrap();
        std::fs::write(
            project.path().join(".smith/config.toml"),
            LOCAL_COMMAND_CONFIG,
        )
        .unwrap();
        std::fs::write(project.path().join("notes.txt"), "NOTE_PAYLOAD_MARKER").unwrap();
        let resolution =
            resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path())).unwrap();
        let catalog = Arc::new(
            serde_json::from_str::<smith_config::catalog::CatalogSnapshot>(
                smith_runtime::model_catalog::EMBEDDED_MODELS_DEV_SEED,
            )
            .unwrap(),
        );
        let inventory =
            local_inventory_with_catalog(&resolution, AVAILABLE_ADAPTER_KINDS, Some(&catalog))
                .unwrap();
        let provider = Arc::new(FakeProvider::text_reply("checked"));
        let skill_path = home.path().join("SKILL.md");
        std::fs::write(&skill_path, BODY).unwrap();
        let (sources, skills) = crate::skills::SkillContext::compose(
            SmithSkillSources::new().with_user(Skill::from_verified_file(
                "inspect",
                "Review changed code",
                &skill_path,
                Sha256::digest(BODY.as_bytes()).into(),
            )),
            &home.path().join(".smith"),
            project.path(),
        )
        .unwrap();
        let request = |selected| RuntimeRequest {
            workspace: Some(Arc::new(ProjectWorkspace::new(project.path()).unwrap())),
            approval: Some(Arc::new(AllowAll)),
            provider: Some(provider.clone()),
            skills: sources.clone(),
            requested_skills: selected,
            ..RuntimeRequest::new(resolution.config.clone(), HostSurface::Terminal)
        };
        let host = Box::pin(smith_runtime::host::start(
            HostSessionRequest::new(request(Vec::new()), project.path())
                .checkpoint_keys(Arc::new(TestCheckpointKeys)),
        ))
        .await
        .unwrap();
        host.set_goal_continuation_enabled(false);
        let id = host.session().id().clone();
        let resources = InteractiveResources {
            inventory,
            agents: resolution.config.agent.clone(),
            catalog,
            sessions: Vec::new(),
            credential_pool: None,
            mcp: None,
            skills: Arc::new(skills),
            commands: Arc::new(
                local_command::file_commands::CommandContext::discover(
                    &home.path().join(".smith"),
                    project.path(),
                )
                .unwrap(),
            ),
            capability_denials: Vec::new(),
        };
        let presentation = PresentationOptions {
            no_color: true,
            no_motion: true,
            reasoning_notice: None,
            host_notice: None,
            mcp_trust_notices: Vec::new(),
            cache_miss_notices: false,
        };
        let mut app =
            prepare_interactive_app(&host, project.path(), &resources, &presentation, None)
                .await
                .app;
        let typed = if task_in_command {
            format!("{command} examine @file:notes.txt")
        } else {
            command.to_owned()
        };
        app.composer.replace(&typed);
        let mut keys = Box::pin(
            futures_util::stream::iter([Ok(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))])
            .chain(futures_util::stream::pending()),
        );
        let (exit, mut app) = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            Box::pin(run_scripted_tui(
                &host,
                project.path(),
                &resources,
                app,
                &mut keys,
                false,
            )),
        )
        .await
        .expect("skill selection should exit for activation")
        .unwrap();
        let InteractiveExit::ActivateSkill {
            selected,
            submission,
        } = exit
        else {
            panic!("skill activation exit")
        };
        assert!(provider.requests().is_empty());
        assert!(host.session().history().is_empty());
        assert_eq!(submission.display_text(), typed);
        let expected = if task_in_command {
            format!("{BODY}\n\nexamine @file:notes.txt")
        } else {
            BODY.to_owned()
        };
        assert_eq!(submission.committed_text(), expected);
        Box::pin(host.shutdown()).await.unwrap();

        let host = Box::pin(smith_runtime::host::start(
            HostSessionRequest::new(request(vec![selected]), project.path())
                .checkpoint_keys(Arc::new(TestCheckpointKeys))
                .resume(id.clone()),
        ))
        .await
        .unwrap();
        host.set_goal_continuation_enabled(false);
        assert_eq!(host.session().id(), &id);
        app.rebind_host();
        resources
            .commands
            .set_skills(smith_client::skill_commands::for_session(
                host.runtime(),
                host.session(),
            ));
        app.set_command_catalog(resources.commands.catalog());
        let initial = if already_active {
            app.composer.replace(&typed);
            vec![Ok(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))]
        } else {
            app.schedule_prepared(submission);
            Vec::new()
        };
        let waiting = provider.clone();
        let quit = futures_util::stream::once(async move {
            while waiting.requests().is_empty() {
                tokio::task::yield_now().await;
            }
            Ok(Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
            )))
        });
        let mut keys = Box::pin(futures_util::stream::iter(initial).chain(quit).chain(
            futures_util::stream::iter([Ok(Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
            )))]),
        ));
        let (exit, _) = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            Box::pin(run_scripted_tui(
                &host,
                project.path(),
                &resources,
                app,
                &mut keys,
                false,
            )),
        )
        .await
        .expect("skill invocation should dispatch a provider request")
        .unwrap();
        assert!(matches!(exit, InteractiveExit::Quit(..)));
        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        let wire = serde_json::to_string(&requests[0].messages).unwrap();
        assert!(wire.contains("MANUAL_SKILL_CONTEXT_MARKER"), "{wire}");
        assert!(
            wire.contains("Keep @file:guide.md and @@literal unchanged."),
            "{wire}"
        );
        assert!(
            requests[0].messages.iter().any(|message| {
                message.role == agent_runtime_core::content::Role::User
                    && message.joined_text().contains(&expected)
            }),
            "skill body must be submitted as user input: {wire}"
        );
        if task_in_command {
            assert!(wire.contains("NOTE_PAYLOAD_MARKER"), "{wire}");
        }
        assert!(!wire.contains("/inspect "));
        Box::pin(host.shutdown()).await.unwrap();
    }
}

#[test]
fn skill_shortcuts_render_in_existing_narrow_and_wide_completion_views() {
    for (width, height) in [(40, 12), (80, 24), (120, 32)] {
        for disabled_reason in [None, Some("awaiting trust")] {
            let mut app = App::new("example-model", "project");
            app.set_command_catalog(
                smith_client::file_commands::CommandCatalog::empty().with_skills(vec![
                    smith_client::skill_commands::SkillCommand {
                        name: "inspect".into(),
                        alias: "inspect".into(),
                        description: "Review changed code".into(),
                        source: "skill · user".into(),
                        disabled_reason: disabled_reason.map(str::to_owned),
                    },
                ]),
            );
            for ch in "/ins".chars() {
                app.on_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            }
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    smith_tui::render::draw(
                        frame,
                        &app,
                        smith_tui::Theme::new().without_color().without_motion(),
                    )
                })
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(text.contains("/inspect"), "{width}x{height}: {text}");
            if width >= 80 {
                assert!(text.contains("Review changed code"), "{text}");
                if disabled_reason.is_some() {
                    assert!(text.contains("awaiting trust"), "{text}");
                }
            }
            app.composer.replace("/zz-no-such-skill");
            terminal
                .draw(|frame| {
                    smith_tui::render::draw(
                        frame,
                        &app,
                        smith_tui::Theme::new().without_color().without_motion(),
                    )
                })
                .unwrap();
            let empty = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(empty.contains("no matching commands"), "{empty}");
            assert!(
                app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                    .is_none()
            );
            assert!(app.overlay.is_none());
        }
    }
}
