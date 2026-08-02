// transcript behavior tests.

    #[test]
    fn working_indicator_replaces_raw_reasoning_until_the_turn_finishes() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::TurnStarted));

        let waiting = render(&app, 74, 12, Theme::new());
        assert!(waiting.contains("Working… · 0s"), "{waiting}");
        assert!(!waiting.contains("plan 0 active"), "{waiting}");
        assert!(!waiting.contains("tools 0 active"), "{waiting}");

        app.transcript
            .push_reasoning_delta("private draft that resembles the answer", false);

        let working = render(&app, 74, 12, Theme::new());
        assert!(working.contains("Working…"), "{working}");
        assert!(
            !working.contains("private draft that resembles the answer"),
            "{working}"
        );

        app.transcript
            .push_notice("monitor", "a background event arrived");
        app.transcript.push_text_delta("The actual visible answer.");
        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        }));
        let answered = render(&app, 74, 12, Theme::new());
        assert!(
            answered.contains("The actual visible answer."),
            "{answered}"
        );
        assert!(!answered.contains("Working…"), "{answered}");
        assert!(answered.contains("Worked"), "{answered}");
        assert!(
            !answered.contains("private draft that resembles the answer"),
            "{answered}"
        );
    }

    #[test]
    fn tool_only_reasoning_only_and_fallback_states_render_at_all_widths() {
        for (width, height) in [(44, 18), (74, 24), (120, 32)] {
            let theme = Theme::new().without_color().without_motion();

            let mut tool_only = App::new("gpt-5.3", "~/work/api");
            tool_only.apply(&event_at(Timestamp(2_000), RuntimeEvent::TurnStarted));
            tool_only.apply(&event(RuntimeEvent::ToolCallRequested {
                call: ToolCallId::new("search-redacted"),
                name: "search".to_owned(),
                argument_keys: vec!["path".to_owned(), "pattern".to_owned()],
                argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
                arguments: None,
            }));
            tool_only.set_tool_display(
                "search-redacted",
                smith_tools::project_tool_call_display(
                    "search",
                    &serde_json::json!({"pattern": "[redacted]", "path": "src"}),
                )
                .expect("reviewed search projection"),
            );
            tool_only.apply(&event(RuntimeEvent::ToolCallCompleted {
                call: ToolCallId::new("search-redacted"),
                name: "search".to_owned(),
                is_error: false,
            }));
            tool_only.apply(&event_at(
                Timestamp(2_842),
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: false,
                },
            ));
            let tool_screen = render(&tool_only, width, height, theme);
            assert!(
                tool_screen.contains("Search("),
                "{width}x{height}: {tool_screen}"
            );
            assert!(
                tool_screen.contains("[redacted]"),
                "{width}x{height}: {tool_screen}"
            );
            assert!(
                tool_screen.contains(" · ok"),
                "{width}x{height}: {tool_screen}"
            );
            assert!(
                tool_screen.contains("Worked for 842ms"),
                "{width}x{height}: {tool_screen}"
            );

            let mut reasoning_only = App::new("gpt-5.3", "~/work/api");
            reasoning_only.apply(&event_at(Timestamp(3_000), RuntimeEvent::TurnStarted));
            reasoning_only
                .transcript
                .push_reasoning_delta("private chain of thought", false);
            reasoning_only.apply(&event_at(
                Timestamp(3_842),
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: false,
                },
            ));
            let reasoning_screen = render(&reasoning_only, width, height, theme);
            assert!(
                reasoning_screen.contains("Worked for 842ms"),
                "{width}x{height}: {reasoning_screen}"
            );
            assert!(
                !reasoning_screen.contains("reasoning only"),
                "{reasoning_screen}"
            );
            assert!(
                !reasoning_screen.contains("private chain of thought"),
                "{reasoning_screen}"
            );

            let mut unavailable_duration = App::new("gpt-5.3", "~/work/api");
            unavailable_duration.apply(&event(RuntimeEvent::TurnStarted));
            unavailable_duration.apply(&event(RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: false,
            }));
            let unavailable_screen = render(&unavailable_duration, width, height, theme);
            assert!(
                unavailable_screen.contains("Worked"),
                "{unavailable_screen}"
            );
            assert!(
                !unavailable_screen.contains("Worked for"),
                "{unavailable_screen}"
            );

            let mut fallback = App::new("gpt-5.3", "~/work/api");
            fallback.apply(&event(RuntimeEvent::ToolCallRequested {
                call: ToolCallId::new("third-party"),
                name: "third_party".to_owned(),
                argument_keys: vec!["path".to_owned()],
                argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
                arguments: None,
            }));
            fallback.apply(&event(RuntimeEvent::ToolCallCompleted {
                call: ToolCallId::new("third-party"),
                name: "third_party".to_owned(),
                is_error: false,
            }));
            let fallback_screen = render(&fallback, width, height, theme);
            assert!(
                fallback_screen.contains("arguments hidden"),
                "{fallback_screen}"
            );
            assert!(
                !fallback_screen.contains("values protected"),
                "{fallback_screen}"
            );
        }
    }

    #[test]
    fn a_multiline_notice_renders_every_line() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.transcript.push_notice(
            "help",
            "/help — list available commands\n/quit — exit Smith",
        );
        let screen = render(&app, 74, 16, Theme::new());
        assert!(
            screen.contains("/help — list available commands"),
            "{screen}"
        );
        assert!(screen.contains("/quit — exit Smith"), "{screen}");
    }

    #[test]
    fn a_tool_row_states_its_outcome_in_words_not_only_color() {
        let app = conversation();
        // Monochrome rendering must still say `ok`.
        let screen = render(&app, 74, 16, Theme::new().without_color());
        assert!(screen.contains("ok"), "{screen}");
        assert!(screen.contains("• Read(src/retry.rs)"), "{screen}");
    }

    #[test]
    fn compact_tool_rows_show_redacted_details_without_results_or_unknown_values() {
        let call_id = ToolCallId::new("search-1");
        let history = vec![
            Message::assistant(vec![ContentPart::ToolCall(ToolCall {
                id: call_id.clone(),
                name: "search".to_owned(),
                arguments: serde_json::json!({
                    "pattern": "TOP_SECRET_PATTERN",
                    "path": "src/\n\u{1b}[31m\u{202e}tests",
                    "unknown": "TOP_SECRET_UNKNOWN"
                }),
            })]),
            Message::tool_result(ToolResultBlock {
                call_id,
                name: "search".to_owned(),
                content: vec![ContentPart::text("TOP_SECRET_RESULT")],
                is_error: false,
            }),
        ];
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.transcript.replace_from_history(&history);
        app.set_tool_display(
            "search-1",
            smith_tools::project_tool_call_display(
                "search",
                &serde_json::json!({
                    "pattern": "[redacted]",
                    "path": "src/\n\u{1b}[31m\u{202e}tests"
                }),
            )
            .expect("reviewed search projection"),
        );
        app.transcript
            .push_tool_call("unknown-1", "third_party", None, &["path".to_owned()]);
        app.transcript
            .complete_tool_call("unknown-1", ToolStatus::Failed);

        let screen = render(&app, 74, 16, Theme::new().without_color());
        assert!(
            screen.contains("• Search(\"[redacted]\" · src/ [31m tests) · ok"),
            "{screen}"
        );
        assert!(
            screen.contains("• third_party(path · arguments hidden) · failed"),
            "{screen}"
        );
        assert!(!screen.contains("TOP_SECRET_PATTERN"), "{screen}");
        assert!(!screen.contains("TOP_SECRET_UNKNOWN"), "{screen}");
        assert!(!screen.contains("TOP_SECRET_RESULT"), "{screen}");
        assert!(!screen.contains('\u{1b}'), "{screen:?}");
        assert!(!screen.contains('\u{202e}'), "{screen:?}");
        assert!(!screen.contains(glyph::BRANCH), "{screen}");
    }

    #[test]
    fn an_unknown_context_renders_as_a_question_mark_not_zero() {
        let app = App::new("gpt-5.3", "~/work/api");
        let screen = render(&app, 74, 16, Theme::new());
        assert!(screen.contains("? ctx"), "{screen}");
        assert!(!screen.contains("0 ctx"), "{screen}");
    }

    #[test]
    fn every_goal_status_and_unknown_usage_stays_visible_at_supported_widths() {
        let theme = Theme::new().without_color().without_motion();
        for status in [
            GoalStatus::Active,
            GoalStatus::Paused,
            GoalStatus::Blocked,
            GoalStatus::UsageLimited,
            GoalStatus::BudgetLimited,
            GoalStatus::Complete,
        ] {
            let mut app = App::new("m", "p");
            app.status.set_agent("b");
            app.status.set_goal(Some(GoalProjection {
                id: GoalId::new("goal-1"),
                generation: 2,
                objective: "Finish".into(),
                status,
                token_budget: Some(100),
                usage: GoalTokenUsage {
                    charged_tokens: None,
                    provenance: GoalUsageProvenance::Unknown,
                    active_elapsed_ms: 50,
                },
                created_at: Timestamp(10),
                updated_at: Timestamp(20),
                stopped_reason: None,
            }));
            for (width, height) in [(44, 14), (74, 24), (120, 32)] {
                let screen = render(&app, width, height, theme);
                let expected = format!("goal {}", status.as_str());
                assert!(
                    screen.contains(&expected),
                    "{width}x{height} missing {expected}:\n{screen}"
                );
                assert!(screen.contains("?/100 tok"), "{width}x{height}:\n{screen}");
                assert!(
                    screen
                        .lines()
                        .all(|line| line.width() <= usize::from(width)),
                    "{width}x{height} overflowed:\n{screen}"
                );
            }
        }
    }

    #[test]
    fn an_empty_transcript_cannot_enter_a_phantom_scroll_state() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        let _ = render_synced(&mut app, 74, 16, Theme::new());

        app.on_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        let screen = render_synced(&mut app, 74, 16, Theme::new());

        assert!(app.following);
        assert_eq!(app.scroll_back, 0);
        assert!(!screen.contains("following paused"), "{screen}");
    }

    #[test]
    fn a_notice_appears_inline_in_the_transcript() {
        let mut app = conversation();
        assert!(!render(&app, 74, 24, Theme::new()).contains("monitor:build"));

        app.transcript.push_notice("monitor:build", "error[E0433]");
        assert!(render(&app, 74, 24, Theme::new()).contains("monitor:build"));
    }

    #[test]
    fn local_results_render_inline_across_supported_sizes() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.show_local_result(
            "diff · all uncommitted",
            "No changes in this scope.\nBinary file exists; content omitted.",
        );
        assert!(app.overlay.is_none());
        for (width, height) in [(44, 12), (74, 20), (120, 30)] {
            let screen = render(&app, width, height, Theme::new().without_color());
            assert!(screen.contains("/diff · all uncommitted"), "{screen}");
            assert!(screen.contains("No changes"), "{screen}");
            assert!(screen.contains("Binary file"), "{screen}");
            assert!(screen.contains("›"), "{screen}");
        }
    }

    #[test]
    fn wrapped_local_result_continuations_keep_the_content_indent() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.show_local_result("status", format!("session: {}", "x".repeat(80)));
        let screen = render(&app, 44, 14, Theme::new().without_color());
        assert!(
            screen.lines().all(|line| !line.starts_with('x')),
            "a wrapped continuation escaped the local-result indent:\n{screen}"
        );
        assert!(
            screen.lines().filter(|line| line.starts_with('│')).count() >= 2,
            "{screen}"
        );
    }

    #[test]
    fn context_status_stays_bounded_across_supported_widths() {
        let mut app = App::new("glm-4.7", "~/work/api");
        app.show_local_result(
            "status",
            "context window: ~98% input left (~1.1k used / 68.9k budget)\n\
             model window: 200k total · 131k reserved\n\
             context plan: estimated · 8 segments\n\
               system instruction: ~21\n\
               tool schema: ~1.1k\n\
               user input: ~12\n\
             provider input (session): 1.3k",
        );

        for (width, height) in [(44, 30), (74, 24), (120, 24)] {
            let screen = render(&app, width, height, Theme::new().without_color());
            assert!(screen.contains("/status"), "{width}×{height}:\n{screen}");
            assert!(screen.contains("~98% input"), "{width}×{height}:\n{screen}");
            assert!(
                screen
                    .lines()
                    .all(|line| line.width() <= usize::from(width)),
                "{width}×{height} overflowed:\n{screen}"
            );
        }
    }

    #[test]
    fn focused_context_view_keeps_the_grid_and_legend_inline() {
        let mut app = App::new("glm-4.7", "~/work/api");
        app.show_local_result(
            "context",
            "Context usage\n\
             glm-4.7 · ~2k / 123.9k input tokens · ~98% left\n\n\
             ■ ■ ◆ ● · · · · □ □\n\
             · · · · · · · · □ □\n\
             · · · · · · · · □ □\n\
             · · · · · · · · □ □\n\
             · · · · · · · · □ □\n\n\
             Estimated usage by category\n\
             ■ system instructions: ~20 (0.1%)\n\
             ◆ tool schemas: ~500 (0.4%)\n\
             ● history: ~1.5k (1.2%)\n\
             · free input: ~121.9k (98.4%)\n\
             □ output/reasoning reserve: 4k (3.1%)\n\
             counting: estimated · 4 segments\n\
             compaction: enabled on overflow · 74.3k recovery target",
        );

        for (width, height) in [(44, 28), (74, 24), (120, 24)] {
            let screen = render(&app, width, height, Theme::new().without_color());
            assert!(screen.contains("/context"), "{width}×{height}:\n{screen}");
            assert!(
                screen.contains("Estimated usage by category"),
                "{width}×{height}:\n{screen}"
            );
            assert!(
                screen.lines().any(|line| line.contains("■ ■ ◆ ● ·")),
                "{width}×{height}:\n{screen}"
            );
            assert!(
                screen
                    .lines()
                    .all(|line| line.width() <= usize::from(width)),
                "{width}×{height} overflowed:\n{screen}"
            );
        }
    }

    #[test]
    fn empty_error_and_oversized_local_results_name_their_state() {
        let mut empty = App::new("gpt-5.3", "~/work/api");
        empty.show_local_empty("agents", "");
        let empty_screen = render(&empty, 74, 12, Theme::new().without_color());
        assert!(empty_screen.contains("/agents"), "{empty_screen}");
        assert!(empty_screen.contains("• No output."), "{empty_screen}");
        assert!(empty_screen.contains("No output."), "{empty_screen}");

        let mut error = App::new("gpt-5.3", "~/work/api");
        error.show_local_error("diff", "Git inspection is unavailable.");
        let error_screen = render(&error, 74, 12, Theme::new().without_color());
        assert!(error_screen.contains("/diff"), "{error_screen}");
        assert!(
            error_screen.contains("■ Git inspection is unavailable."),
            "{error_screen}"
        );
        assert!(
            error_screen.contains("Git inspection is unavailable."),
            "{error_screen}"
        );

        let mut oversized = App::new("gpt-5.3", "~/work/api");
        oversized.show_local_result("diff", "x".repeat(MAX_LOCAL_RESULT_BYTES + 1));
        let oversized_screen = render(&oversized, 74, 12, Theme::new().without_color());
        assert!(
            oversized_screen.contains("[local result truncated at the display limit]"),
            "{oversized_screen}"
        );
    }

    #[tokio::test]
    async fn a_diff_marks_its_lines_with_signs_not_only_color() {
        let app = edit_approval("once();\n", "twice();\n").await;
        // Monochrome rendering must still distinguish removal from addition.
        let screen = render(&app, 74, 24, Theme::new().without_color());
        insta_like(&screen, &["- once();", "+ twice();"]);
    }

    #[tokio::test]
    async fn malformed_edit_arguments_fall_back_rather_than_show_an_empty_diff() {
        let mut app = conversation();
        // `new_string` is missing: the call cannot be reviewed truthfully.
        app.present_approval(
            prompt(
                "edit",
                serde_json::json!({"path": "src/retry.rs", "old_string": "once();"}),
            )
            .await,
        );
        let screen = render(&app, 74, 24, Theme::new());

        insta_like(&screen, &["\"old_string\"", "y allow once"]);
        assert!(
            !screen.contains("change  "),
            "an unreviewable edit must not claim a diff:\n{screen}"
        );
    }
