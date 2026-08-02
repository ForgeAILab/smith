// input behavior tests.

    #[test]
    fn a_blank_composer_sends_nothing() {
        let mut app = app();
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        type_text(&mut app, "   ");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(app.transcript.is_empty());
    }

    #[test]
    fn escape_interrupts_a_running_turn_and_otherwise_clears_input() {
        let mut app = app();
        type_text(&mut app, "draft");
        assert_eq!(app.on_key(key(KeyCode::Esc)), None);
        assert!(app.composer.is_empty());

        app.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(app.on_key(key(KeyCode::Esc)), Some(Action::Interrupt));
        assert_eq!(app.status.activity, Activity::Interrupting);
    }

    #[test]
    fn accepted_inputs_share_history_while_rejected_input_stays_scratch() {
        let mut app = app();

        type_text(&mut app, "first prompt");
        assert_eq!(
            expect_whole_submission(app.on_key(key(KeyCode::Enter))).display_text(),
            "first prompt"
        );
        type_text(&mut app, "/status");
        assert!(matches!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Command(CommandAction::Status))
        ));
        type_text(&mut app, "!cargo test");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::RunShell {
                command: "cargo test".to_owned()
            })
        );

        type_text(&mut app, "/model missing");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(app.composer.text(), "/model missing");
        assert!(app.overlay.is_none());

        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "!cargo test");
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "/status");
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "first prompt");
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.composer.text(), "/model missing");
    }

    #[test]
    fn keyboard_cursor_movement_keeps_the_composer_editable() {
        let mut app = app();
        type_text(&mut app, "copy");
        app.on_key(key(KeyCode::Left));
        app.on_key(key(KeyCode::Left));
        app.on_key(key(KeyCode::Char('-')));
        assert_eq!(app.composer.text(), "co-py");
    }

    #[test]
    fn arrow_history_restores_a_non_empty_composer_scratch() {
        let mut app = app();
        type_text(&mut app, "completed input");
        app.on_key(key(KeyCode::Enter));
        type_text(&mut app, "work in progress");

        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "completed input");
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.composer.text(), "work in progress");
    }

    #[test]
    fn ctrl_r_search_cycles_accepts_and_cancels_losslessly() {
        let mut app = app();
        for input in ["fix older", "unrelated", "fix newest"] {
            type_text(&mut app, input);
            app.on_key(key(KeyCode::Enter));
        }
        type_text(&mut app, "scratch draft");

        app.on_key(ctrl('r'));
        type_text(&mut app, "FIX");
        assert!(matches!(
            &app.overlay,
            Some(Overlay::HistorySearch { matched, .. })
                if matched.as_deref() == Some("fix newest")
        ));
        app.on_key(ctrl('r'));
        assert!(matches!(
            &app.overlay,
            Some(Overlay::HistorySearch { matched, .. })
                if matched.as_deref() == Some("fix older")
        ));
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.composer.text(), "scratch draft");

        app.on_key(ctrl('r'));
        type_text(&mut app, "fix");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(app.overlay.is_none());
        assert_eq!(app.composer.text(), "fix newest");

        app.on_key(ctrl('r'));
        type_text(&mut app, "missing");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(&app.overlay, Some(Overlay::HistorySearch { .. })));
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.composer.text(), "fix newest");
    }

    #[test]
    fn ctrl_c_from_reverse_search_stashes_the_original_draft() {
        let mut app = app();
        type_text(&mut app, "recover through search");
        app.on_key(ctrl('r'));
        type_text(&mut app, "query");

        assert_eq!(app.on_key(ctrl('c')), None);
        assert!(app.overlay.is_none());
        assert!(app.composer.is_empty());
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "recover through search");
    }

    #[test]
    fn ctrl_r_does_not_steal_an_existing_overlay() {
        let mut app = app();
        type_text(&mut app, "/model");
        app.on_key(key(KeyCode::Enter));
        assert!(matches!(
            &app.overlay,
            Some(Overlay::ResourcePicker {
                target: ResourceTarget::Model,
                ..
            })
        ));

        app.on_key(ctrl('r'));
        assert!(matches!(
            &app.overlay,
            Some(Overlay::ResourcePicker {
                target: ResourceTarget::Model,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn denying_marks_the_tool_row_denied() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("c1"),
            name: "shell".into(),
            argument_keys: vec!["command".into()],
            argument_fingerprint: fingerprint("arguments"),
            arguments: None,
        }));
        app.present_approval(prompt("shell").await);
        app.on_key(key(KeyCode::Char('n')));

        match &app.transcript.blocks()[0] {
            Block::Tool {
                status,
                display,
                protected_summary,
                ..
            } => {
                assert_eq!(*status, ToolStatus::Denied);
                assert!(display.is_none());
                assert_eq!(protected_summary, "command · details unavailable");
            }
            other => panic!("expected a tool block, got {other:?}"),
        }
    }

    #[test]
    fn scrolling_up_pauses_following_until_the_user_returns() {
        let mut app = app();
        app.sync_scroll_limit(30);
        assert!(app.following);

        app.on_key(key(KeyCode::PageUp));
        assert!(!app.following);
        assert_eq!(app.scroll_back, 10);

        app.on_key(key(KeyCode::End));
        assert!(app.following);
        assert_eq!(app.scroll_back, 0);

        app.on_key(key(KeyCode::Home));
        assert!(!app.following);
        assert_eq!(app.scroll_back, 30);

        app.on_key(ctrl('l'));
        assert!(app.following);
        assert_eq!(app.scroll_back, 0);
    }

    #[test]
    fn scrolling_without_overflow_keeps_following_newest() {
        let mut app = app();

        app.on_key(key(KeyCode::PageUp));
        app.on_key(key(KeyCode::Home));

        assert!(app.following);
        assert_eq!(app.scroll_back, 0);
    }

    #[test]
    fn paused_scrolling_keeps_the_visible_offset_when_content_grows() {
        let mut app = app();
        app.sync_scroll_limit(20);
        app.scroll_up(5);

        app.sync_scroll_limit(30);

        assert!(!app.following);
        assert_eq!(app.scroll_back, 15);
    }

    #[test]
    fn tab_never_changes_regions_and_completes_without_execution() {
        let mut app = app();
        app.on_key(key(KeyCode::Tab));
        assert!(app.composer.is_empty());

        type_text(&mut app, "/sta");
        assert!(matches!(app.overlay, Some(Overlay::Palette { .. })));
        assert_eq!(app.on_key(key(KeyCode::Tab)), None);
        assert_eq!(app.composer.text(), "/status");
        assert!(app.overlay.is_none());
        assert!(app.transcript.is_empty(), "completion must not execute");

        app.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(app.composer.text(), "/status");
    }

    #[test]
    fn dismissing_ctrl_p_restores_the_original_draft() {
        let mut app = app();
        type_text(&mut app, "keep this");
        app.on_key(ctrl('p'));
        assert!(app.composer.text().starts_with('/'));
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.composer.text(), "keep this");
        assert!(app.overlay.is_none());
    }

    #[test]
    fn context_planning_telemetry_becomes_bounded_status_state() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::ContextPlanned {
            context: fingerprint("context"),
            cache_plan: fingerprint("cache"),
            segment_count: 2,
            totals: BTreeMap::from([
                (SegmentKind::new("history"), 1_500),
                (SegmentKind::new("tool_schema"), 500),
            ]),
            input_tokens: 2_000,
            input_budget_tokens: 10_000,
            reserved_tokens: 2_000,
            confidence: EstimationConfidence::Estimated,
        }));

        let plan = app.status.context_plan.expect("context plan");
        assert_eq!(plan.fingerprint, fingerprint("context").as_str());
        assert_eq!(plan.cache_fingerprint, fingerprint("cache").as_str());
        assert_eq!(plan.input_tokens, 2_000);
        assert_eq!(plan.input_budget_tokens, 10_000);
        assert_eq!(plan.reserved_tokens, 2_000);
        assert_eq!(plan.segment_count, 2);
        assert_eq!(plan.totals["history"], 1_500);
        assert_eq!(plan.render_footer(), "~80% ctx");
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut app = app();
        let mut release = key(KeyCode::Char('x'));
        release.kind = KeyEventKind::Release;
        app.on_key(release);
        assert!(app.composer.is_empty());
    }
