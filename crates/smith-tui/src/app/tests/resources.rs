// resources behavior tests.

    #[test]
    fn a_known_slash_command_dispatches_locally_without_a_send() {
        let mut app = app();
        type_text(&mut app, "/model model-2");
        let action = app.on_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            Some(Action::Reconfigure(SessionControl::Reconfigure(
                SelectionCommand::Model {
                    provider: Some("local".into()),
                    model: "model-2".into(),
                }
            ))),
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
    fn context_window_commands_reconfigure_at_the_idle_boundary() {
        let mut app = app();
        app.set_resources(RuntimeResources {
            context_windows: vec![
                ResourceEntry::new("272k", "272k", "smaller context window"),
                ResourceEntry::new("1m", "1m", "larger context window"),
            ],
            context_window: Some("1m".into()),
            ..RuntimeResources::default()
        });
        type_text(&mut app, "/context 272k");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(SessionControl::Reconfigure(
                SelectionCommand::ContextWindow(Some("272k".into()))
            )))
        );

        type_text(&mut app, "/context default");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(SessionControl::Reconfigure(
                SelectionCommand::ContextWindow(None)
            )))
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
                Block::Local(LocalResult::Help(report)) => {
                    Some(smith_client::help_report::render_plain(report))
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
            "/connect",
            "/disconnect",
            "/model",
            "/agent",
            "/diff",
            "/review",
            "/undo",
            "/revert",
            "/skills",
            "/quit",
        ] {
            assert!(help.contains(command), "help must list {command}");
        }
    }

    #[test]
    fn connect_and_disconnect_use_idle_local_pickers_and_typed_boundaries() {
        let mut connect = app();
        type_text(&mut connect, "/connect");
        assert_eq!(connect.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            connect.overlay,
            Some(Overlay::ResourcePicker {
                target: ResourceTarget::Connect,
                ..
            })
        ));
        connect.on_key(key(KeyCode::Down));
        assert_eq!(
            connect.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(SessionControl::Connect(
                "openrouter".to_owned()
            )))
        );

        let mut disconnect = app();
        type_text(&mut disconnect, "/disconnect local");
        assert_eq!(
            disconnect.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(SessionControl::Disconnect(
                "local".to_owned()
            )))
        );
    }

    #[test]
    fn question_mark_opens_local_shortcuts_without_a_provider_send() {
        let mut app = app();
        assert_eq!(app.on_key(key(KeyCode::Char('?'))), None);
        assert!(app.composer.is_empty());
        assert!(matches!(app.overlay, Some(Overlay::Shortcuts)));
        assert!(app.transcript.is_empty());
        assert!(
            commands::help()
                .keys
                .iter()
                .any(|key| key.key == "Ctrl+C twice")
        );
    }

    #[test]
    fn the_palette_emits_typed_safe_boundary_commands() {
        let cases = [
            ("new", SelectionCommand::NewSession),
            (
                "resume session-7",
                SelectionCommand::Resume("session-7".into()),
            ),
            ("profile work", SelectionCommand::Profile("work".into())),
            (
                "provider local",
                SelectionCommand::Model {
                    provider: Some("local".into()),
                    model: "model-2".into(),
                },
            ),
            (
                "model model-2",
                SelectionCommand::Model {
                    provider: Some("local".into()),
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
                Some(Action::Reconfigure(SessionControl::Reconfigure(expected)))
            );
            assert!(app.overlay.is_none());
        }
    }

    #[test]
    fn a_selector_without_a_value_opens_a_local_picker_and_escape_clears_the_command() {
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
        assert!(app.composer.is_empty());
        assert!(app.overlay.is_none());
    }

    #[test]
    fn reasoning_commands_share_typed_direct_and_picker_validation() {
        let mut direct = app();
        type_text(&mut direct, "/think off");
        assert_eq!(
            direct.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(SessionControl::Reconfigure(
                SelectionCommand::Think(Some(false))
            )))
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
            Some(Action::Reconfigure(SessionControl::Reconfigure(
                SelectionCommand::Effort(Some("low".to_owned()))
            )))
        );
    }

    fn advisor_app() -> App {
        let mut app = app();
        app.resources.advisors = vec![
            ResourceEntry::new("default", "configured default", "none configured").active(true),
            ResourceEntry::new("on", "on", "use the configured advisor")
                .disabled("no advisor is configured; name a profile or provider/model instead"),
            ResourceEntry::new("off", "off", "consult no advisor in this session"),
            ResourceEntry::new("sol", "sol", "acme/advisor-model"),
            ResourceEntry::new("acme/big", "acme/big", "Big"),
        ];
        app
    }

    #[test]
    fn advisor_command_selects_keywords_and_offered_targets_directly() {
        for (input, expected) in [
            ("/advisor off", AdvisorChoice::Off),
            ("/advisor OFF", AdvisorChoice::Off),
            ("/advisor default", AdvisorChoice::Default),
            ("/advisor sol", AdvisorChoice::Target("sol".to_owned())),
            (
                "/advisor acme/big",
                AdvisorChoice::Target("acme/big".to_owned()),
            ),
        ] {
            let mut app = advisor_app();
            type_text(&mut app, input);
            assert_eq!(
                app.on_key(key(KeyCode::Enter)),
                Some(Action::Reconfigure(SessionControl::Reconfigure(
                    SelectionCommand::Advisor(expected)
                ))),
                "{input}"
            );
        }
    }

    #[test]
    fn advisor_command_without_a_value_opens_its_picker() {
        let mut app = advisor_app();
        type_text(&mut app, "/advisor");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::ResourcePicker {
                target: ResourceTarget::Advisor,
                ..
            })
        ));
    }

    #[test]
    fn an_unresolvable_or_unconfigured_advisor_is_refused_before_any_rebuild() {
        for (input, reason) in [
            ("/advisor on", "no advisor is configured"),
            ("/advisor nobody", "not a configured profile or provider/model"),
        ] {
            let mut app = advisor_app();
            type_text(&mut app, input);
            assert_eq!(app.on_key(key(KeyCode::Enter)), None, "{input}");
            let transcript = format!("{:?}", app.transcript);
            assert!(transcript.contains(reason), "{input}: {transcript}");
        }
    }

    #[test]
    fn capabilities_deny_and_allow_narrow_only_the_session() {
        let mut denying = app();
        type_text(&mut denying, "/capabilities deny tool:shell");
        assert_eq!(
            denying.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(SessionControl::Reconfigure(
                SelectionCommand::CapabilityDeny("tool:shell".to_owned())
            )))
        );

        // Lifting something this session never denied would widen the
        // profile, so it is refused before any rebuild.
        let mut widening = app();
        type_text(&mut widening, "/capabilities allow tool:shell");
        assert_eq!(widening.on_key(key(KeyCode::Enter)), None);
        assert!(
            format!("{:?}", widening.transcript).contains("cannot be lifted here"),
            "{:?}",
            widening.transcript
        );

        let mut lifting = app();
        lifting.resources.capability_denials = vec!["tool:shell".to_owned()];
        type_text(&mut lifting, "/capabilities allow tool:shell");
        assert_eq!(
            lifting.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(SessionControl::Reconfigure(
                SelectionCommand::CapabilityAllow("tool:shell".to_owned())
            )))
        );
    }

    #[test]
    fn capabilities_without_a_value_asks_the_host_for_the_listing() {
        let mut app = app();
        type_text(&mut app, "/capabilities");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Command(HostCommand::Capabilities(
                crate::commands::CapabilitiesAction::List
            )))
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
            Some(Action::Reconfigure(SessionControl::Reconfigure(
                SelectionCommand::Model {
                    provider: Some("openrouter".into()),
                    model: "openai/gpt-4o-mini".into(),
                }
            )))
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
            Some(Action::Reconfigure(SessionControl::Reconfigure(
                SelectionCommand::Model {
                    provider: Some("router".into()),
                    model: "alpha".into(),
                }
            )))
        );
    }

    #[test]
    fn model_picker_opens_on_the_active_model_in_a_long_inventory() {
        let mut app = app();
        app.resources.models = (0..466)
            .map(|index| {
                ResourceEntry::new(
                    format!("zai/model-{index}"),
                    format!("Model {index}"),
                    "1M context",
                )
                .active(index == 462)
            })
            .collect();
        app.resources.models[462].id = "zai/glm-5.3".into();
        type_text(&mut app, "/model");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        let Some(Overlay::ResourcePicker { picker, .. }) = &app.overlay else {
            panic!("model picker");
        };
        assert_eq!(picker.selected, 462);
        assert_eq!(
            picker.selected_entry().expect("active model").id,
            "zai/glm-5.3"
        );
        app.on_key(key(KeyCode::Esc));
        app.composer.clear();
        for entry in &mut app.resources.models {
            entry.active = false;
        }
        type_text(&mut app, "/model");
        app.on_key(key(KeyCode::Enter));
        let Some(Overlay::ResourcePicker { picker, .. }) = &app.overlay else {
            panic!("model picker");
        };
        assert_eq!(picker.selected, 0);
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
            Block::Notice { kind: source, text } => {
                assert_eq!(source.label(), "provider");
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
    fn enter_activates_the_highlighted_command_from_the_unfiltered_palette() {
        let mut app = app();
        type_text(&mut app, "/");
        app.on_key(key(KeyCode::Down));
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Command(HostCommand::Goal(GoalAction::Show)))
        );
        assert!(app.overlay.is_none());
        assert!(app.composer.is_empty());
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "/goal");
    }

    #[test]
    fn enter_activates_a_bare_reasoning_command_prefix() {
        let mut app = app();
        type_text(&mut app, "/eff");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::ResourcePicker {
                target: ResourceTarget::Effort,
                ..
            })
        ));
        assert!(app.composer.is_empty());
    }

    #[test]
    fn description_search_finds_switchable_resources_after_name_prefixes() {
        assert_eq!(
            commands::matches("switch")
                .into_iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            ["model", "profile", "provider"]
        );
    }

    #[test]
    fn enter_activates_a_description_match_without_turning_it_into_text() {
        let mut app = app();
        app.on_key(ctrl('p'));
        type_text(&mut app, "switch");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::ResourcePicker {
                target: ResourceTarget::Model,
                ..
            })
        ));
        assert!(app.composer.is_empty());
        assert!(!app
            .transcript
            .blocks()
            .iter()
            .any(|block| matches!(block, Block::User { .. })));
    }

    #[test]
    fn exact_commands_with_invalid_arguments_keep_parser_errors_and_drafts() {
        let mut app = app();
        type_text(&mut app, "/status unexpected");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(app.composer.text(), "/status unexpected");
        let error = match &app.overlay {
            Some(Overlay::Palette {
                error: Some(error), ..
            }) => error,
            other => panic!("expected the parser error to remain visible, got {other:?}"),
        };
        assert!(error.contains("/status"), "{error}");
    }

    #[test]
    fn a_busy_selected_completion_preserves_the_original_search_draft() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        type_text(&mut app, "/eff");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(app.overlay.is_none());
        assert_eq!(app.composer.text(), "/eff");
        assert_feedback_hint(&app, "/effort requires an idle turn; draft preserved");
    }

    #[test]
    fn tab_cycles_only_an_empty_idle_main_profile() {
        let mut app = agent_first_app();
        assert_eq!(
            app.on_key(key(KeyCode::Tab)),
            Some(Action::Reconfigure(SessionControl::Reconfigure(
                SelectionCommand::Profile("plan".to_owned())
            )))
        );

        type_text(&mut app, "draft");
        assert_eq!(app.on_key(key(KeyCode::Tab)), None);
        assert_eq!(app.composer.text(), "draft");

        app.composer.clear();
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(Action::Reconfigure(SessionControl::Reconfigure(
                SelectionCommand::Profile("review".to_owned())
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
            Some(Action::Reconfigure(SessionControl::Reconfigure(
                SelectionCommand::Agent("review".to_owned())
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

        assert_eq!(
            slash_matches
                .iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            palette_matches
                .iter()
                .map(|command| command.name)
                .collect::<Vec<_>>()
        );
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
        use smith_client::message_report::MessageReport;

        let mut app = app();
        type_text(&mut app, "keep drafting");
        app.scroll_up(4);

        app.show_local_report(LocalResult::Message(Box::new(MessageReport::Notice {
            title: "status".to_owned(),
            message: "model: example".to_owned(),
        })));
        app.show_local_report(LocalResult::Message(Box::new(MessageReport::Empty {
            title: "agents".to_owned(),
            message: "No child agents in this session.".to_owned(),
        })));

        assert_eq!(app.composer.text(), "keep drafting");
        assert!(app.overlay.is_none());
        assert!(app.following);
        let results = app
            .transcript
            .blocks()
            .iter()
            .filter_map(|block| match block {
                Block::Local(result) => Some(result.title()),
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

    #[test]
    fn busy_goal_changes_are_feedback_and_preserve_the_command() {
        for command in [
            "/goal finish the task",
            "/goal edit revised objective",
            "/goal budget 100",
            "/goal resume",
            "/goal clear",
        ] {
            let mut app = app();
            app.apply(&event(RuntimeEvent::TurnStarted));
            app.composer.replace(command);
            assert_eq!(app.on_key(key(KeyCode::Enter)), None, "{command}");
            assert_eq!(app.composer.text(), command);
            assert!(app.is_busy());
            assert!(app.transcript.is_empty());
            assert_feedback_hint(
                &app,
                "this goal change requires an idle turn; command preserved",
            );
        }
    }

    #[test]
    fn unchanged_session_and_account_selections_are_feedback() {
        let mut app = app();
        app.resources.current_session = Some("session-7".to_owned());
        app.composer.replace("/resume session-7");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_feedback_hint(&app, "already in the selected session");
        assert!(app.transcript.is_empty());
        assert_eq!(app.resources.current_session.as_deref(), Some("session-7"));

        app.set_accounts(vec![
            ResourceEntry::new("0", "account 1", "active").active(true),
        ]);
        app.composer.replace("/account 1");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_feedback_hint(&app, "already using that account");
        assert!(app.transcript.is_empty());
        assert!(app.resources.accounts[0].active);
        app.on_key(key(KeyCode::Left));
        assert!(app.feedback_notice().is_none());
    }

    #[test]
    fn busy_child_resume_is_feedback_and_preserves_the_draft() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.composer.replace("/agent resume child-1");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(app.composer.text(), "/agent resume child-1");
        assert!(app.is_busy());
        assert_feedback_hint(
            &app,
            "exact child resume requires an idle root turn; draft preserved",
        );
        assert!(app.transcript.is_empty());
    }
