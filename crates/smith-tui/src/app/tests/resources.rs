// resources behavior tests.

    #[test]
    fn a_known_slash_command_dispatches_locally_without_a_send() {
        let mut app = app();
        type_text(&mut app, "/model model-2");
        let action = app.on_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            Some(Action::Reconfigure(PaletteCommand::Model {
                provider: "local".into(),
                model: "model-2".into(),
            })),
            "a slash command runs the same host action as the palette"
        );
        assert!(
            !app.transcript
                .blocks()
                .iter()
                .any(|block| matches!(block, Block::User { .. })),
            "an intercepted command must not become a user turn"
        );
    }

    #[test]
    fn an_unknown_slash_command_fails_locally_and_names_help() {
        let mut app = app();
        type_text(&mut app, "/frobnicate");
        let action = app.on_key(key(KeyCode::Enter));
        assert_eq!(action, None, "no provider request may result");
        let error = match &app.overlay {
            Some(Overlay::Palette {
                error: Some(error), ..
            }) => error,
            other => panic!("expected a local command error, got {other:?}"),
        };
        assert!(error.contains("/help"), "{error}");
    }

    #[test]
    fn slash_help_lists_every_command_locally() {
        let mut app = app();
        type_text(&mut app, "/help");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        let help = app
            .transcript
            .blocks()
            .iter()
            .find_map(|block| match block {
                Block::LocalResult { title, content, .. } if title == "help" => {
                    Some(content.clone())
                }
                _ => None,
            })
            .expect("an inline help result");
        for command in [
            "/help",
            "/status",
            "/context",
            "/new",
            "/resume",
            "/profile",
            "/provider",
            "/model",
            "/agent",
            "/diff",
            "/review",
            "/undo",
            "/revert",
            "/quit",
        ] {
            assert!(help.contains(command), "help must list {command}");
        }
    }

    #[test]
    fn question_mark_opens_the_same_local_help_without_a_provider_send() {
        let mut app = app();
        assert_eq!(app.on_key(key(KeyCode::Char('?'))), None);
        assert!(app.composer.is_empty());
        let help = app
            .transcript
            .blocks()
            .iter()
            .find_map(|block| match block {
                Block::LocalResult { title, content, .. } if title == "help" => Some(content),
                _ => None,
            })
            .expect("question mark should render local help");
        assert!(help.contains("Ctrl+C twice"), "{help}");
        assert!(help.contains("Up/Down browse"), "{help}");
        assert!(help.contains("Ctrl+R searches"), "{help}");
    }

    #[test]
    fn the_palette_emits_typed_safe_boundary_commands() {
        let cases = [
            ("new", PaletteCommand::NewSession),
            (
                "resume session-7",
                PaletteCommand::Resume("session-7".into()),
            ),
            ("profile work", PaletteCommand::Profile("work".into())),
            (
                "provider local",
                PaletteCommand::Model {
                    provider: "local".into(),
                    model: "model-2".into(),
                },
            ),
            (
                "model model-2",
                PaletteCommand::Model {
                    provider: "local".into(),
                    model: "model-2".into(),
                },
            ),
        ];
        for (input, expected) in cases {
            let mut app = app();
            assert_eq!(app.on_key(ctrl('p')), None);
            assert!(matches!(app.overlay, Some(Overlay::Palette { .. })));
            type_text(&mut app, input);
            assert_eq!(
                app.on_key(key(KeyCode::Enter)),
                Some(Action::Reconfigure(expected))
            );
            assert!(app.overlay.is_none());
        }
    }

    #[test]
    fn a_selector_without_a_value_opens_a_local_picker_and_escape_restores_the_draft() {
        let mut app = app();
        app.on_key(ctrl('p'));
        type_text(&mut app, "resume");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(app.composer.is_empty());
        assert!(matches!(
            app.overlay,
            Some(Overlay::ResourcePicker {
                target: ResourceTarget::Resume,
                ..
            })
        ));
        assert_eq!(app.on_key(key(KeyCode::Esc)), None);
        assert_eq!(app.composer.text(), "/resume");
        assert!(app.overlay.is_none());
    }

    #[test]
    fn reasoning_commands_share_typed_direct_and_picker_validation() {
        let mut direct = app();
        type_text(&mut direct, "/think off");
        assert_eq!(
            direct.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(PaletteCommand::Think(Some(false))))
        );

        let mut picker_app = app();
        type_text(&mut picker_app, "/effort");
        assert_eq!(picker_app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            picker_app.overlay,
            Some(Overlay::ResourcePicker {
                target: ResourceTarget::Effort,
                ..
            })
        ));
        picker_app.on_key(key(KeyCode::Down));
        assert_eq!(
            picker_app.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(PaletteCommand::Effort(Some(
                "low".to_owned()
            ))))
        );
    }

    #[test]
    fn unavailable_reasoning_choice_fails_locally_without_reconfiguration() {
        let mut app = app();
        app.resources.thinking[2] = app.resources.thinking[2]
            .clone()
            .disabled("reasoning is mandatory for this provider/model");
        type_text(&mut app, "/think off");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(app.transcript.blocks().iter().any(|block| {
            matches!(block, Block::Error { message } if message.contains("mandatory"))
        }));
    }

    #[test]
    fn model_picker_applies_a_cross_provider_pair_atomically() {
        let mut app = app();
        app.resources.providers.push(ResourceEntry::new(
            "openrouter",
            "openrouter",
            "openai-compatible · 1 model",
        ));
        app.resources.models.push(ResourceEntry::new(
            "openrouter/openai/gpt-4o-mini",
            "openrouter/openai/gpt-4o-mini",
            "configured limits",
        ));
        type_text(&mut app, "/model");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::ResourcePicker {
                target: ResourceTarget::Model,
                ..
            })
        ));
        app.on_key(key(KeyCode::Down));
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(PaletteCommand::Model {
                provider: "openrouter".into(),
                model: "openai/gpt-4o-mini".into(),
            }))
        );
    }

    #[test]
    fn provider_with_several_models_cascades_to_a_scoped_model_picker() {
        let mut app = app();
        app.resources.providers.push(ResourceEntry::new(
            "router",
            "router",
            "openai-compatible · 2 models",
        ));
        app.resources.models.extend([
            ResourceEntry::new("router/alpha", "router/alpha", "configured limits"),
            ResourceEntry::new("router/beta", "router/beta", "configured limits"),
        ]);
        type_text(&mut app, "/provider router");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        let picker = match &app.overlay {
            Some(Overlay::ResourcePicker {
                picker,
                target: ResourceTarget::Model,
                ..
            }) => picker,
            other => panic!("expected a model cascade, got {other:?}"),
        };
        assert_eq!(picker.entries.len(), 2);
        assert!(
            picker
                .entries
                .iter()
                .all(|entry| entry.id.starts_with("router/"))
        );
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(PaletteCommand::Model {
                provider: "router".into(),
                model: "alpha".into(),
            }))
        );
    }

    #[test]
    fn ambiguous_unqualified_model_opens_qualified_choices_without_applying_one() {
        let mut app = app();
        app.resources.providers.extend([
            ResourceEntry::new("a", "a", "one model"),
            ResourceEntry::new("b", "b", "one model"),
        ]);
        app.resources.models.extend([
            ResourceEntry::new("a/shared", "a/shared", "configured limits"),
            ResourceEntry::new("b/shared", "b/shared", "configured limits"),
        ]);
        type_text(&mut app, "/model shared");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        let picker = match &app.overlay {
            Some(Overlay::ResourcePicker {
                picker,
                target: ResourceTarget::Model,
                ..
            }) => picker,
            other => panic!("expected qualified choices, got {other:?}"),
        };
        assert_eq!(picker.filtered_indices().len(), 2);
        assert!(
            app.transcript.blocks().iter().any(
                |block| matches!(block, Block::Error { message } if message.contains("multiple providers"))
            )
        );
    }

    #[test]
    fn empty_model_picker_is_non_effectful_and_points_to_setup() {
        let mut app = app();
        app.resources.models.clear();
        type_text(&mut app, "/model");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        let picker = match &app.overlay {
            Some(Overlay::ResourcePicker { picker, .. }) => picker,
            other => panic!("expected an empty picker, got {other:?}"),
        };
        assert!(picker.entries.is_empty());
        assert!(picker.empty_guidance.contains("smith setup add-model"));
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
    }

    #[test]
    fn a_provider_change_warns_that_the_cache_does_not_transfer() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::ModelProfileResolved {
            provider: "openai".into(),
            model: ModelId::new("gpt-5.3"),
            profile: fingerprint("profile"),
        }));
        // The first resolution is not a change, so it must not add a notice.
        assert!(app.transcript.is_empty());

        app.apply(&event(RuntimeEvent::Usage {
            record: UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance::default(),
                delta: UsageDelta::new().with(CounterKind::InputUncached, 9_000),
            },
        }));
        app.apply(&event(RuntimeEvent::ModelProfileResolved {
            provider: "anthropic".into(),
            model: ModelId::new("claude-opus-5"),
            profile: fingerprint("profile"),
        }));

        match &app.transcript.blocks()[0] {
            Block::Notice { source, text } => {
                assert_eq!(source, "provider");
                assert!(text.contains("not transferable"), "{text}");
            }
            other => panic!("expected a provider notice, got {other:?}"),
        }
        assert_eq!(app.status.context.render(), "~9k");
    }

    #[test]
    fn tab_completes_the_highlighted_palette_entry() {
        // `/re` matches resume, review, redo, and revert; the initial
        // highlight sits on the first match, and Tab must complete that exact
        // entry — not its successor.
        let mut first = app();
        type_text(&mut first, "/re");
        assert_eq!(
            commands::matches("/re").first().map(|command| command.name),
            Some("resume")
        );
        assert_eq!(first.on_key(key(KeyCode::Tab)), None);
        assert_eq!(first.composer.text(), "/resume ");

        // Down moves the highlight without completing; Tab then completes the
        // entry Enter would act on.
        let mut moved = app();
        type_text(&mut moved, "/re");
        moved.on_key(key(KeyCode::Down));
        assert_eq!(moved.on_key(key(KeyCode::Tab)), None);
        assert_eq!(moved.composer.text(), "/review ");
    }

    #[test]
    fn enter_completes_a_bare_reasoning_command_prefix() {
        let mut app = app();
        type_text(&mut app, "/eff");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(app.composer.text(), "/effort ");
        assert!(app.overlay.is_none());
    }

    #[test]
    fn tab_cycles_only_an_empty_idle_main_profile() {
        let mut app = agent_first_app();
        assert_eq!(
            app.on_key(key(KeyCode::Tab)),
            Some(Action::Reconfigure(PaletteCommand::Profile(
                "plan".to_owned()
            )))
        );

        type_text(&mut app, "draft");
        assert_eq!(app.on_key(key(KeyCode::Tab)), None);
        assert_eq!(app.composer.text(), "draft");

        app.composer.clear();
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(Action::Reconfigure(PaletteCommand::Profile(
                "review".to_owned()
            )))
        );

        app.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(app.on_key(key(KeyCode::Tab)), None);
    }

    #[test]
    fn tab_routes_a_legacy_cycle_entry_without_inventing_a_profile() {
        let mut app = agent_first_app();
        app.resources.main_profiles = vec![
            ResourceEntry::new(
                format!("{LEGACY_AGENT_PROFILE_PREFIX}build"),
                "build",
                "legacy build adapter",
            )
            .active(true),
            ResourceEntry::new(
                format!("{LEGACY_AGENT_PROFILE_PREFIX}review"),
                "review",
                "legacy review adapter",
            ),
        ];

        assert_eq!(
            app.on_key(key(KeyCode::Tab)),
            Some(Action::Reconfigure(PaletteCommand::Agent(
                "review".to_owned()
            )))
        );
    }

    #[test]
    fn slash_and_ctrl_p_open_the_same_filtered_registry() {
        let mut slash = app();
        type_text(&mut slash, "/rev");
        let slash_matches = commands::matches(slash.composer.text());

        let mut palette = app();
        palette.on_key(ctrl('p'));
        type_text(&mut palette, "rev");
        let palette_matches = commands::matches(palette.composer.text());

        assert_eq!(slash_matches, palette_matches);
        assert_eq!(
            slash_matches
                .iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            ["review", "revert"]
        );
    }

    #[test]
    fn local_results_append_without_stealing_the_composer() {
        let mut app = app();
        type_text(&mut app, "keep drafting");
        app.scroll_up(4);

        app.show_local_result("status", "model: example");
        app.show_local_empty("agents", "No child agents in this session.");

        assert_eq!(app.composer.text(), "keep drafting");
        assert!(app.overlay.is_none());
        assert!(app.following);
        let results = app
            .transcript
            .blocks()
            .iter()
            .filter_map(|block| match block {
                Block::LocalResult { title, .. } => Some(title.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(results, ["status", "agents"]);
        assert!(
            !app.transcript
                .blocks()
                .iter()
                .any(|block| matches!(block, Block::User { text } if text == "keep drafting")),
            "a local result must not turn the draft into provider input"
        );
    }
