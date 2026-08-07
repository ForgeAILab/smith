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
                // No spawn call preceded these events, so the panel falls
                // back to the root's own profile — the same "inherited"
                // resolution `ChildSpawned` applies when nothing was
                // selected.
                "○ child-a  build · read-only",
                "○ child-b  build · completed · No findings.",
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
    fn a_working_childs_row_shows_the_reviewed_projection_profile_and_coordinator_counts() {
        use agent_runtime_core::delegation::WorkspacePolicy;
        use agent_runtime_core::ids::ChildId;

        let mut app = App::new("gpt-5.3", "~/work/api");
        let spawn_args = serde_json::json!({
            "action": "spawn",
            "task": "review the diff",
            "tools": "read_only",
            "workspace": "shared",
            "profile": "review"
        });
        let display = smith_tools::project_tool_call_display("agent", &spawn_args)
            .expect("reviewed spawn projection");
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("spawn-1"),
            name: "agent".to_owned(),
            argument_keys: vec![
                "action".into(),
                "task".into(),
                "tools".into(),
                "workspace".into(),
                "profile".into(),
            ],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        app.note_pending_spawn("spawn-1", &display);
        app.set_tool_display("spawn-1", display);
        let child = ChildId::new("child-1");
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::SharedProject,
            max_turns: 5,
            max_tokens: None,
            deadline_ms: None,
        }));

        app.apply_child(
            child.as_str(),
            &event(RuntimeEvent::ToolCallRequested {
                call: ToolCallId::new("child-call-1"),
                name: "read".to_owned(),
                argument_keys: vec!["path".to_owned()],
                argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
                arguments: None,
            }),
        );
        app.set_child_tool_display(
            child.as_str(),
            "child-call-1",
            smith_tools::project_tool_call_display("read", &serde_json::json!({"path": "src/retry.rs"}))
                .expect("reviewed read projection"),
        );
        app.set_child_counts(std::collections::BTreeMap::from([(
            child.to_string(),
            crate::app::ChildCounts {
                turns_used: 2,
                max_turns: 5,
                tokens_used: 12_400,
            },
        )]));

        let screen = render(&app, 100, 24, Theme::new().without_color());
        assert!(
            screen.contains("○ child-1  review · Read(src/retry.rs) · 2/5 turns · 12.4k tokens"),
            "the row shows the reviewed projection, not the bare tool name, beside the \
             child's profile and the coordinator's own counts:\n{screen}"
        );
    }

    #[test]
    fn a_child_tool_with_no_reviewed_projection_names_the_tool_with_an_honest_label() {
        use agent_runtime_core::delegation::WorkspacePolicy;
        use agent_runtime_core::ids::ChildId;

        let mut app = App::new("gpt-5.3", "~/work/api");
        let child = ChildId::new("child-plain");
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        }));
        app.apply_child(
            child.as_str(),
            &event(RuntimeEvent::ToolCallRequested {
                call: ToolCallId::new("call-1"),
                name: "mcp.some_third_party_tool".to_owned(),
                argument_keys: vec!["query".to_owned()],
                argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
                arguments: None,
            }),
        );

        let screen = render(&app, 100, 24, Theme::new().without_color());
        let row = screen
            .lines()
            .find(|line| line.contains(child.as_str()))
            .expect("a panel row");
        assert!(
            // `query` is the argument's *key* — safe metadata the fallback
            // always names — never a value, since none was ever supplied
            // here or anywhere on this honest-fallback path.
            row.contains("mcp.some_third_party_tool(query") && row.contains("arguments hidden"),
            "the tool is named with an honest unavailable label rather than a raw argument \
             value: {row}"
        );
    }

    #[test]
    fn a_long_child_activity_clips_before_the_docked_clock() {
        use agent_runtime_core::delegation::WorkspacePolicy;
        use agent_runtime_core::ids::ChildId;

        let mut app = App::new("gpt-5.3", "~/work/api");
        let child = ChildId::new("child-verbose");
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        }));
        let many_keys = (0..12)
            .map(|index| format!("argument_key_number_{index}"))
            .collect::<Vec<_>>();
        app.apply_child(
            child.as_str(),
            &event(RuntimeEvent::ToolCallRequested {
                call: ToolCallId::new("call-1"),
                name: "mcp.some_third_party_tool".to_owned(),
                argument_keys: many_keys,
                argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
                arguments: None,
            }),
        );

        let screen = render(&app, 80, 24, Theme::new().without_color());
        let row = screen
            .lines()
            .find(|line| line.contains(child.as_str()))
            .expect("a panel row");
        assert!(
            row.width() <= 80,
            "the row must not overflow the panel width: {row}"
        );
        assert!(
            row.trim_end().ends_with("0s"),
            "the elapsed clock stays docked at the right edge even though the activity is \
             long enough to clip: {row:?}"
        );
        assert!(
            !row.contains("argument_key_number_11"),
            "the key list is bounded, not an unbounded dump, and the whole thing still \
             clips well short of the clock: {row}"
        );
    }

    #[test]
    fn inspecting_a_child_swaps_the_transcript_for_its_log_and_returns_it_on_escape() {
        use agent_runtime_core::delegation::WorkspacePolicy;
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
        // The child's own stream, folded by the same code as the root's.
        app.apply_child(
            child.as_str(),
            &event(RuntimeEvent::ToolCallRequested {
                call: agent_runtime_core::ids::ToolCallId::new("child-call-1"),
                name: "search".to_owned(),
                argument_keys: vec!["path".to_owned(), "pattern".to_owned()],
                argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
                arguments: None,
            }),
        );
        // The host resolves the call against the child's canonical history
        // and hands back the same redacted projection the root timeline gets.
        app.set_child_tool_display(
            child.as_str(),
            "child-call-1",
            smith_tools::project_tool_call_display(
                "search",
                &serde_json::json!({"path": "src/retry.rs", "pattern": "backoff"}),
            )
            .expect("reviewed search projection"),
        );
        app.apply_child(
            child.as_str(),
            &event(RuntimeEvent::ToolCallCompleted {
                call: agent_runtime_core::ids::ToolCallId::new("child-call-1"),
                name: "search".to_owned(),
                is_error: false,
            }),
        );
        app.set_child_tool_result_preview(child.as_str(), "child-call-1", "src/retry.rs:42");
        // Mid-sentence: uncommitted, and drawn like the root's own streaming
        // answer rather than held back until the child finishes.
        app.apply_child(
            child.as_str(),
            &event(RuntimeEvent::TextDelta {
                request: RequestId::new("child-request-1"),
                attempt: AttemptId::new("child-attempt-1"),
                text: "The retry policy backs off".to_owned(),
            }),
        );

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
                // The child's tool call draws as the root timeline's do:
                // same row, same shape, the same reviewed arguments and
                // result lines beneath it.
                "Search(\"backoff\" · src/retry.rs) · ok",
                "src/retry.rs:42",
                "The retry policy backs off",
                "esc back to main",
            ],
        );
        assert!(
            !root.contains("The retry policy backs off"),
            "a child streaming is not the root session streaming:\n{root}"
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

    #[test]
    fn a_clean_finish_reads_as_success_wherever_its_state_is_named() {
        use agent_runtime_core::delegation::WorkspacePolicy;
        use agent_runtime_core::error::RuntimeError;
        use agent_runtime_core::ids::ChildId;

        let mut app = App::new("gpt-5.3", "~/work/api");
        for id in ["child-done", "child-bad"] {
            app.apply(&event(RuntimeEvent::ChildSpawned {
                child: ChildId::new(id),
                workspace: WorkspacePolicy::ReadOnlyView,
                max_turns: 1,
                max_tokens: None,
                deadline_ms: None,
            }));
        }
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: ChildId::new("child-done"),
            result: "No findings.".to_owned(),
        }));
        app.apply(&event(RuntimeEvent::ChildFailed {
            child: ChildId::new("child-bad"),
            error: RuntimeError::internal("provider refused"),
        }));

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("a test terminal");
        terminal
            .draw(|frame| draw(frame, &app, Theme::new()))
            .expect("a frame");
        let buffer = terminal.backend().buffer().clone();
        // The panel row, not the transcript notice that names the same child.
        let panel_colour_of = |child: &str| {
            let needle = format!("○ {child}");
            for y in 0..buffer.area.height {
                let row: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect();
                if let Some(x) = row.find(&needle) {
                    let column = u16::try_from(x + 2).expect("an on-screen column");
                    return buffer[(column, y)].fg;
                }
            }
            panic!("missing the `{child}` panel row on screen");
        };
        assert_eq!(panel_colour_of("child-done"), Color::Green);
        assert_eq!(
            panel_colour_of("child-bad"),
            Color::Red,
            "a failure must not be recoloured by the success rule"
        );

        // The inspector heading names the same state and must agree with it.
        app.inspect_child("child-done");
        let lines = transcript_lines(&app, Theme::new(), 80);
        let heading = lines
            .iter()
            .find(|line| line.spans.iter().any(|span| span.content == "child-done"))
            .expect("an inspector heading");
        assert_eq!(
            heading
                .spans
                .last()
                .expect("the state segment")
                .style
                .fg,
            Some(Color::Green)
        );
    }

    #[test]
    fn the_inspector_renders_a_child_answer_as_prose_not_one_clipped_line() {
        use agent_runtime_core::delegation::WorkspacePolicy;
        use agent_runtime_core::ids::ChildId;

        let mut app = App::new("gpt-5.3", "~/work/api");
        let child = ChildId::new("child-a");
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        }));
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: child.clone(),
            result: "## report\n\nOne finding in `resolve`.\nA second line.".to_owned(),
        }));
        app.inspect_child("child-a");

        let lines = transcript_lines(&app, Theme::new(), 80);
        let text = |line: &Line<'_>| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };
        let rendered = lines.iter().map(text).collect::<Vec<_>>();
        assert!(
            rendered.iter().all(|line| !line.contains('\n')),
            "an embedded newline draws as a glyph, not a line break: {rendered:#?}"
        );
        assert!(
            rendered.iter().any(|line| line.contains("A second line.")),
            "every line of the answer is kept: {rendered:#?}"
        );

        let heading = lines
            .iter()
            .find(|line| text(line).contains("report"))
            .expect("the answer's own heading");
        assert!(
            !text(heading).contains('#'),
            "the answer renders as Markdown, like the root transcript's prose"
        );
        let code = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|span| span.content == "resolve")
            .expect("inline code in the answer");
        assert_eq!(code.style.fg, Some(Color::Cyan));
    }



    #[test]
    fn a_child_answer_draws_exactly_as_the_root_timeline_draws_one() {
        use agent_runtime_core::delegation::WorkspacePolicy;
        use agent_runtime_core::ids::ChildId;

        const ANSWER: &str = "## report\n\nOne finding in `resolve`.\nA second line.";
        let styled = |lines: &[Line<'static>], needle: &str| {
            lines
                .iter()
                .filter(|line| {
                    line.spans
                        .iter()
                        .any(|span| span.content.contains(needle))
                })
                .map(|line| {
                    line.spans
                        .iter()
                        .map(|span| (span.content.to_string(), span.style))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };

        let mut root = App::new("gpt-5.3", "~/work/api");
        root.transcript.push_text_delta(ANSWER);
        root.transcript.close_open();
        let root_lines = transcript_lines(&root, Theme::new(), 80);

        let mut app = App::new("gpt-5.3", "~/work/api");
        let child = ChildId::new("child-a");
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        }));
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: child.clone(),
            result: ANSWER.to_owned(),
        }));
        app.inspect_child("child-a");
        let child_lines = transcript_lines(&app, Theme::new(), 80);

        for needle in ["report", "One finding", "A second line."] {
            let from_root = styled(&root_lines, needle);
            assert!(!from_root.is_empty(), "`{needle}` is missing from the root");
            assert_eq!(
                styled(&child_lines, needle),
                from_root,
                "a delegated child is an agent that reports back, so its answer \
                 must draw through the same renderer, down to the styles"
            );
        }
    }

    #[test]
    fn open_items_render_first_and_completed_items_collapse_to_one_struck_row() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::PlanUpdated {
            revision: 1,
            sensitivity: PlanSensitivity::Public,
            counts: std::collections::BTreeMap::new(),
            items: Some(vec![
                PlanItemProjection {
                    id: "inspect".to_owned(),
                    text: "Inspect the retry module".to_owned(),
                    status: PlanItemStatus::Completed,
                    reason: None,
                },
                PlanItemProjection {
                    id: "change".to_owned(),
                    text: "Implement the fix".to_owned(),
                    status: PlanItemStatus::InProgress,
                    reason: None,
                },
                PlanItemProjection {
                    id: "verify".to_owned(),
                    text: "Run the focused tests".to_owned(),
                    status: PlanItemStatus::Pending,
                    reason: None,
                },
                PlanItemProjection {
                    id: "docs".to_owned(),
                    text: "Update the docs".to_owned(),
                    status: PlanItemStatus::Completed,
                    reason: None,
                },
                PlanItemProjection {
                    id: "changelog".to_owned(),
                    text: "Note the change in the changelog".to_owned(),
                    status: PlanItemStatus::Completed,
                    reason: None,
                },
            ]),
        }));

        let screen = render(&app, 100, 20, Theme::new().without_color());
        let lines: Vec<&str> = screen.lines().collect();
        let heading = lines
            .iter()
            .position(|line| line.contains("Todo"))
            .expect("todo heading");
        let implement = lines
            .iter()
            .position(|line| line.contains("Implement the fix"))
            .expect("open item in authored order");
        let verify = lines
            .iter()
            .position(|line| line.contains("Run the focused tests"))
            .expect("open item in authored order");
        let collapsed = lines
            .iter()
            .position(|line| line.contains("Note the change in the changelog"))
            .expect("the collapsed row names the most recently completed item");
        assert!(
            heading < implement && implement < verify && verify < collapsed,
            "open items render first in authored order, the collapsed row last:\n{screen}"
        );
        assert!(
            lines[collapsed].contains("(+2 done)"),
            "two completed items sit behind the one the row names:\n{}",
            lines[collapsed]
        );
        assert!(
            !screen.contains("Inspect the retry module") && !screen.contains("Update the docs"),
            "a completed item other than the most recent one gets no row of its own:\n{screen}"
        );

        // The collapsed row's text is struck through and dim, not the
        // Success-toned `[x]` an uncollapsed completed item would have used.
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).expect("a test terminal");
        terminal
            .draw(|frame| draw(frame, &app, Theme::new()))
            .expect("a frame");
        let buffer = terminal.backend().buffer().clone();
        let row = (0..buffer.area.height)
            .find(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, *y)].symbol())
                    .collect::<String>()
                    .contains("Note the change")
            })
            .expect("the collapsed row");
        let rendered = (0..buffer.area.width)
            .map(|x| buffer[(x, row)].symbol())
            .collect::<String>();
        let text_x = u16::try_from(rendered.find("Note the change").expect("collapsed text"))
            .expect("text position fits");
        let cell = &buffer[(text_x, row)];
        assert!(
            cell.modifier.contains(Modifier::CROSSED_OUT),
            "the collapsed row's text is struck through"
        );
        assert!(
            cell.modifier.contains(Modifier::DIM),
            "the collapsed row's text is dim"
        );
    }

    #[test]
    fn a_single_completed_item_reports_no_done_count() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::PlanUpdated {
            revision: 1,
            sensitivity: PlanSensitivity::Public,
            counts: std::collections::BTreeMap::new(),
            items: Some(vec![
                PlanItemProjection {
                    id: "inspect".to_owned(),
                    text: "Inspect the retry module".to_owned(),
                    status: PlanItemStatus::Completed,
                    reason: None,
                },
                PlanItemProjection {
                    id: "verify".to_owned(),
                    text: "Run the focused tests".to_owned(),
                    status: PlanItemStatus::Pending,
                    reason: None,
                },
            ]),
        }));

        let screen = render(&app, 100, 20, Theme::new().without_color());
        assert!(
            screen.contains("Inspect the retry module"),
            "the single completed item still names itself:\n{screen}"
        );
        assert!(
            !screen.contains("done)"),
            "no item is hidden behind it, so no `(+N done)` suffix renders:\n{screen}"
        );
    }

    #[test]
    fn a_cancelled_item_keeps_its_own_row_and_is_excluded_from_the_collapse() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::PlanUpdated {
            revision: 1,
            sensitivity: PlanSensitivity::Public,
            counts: std::collections::BTreeMap::new(),
            items: Some(vec![
                PlanItemProjection {
                    id: "skip".to_owned(),
                    text: "Skip the deprecated path".to_owned(),
                    status: PlanItemStatus::Cancelled,
                    reason: None,
                },
                PlanItemProjection {
                    id: "inspect".to_owned(),
                    text: "Inspect the retry module".to_owned(),
                    status: PlanItemStatus::Completed,
                    reason: None,
                },
                PlanItemProjection {
                    id: "verify".to_owned(),
                    text: "Run the focused tests".to_owned(),
                    status: PlanItemStatus::Completed,
                    reason: None,
                },
            ]),
        }));

        let screen = render(&app, 100, 20, Theme::new().without_color());
        assert!(
            screen.contains("[-] Skip the deprecated path"),
            "a cancelled item keeps its own row among the open items:\n{screen}"
        );
        assert!(
            screen.contains("(+1 done)"),
            "two items are completed, so the cancelled item must not inflate \
             the hidden count to two:\n{screen}"
        );
    }

    #[test]
    fn a_fully_completed_plan_stays_visible_while_working_and_retires_once_the_turn_stops() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::PlanUpdated {
            revision: 1,
            sensitivity: PlanSensitivity::Public,
            counts: std::collections::BTreeMap::new(),
            items: Some(vec![
                PlanItemProjection {
                    id: "inspect".to_owned(),
                    text: "Inspect the retry module".to_owned(),
                    status: PlanItemStatus::Completed,
                    reason: None,
                },
                PlanItemProjection {
                    id: "verify".to_owned(),
                    text: "Run the focused tests".to_owned(),
                    status: PlanItemStatus::Completed,
                    reason: None,
                },
            ]),
        }));

        let while_working = render(&app, 100, 20, Theme::new().without_color());
        assert!(
            while_working.contains("Todo") && while_working.contains("Run the focused tests"),
            "a fully-completed plan still renders while the turn is working:\n{while_working}"
        );

        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        }));
        let after_stop = render(&app, 100, 20, Theme::new().without_color());
        assert!(
            !after_stop.contains("Todo") && !after_stop.contains("Run the focused tests"),
            "once the turn is no longer running, a fully-completed plan retires \
             instead of pinning the finished list until the next turn:\n{after_stop}"
        );
    }

    #[test]
    fn a_sensitive_plan_shows_no_item_text_and_no_collapse_row() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::PlanUpdated {
            revision: 1,
            sensitivity: PlanSensitivity::Sensitive,
            counts: std::collections::BTreeMap::from([("completed".to_owned(), 3)]),
            items: Some(vec![PlanItemProjection {
                id: "hidden".to_owned(),
                text: "Sensitive step".to_owned(),
                status: PlanItemStatus::Completed,
                reason: None,
            }]),
        }));

        let screen = render(&app, 100, 20, Theme::new().without_color());
        assert!(
            !screen.contains("Todo"),
            "a sensitive plan renders no anchored pane, collapsed or otherwise:\n{screen}"
        );
        assert!(!screen.contains("Sensitive step"), "{screen}");
        assert!(!screen.contains("done)"), "{screen}");
    }
