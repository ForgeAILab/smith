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
    fn registered_paste_and_image_labels_keep_their_compact_accented_surface() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.on_paste("one\ntwo\nthree");
        app.attach_image("data:image/png;base64,IMAGE".into(), 32, 32);

        let mut terminal = Terminal::new(TestBackend::new(80, 14)).expect("a test terminal");
        terminal
            .draw(|frame| draw(frame, &app, Theme::new()))
            .expect("a frame");
        let buffer = terminal.backend().buffer();
        let row = (0..buffer.area.height)
            .find(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, *y)].symbol())
                    .collect::<String>()
                    .contains("[Pasted text #1 +3 lines][Image #1 32×32]")
            })
            .expect("composer attachment row");
        let rendered = (0..buffer.area.width)
            .map(|x| buffer[(x, row)].symbol())
            .collect::<String>();
        let paste_x = u16::try_from(rendered.find("[Pasted").expect("paste label"))
            .expect("paste position fits");
        let image_x = u16::try_from(rendered.find("[Image").expect("image label"))
            .expect("image position fits");
        assert_eq!(buffer[(paste_x, row)].fg, Color::Cyan);
        assert_eq!(buffer[(image_x, row)].fg, Color::Cyan);
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

    #[test]
    fn running_background_tasks_appear_in_the_footer_and_clear_when_the_poll_reports_none() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.set_running_tasks(vec![crate::app::RunningTaskSummary {
            task_id: "task:3".to_owned(),
            command_hint: "npm test".to_owned(),
        }]);

        let with_task = render(&app, 100, 20, Theme::new().without_color());
        assert!(with_task.contains("task:3"), "{with_task}");

        app.set_running_tasks(Vec::new());
        let cleared = render(&app, 100, 20, Theme::new().without_color());
        assert!(!cleared.contains("task:3"), "{cleared}");
    }

    #[test]
    fn delegated_agents_panel_lists_children_under_the_hint_with_clocks() {
        use agent_runtime_core::delegation::WorkspacePolicy;
        use agent_runtime_core::ids::ChildId;

        let mut app = App::new("gpt-5.3", "~/work/api");
        for id in ["child-a", "child-b"] {
            app.apply(&event(RuntimeEvent::ChildSpawned {
                child: ChildId::new(id),
                workspace: WorkspacePolicy::ReadOnlyView,
                max_turns: 1,
                max_tokens: None,
                deadline_ms: None,
            }));
        }
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: ChildId::new("child-b"),
            result: "No findings.".to_owned(),
        }));

        let screen = render(&app, 80, 24, Theme::new().without_color());
        insta_like(
            &screen,
            &[
                "● main",
                "○ child-a  read-only",
                "○ child-b  completed · No findings.",
            ],
        );
        let clocks = screen
            .lines()
            .filter(|line| line.contains("child-") && line.ends_with("0s"))
            .count();
        assert_eq!(clocks, 2, "both rows dock a right-aligned clock:\n{screen}");
        let main_row = screen
            .lines()
            .position(|line| line.contains("● main"))
            .expect("a main row");
        let composer_row = screen
            .lines()
            .position(|line| line.contains("Ask Smith to do anything"))
            .expect("the composer placeholder");
        assert!(
            main_row > composer_row,
            "the panel sits below the composer:\n{screen}"
        );
    }

    #[test]
    fn inspecting_a_child_swaps_the_transcript_for_its_log_and_returns_it_on_escape() {
        use agent_runtime_core::delegation::WorkspacePolicy;
        use agent_runtime_core::event::ChildPhase;
        use agent_runtime_core::ids::ChildId;

        let mut app = App::new("gpt-5.3", "~/work/api");
        app.transcript.push_user("explain the retry policy");
        let child = ChildId::new("child-a");
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 3,
            max_tokens: None,
            deadline_ms: None,
        }));
        app.apply(&event(RuntimeEvent::ChildProgress {
            child: child.clone(),
            phase: ChildPhase::ToolCall {
                name: "Grep".to_owned(),
            },
        }));

        let root = render(&app, 80, 24, Theme::new().without_color());
        assert!(
            root.contains("explain the retry policy"),
            "the root timeline is what the transcript shows:\n{root}"
        );
        assert!(
            !root.contains("sub-agent · child-a ran"),
            "a child's tool call is panel activity, not a transcript notice:\n{root}"
        );

        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let inspected = render(&app, 80, 24, Theme::new().without_color());
        insta_like(
            &inspected,
            &[
                "child-a · running",
                "started · read-only · up to 3 turns",
                "ran Grep",
                "esc back to main",
            ],
        );
        assert!(
            !inspected.contains("explain the retry policy"),
            "the inspector borrows the whole transcript region:\n{inspected}"
        );
        // The panel marks which row the region belongs to.
        insta_like(&inspected, &["○ main", "● child-a"]);

        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let restored = render(&app, 80, 24, Theme::new().without_color());
        assert!(
            restored.contains("explain the retry policy")
                && !restored.contains("child-a · running"),
            "esc gives the region back unchanged:\n{restored}"
        );
    }

    #[test]
    fn the_working_row_counts_live_delegated_agents() {
        use agent_runtime_core::delegation::WorkspacePolicy;
        use agent_runtime_core::ids::ChildId;

        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::TurnStarted));
        for id in ["child-a", "child-b"] {
            app.apply(&event(RuntimeEvent::ChildSpawned {
                child: ChildId::new(id),
                workspace: WorkspacePolicy::ReadOnlyView,
                max_turns: 1,
                max_tokens: None,
                deadline_ms: None,
            }));
        }
        let screen = render(&app, 80, 24, Theme::new().without_color());
        insta_like(&screen, &["· 2 agents"]);

        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: ChildId::new("child-b"),
            result: "done".to_owned(),
        }));
        let screen = render(&app, 80, 24, Theme::new().without_color());
        insta_like(&screen, &["· 1 agent"]);
    }

    #[test]
    fn background_tasks_join_the_delegated_panel_with_clocks_and_leave_on_poll() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.set_running_tasks(vec![crate::app::RunningTaskSummary {
            task_id: "task:3".to_owned(),
            command_hint: "npm test".to_owned(),
        }]);

        let screen = render(&app, 80, 24, Theme::new().without_color());
        insta_like(&screen, &["● main", "○ task:3  npm test"]);
        assert!(
            screen
                .lines()
                .any(|line| line.contains("task:3") && line.ends_with("0s")),
            "the task row docks a right-aligned clock:\n{screen}"
        );

        app.set_running_tasks(Vec::new());
        let cleared = render(&app, 80, 24, Theme::new().without_color());
        assert!(!cleared.contains("○ task:3"), "{cleared}");
        assert!(!cleared.contains("● main"), "an empty panel vanishes:\n{cleared}");
    }
