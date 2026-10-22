use super::*;

use agent_runtime::ability::Skill;
use agent_runtime::provider::fake::FakeProvider;
use agent_runtime_core::approval::AllowAll;
use crossterm::event::Event;
use futures_util::StreamExt;
use smith_runtime::skills::SmithSkillSources;

use crate::tui_driver::{
    InteractiveExit, InteractiveResources, PresentationOptions, prepare_interactive_app,
    run_scripted_tui,
};

#[tokio::test]
async fn skill_shortcuts_activate_without_spend_then_dispatch_through_the_production_loop() {
    for task_in_command in [false, true] {
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
        let (sources, skills) = crate::skills::SkillContext::compose(
            SmithSkillSources::new().with_user(Skill::inline(
                "inspect",
                "Review changed code",
                "MANUAL_SKILL_CONTEXT_MARKER",
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
        app.composer.replace(if task_in_command {
            "/inspect examine @file:notes.txt"
        } else {
            "/inspect"
        });
        let mut keys = Box::pin(
            futures_util::stream::iter([Ok(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))])
            .chain(futures_util::stream::pending()),
        );
        let (exit, mut app) = Box::pin(run_scripted_tui(
            &host,
            project.path(),
            &resources,
            app,
            &mut keys,
            false,
        ))
        .await
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
        assert_eq!(submission.is_some(), task_in_command);
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
        let initial = if let Some(submission) = submission {
            app.schedule_prepared(submission);
            Vec::new()
        } else {
            app.composer.replace("/inspect Hello.");
            vec![Ok(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))]
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
        let (exit, _) = Box::pin(run_scripted_tui(
            &host,
            project.path(),
            &resources,
            app,
            &mut keys,
            false,
        ))
        .await
        .unwrap();
        assert!(matches!(exit, InteractiveExit::Quit(..)));
        let requests = provider.requests();
        assert_eq!(requests.len(), 1);
        let wire = serde_json::to_string(&requests[0].messages).unwrap();
        assert!(wire.contains("MANUAL_SKILL_CONTEXT_MARKER"), "{wire}");
        if task_in_command {
            assert!(wire.contains("NOTE_PAYLOAD_MARKER"), "{wire}");
        } else {
            assert!(wire.contains("Hello."));
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
