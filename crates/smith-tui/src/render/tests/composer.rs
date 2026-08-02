// composer behavior tests.

    #[test]
    fn pending_input_is_bounded_labelled_and_shares_the_anchored_budget_with_todos() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.composer.replace("correct the active turn");
        let submission = match app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
            Some(crate::app::Action::Submit { submission, .. }) => submission,
            other => panic!("expected a steer submission, got {other:?}"),
        };
        app.accept_steer(
            SteerReceipt {
                id: SteerId::new("steer-1"),
                turn: TurnId::new("turn-1"),
                ordinal: 1,
            },
            submission,
        );
        for index in 1..=5 {
            app.composer.replace(format!("queued turn {index}"));
            app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        app.apply(&event(RuntimeEvent::PlanUpdated {
            revision: 1,
            sensitivity: PlanSensitivity::Public,
            counts: std::collections::BTreeMap::new(),
            items: Some(vec![PlanItemProjection {
                id: "todo-1".to_owned(),
                text: "Run the focused tests".to_owned(),
                status: PlanItemStatus::InProgress,
                reason: None,
            }]),
        }));

        let screen = render(&app, 100, 20, Theme::new().without_color());
        insta_like(
            &screen,
            &[
                "Pending for this turn",
                "process-local",
                "correct the active turn",
                "Queued turns",
                "+2 more queued turns",
                "Todo",
                "Run the focused tests",
            ],
        );
        assert!(
            screen.lines().all(|line| line.width() <= 100),
            "pending input overflowed:\n{screen}"
        );

        app.composer.clear();
        app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        let picker = render(&app, 100, 20, Theme::new().without_color());
        assert!(!picker.contains("Pending for this turn"), "{picker}");
        assert!(!picker.contains("Todo"), "{picker}");
    }

    #[test]
    fn a_conversation_renders_its_transcript_composer_and_footer() {
        let app = conversation();
        let screen = render(&app, 74, 16, Theme::new());
        insta_like(
            &screen,
            &[
                "gpt-5.3",
                "87% ctx",
                "› explain the retry policy",
                "• The retry policy classifies failures.",
                "• Read(src/retry.rs) · ok",
                "Ask Smith to do anything",
            ],
        );
    }

    #[test]
    fn command_completion_renders_above_the_composer_without_a_control_strip() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        for character in "bogus".chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let screen = render(&app, 74, 20, Theme::new().without_color());
        insta_like(
            &screen,
            &[
                "no matching commands",
                "› /bogus",
                "unknown command",
                "gpt-5.3",
            ],
        );
        assert!(!screen.contains("tab complete"), "{screen}");
        assert!(!screen.contains("↑↓ select"), "{screen}");
        assert!(!screen.contains("enter run"), "{screen}");
        assert!(!screen.contains("esc close"), "{screen}");
        assert!(
            !screen.contains("command completion"),
            "the Codex-style completion list must not grow a modal title:\n{screen}"
        );
    }

    #[test]
    fn command_completion_keeps_one_identity_footer_row_at_all_widths() {
        for (width, height) in [(44, 14), (74, 24), (120, 32)] {
            let mut app = App::new("gpt-5.3", "~/work/api");
            app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));

            let screen = render(
                &app,
                width,
                height,
                Theme::new().without_color().without_motion(),
            );
            assert!(screen.contains("/help"), "{width}x{height}: {screen}");
            assert!(
                screen
                    .lines()
                    .last()
                    .is_some_and(|line| line.contains("gpt-5.3")),
                "{width}x{height}: {screen}"
            );
            assert!(
                !screen.contains("tab complete"),
                "{width}x{height}: {screen}"
            );
            assert!(!screen.contains("enter run"), "{width}x{height}: {screen}");
            assert!(!screen.contains("esc close"), "{width}x{height}: {screen}");
        }
    }

    #[test]
    fn command_completion_and_footer_keep_the_codex_color_roles() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));

        {
            let mut terminal = Terminal::new(TestBackend::new(74, 20)).expect("a test terminal");
            terminal
                .draw(|frame| draw(frame, &app, Theme::new()))
                .expect("a frame");
            let buffer = terminal.backend().buffer();
            let row = |y: u16| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            };

            let completion_y = (0..buffer.area.height)
                .find(|y| row(*y).contains("/help"))
                .expect("selected completion row");
            let composer_y = (0..buffer.area.height)
                .find(|y| row(*y).trim_end() == "› /")
                .expect("composer row");
            assert!(
                completion_y < composer_y,
                "completion should sit above the fixed composer"
            );
            let completion = row(completion_y);
            let command_x = u16::try_from(completion.find("/help").expect("command position"))
                .expect("command position fits");
            let description_x = u16::try_from(
                completion
                    .find("list available commands")
                    .expect("description"),
            )
            .expect("description position fits");
            assert_eq!(buffer[(command_x, completion_y)].fg, Color::Cyan);
            assert!(
                buffer[(command_x, completion_y)]
                    .modifier
                    .contains(Modifier::BOLD)
            );
            assert!(
                buffer[(description_x, completion_y)]
                    .modifier
                    .contains(Modifier::DIM)
            );
            assert!(!completion.contains("command completion"));
            assert!(!completion.contains('╭'));
        }

        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let mut terminal = Terminal::new(TestBackend::new(74, 20)).expect("a test terminal");
        terminal
            .draw(|frame| draw(frame, &app, Theme::new()))
            .expect("a frame");
        let buffer = terminal.backend().buffer();
        let footer_y = buffer.area.height - 1;
        let footer = (0..buffer.area.width)
            .map(|x| buffer[(x, footer_y)].symbol())
            .collect::<String>();
        let model_x = u16::try_from(footer.find("gpt-5.3").expect("model in footer"))
            .expect("model position fits");
        let path_x = u16::try_from(footer.find("~/work/api").expect("path in footer"))
            .expect("path position fits");
        assert_eq!(buffer[(model_x, footer_y)].fg, Color::Cyan);
        assert_eq!(buffer[(path_x, footer_y)].fg, Color::Green);
    }

    #[test]
    fn a_narrow_footer_keeps_the_model_and_drops_detail() {
        let app = conversation();
        let screen = render(&app, 44, 14, Theme::new());
        let footer = screen.lines().last().unwrap_or_default();
        assert!(footer.contains("gpt-5.3"), "{footer}");
        assert!(
            footer.width() <= 44,
            "the footer must not overflow: {footer}"
        );
    }

    #[test]
    fn non_default_reasoning_hint_is_width_gated_in_the_existing_footer() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.status
            .set_reasoning_hint(Some("think on · effort high".to_owned()));
        let narrow = render(&app, 74, 14, Theme::new().without_color().without_motion());
        assert!(!narrow.contains("effort high"), "{narrow}");

        let wide = render(&app, 120, 14, Theme::new().without_color().without_motion());
        assert!(wide.contains("think on · effort high"), "{wide}");
    }

    #[test]
    fn ctrl_c_exit_hint_replaces_then_restores_the_footer_at_all_widths() {
        for (width, height) in [(44, 14), (74, 24), (120, 32)] {
            let mut app = App::new("glm-5.2", "/Volumes/Data/codes/ai/agent-runtime:main");
            app.status.switch_model(Some("zai".to_owned()), "glm-5.2");
            app.status.set_agent("build");
            let theme = Theme::new().without_color().without_motion();

            let baseline = render(&app, width, height, theme);
            let baseline_footer = baseline.lines().last().unwrap_or_default().to_owned();
            assert!(baseline_footer.contains("glm-5.2"), "{baseline_footer}");

            assert_eq!(
                app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
                None
            );
            let armed = render(&app, width, height, theme);
            let armed_footer = armed.lines().last().unwrap_or_default();
            assert_eq!(
                armed_footer.trim(),
                "press Ctrl+C again to exit",
                "{width}x{height}: {armed}"
            );
            assert!(!armed_footer.contains("glm-5.2"), "{armed_footer}");
            assert!(!armed_footer.contains("? ctx"), "{armed_footer}");

            assert!(app.expire_ctrl_c_exit_hint_at(
                std::time::Instant::now() + std::time::Duration::from_secs(1)
            ));
            let restored = render(&app, width, height, theme);
            assert_eq!(
                restored.lines().last().unwrap_or_default(),
                baseline_footer,
                "{width}x{height}: {restored}"
            );
        }
    }
