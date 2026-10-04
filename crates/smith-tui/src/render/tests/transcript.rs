// transcript behavior tests.

    #[test]
    fn a_success_summary_belongs_to_its_turn_and_disappears_after_any_later_block() {
        for (width, height) in [(44, 16), (80, 24), (100, 32)] {
            for append in 0..5 {
                let theme = Theme::new().without_color().without_motion();
                let mut app = App::new("model", "project");
                app.apply(&event_at(Timestamp(1_000), RuntimeEvent::TurnStarted));
                app.transcript.push_text_delta("The turn's answer.");
                app.apply(&event_at(
                    Timestamp(73_000),
                    RuntimeEvent::TurnCompleted {
                        finish: TurnFinish::Completed,
                        visible_output: true,
                    },
                ));
                let before = render(&app, width, height, theme);
                assert_eq!(before.matches("✻ Worked for 1m 12s").count(), 1, "{before}");
                let lines = transcript_lines(&app, theme, width);
                let summary = lines.last().unwrap();
                assert_eq!(summary.to_string(), "✻ Worked for 1m 12s");
                assert!(summary.spans[0].style.add_modifier.contains(Modifier::DIM));
                assert_eq!(app.transcript.len(), 1);
                match append {
                    0 => app.show_local_report(LocalResult::Status(Box::new(status_report()))),
                    1 => app.transcript.push_notice(NoticeKind::Monitor, "later notice"),
                    2 => app.transcript.push_user("another turn"),
                    3 => app
                        .transcript
                        .push_tool_call("later-call", "read", None, &[]),
                    _ => app
                        .transcript
                        .push_reasoning_delta("later hidden reasoning", false),
                }
                let after = render(&app, width, height, theme);
                assert!(!after.contains("Worked"), "{after}");
                app.apply(&event(RuntimeEvent::RegistrySnapshotSealed {
                    snapshot: agent_runtime_registry::Fingerprint::of("registry"),
                    entries: 1,
                }));
                assert!(!render(&app, width, height, theme).contains("Worked"));
            }
        }
    }

    #[test]
    fn a_local_card_appended_before_completion_does_not_receive_the_turn_summary() {
        let mut app = App::new("model", "project");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::TextDelta {
            request: RequestId::new("r"),
            attempt: AttemptId::new("a"),
            text: "The turn's answer.".to_owned(),
        }));
        app.apply(&event(RuntimeEvent::ProviderAttemptOutputCommitted {
            request: RequestId::new("r"),
            attempt: AttemptId::new("a"),
        }));
        app.show_local_report(LocalResult::Status(Box::new(status_report())));
        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        }));
        let screen = render(&app, 100, 32, Theme::new());
        assert!(
            screen.contains("The turn's answer.") && screen.contains("/status"),
            "{screen}"
        );
        assert!(!screen.contains("Worked"), "{screen}");
    }

    #[test]
    fn journal_and_history_replay_preserve_rows_without_replaying_a_success_summary() {
        let events = [
            event_at(Timestamp(1_000), RuntimeEvent::TurnStarted),
            event(RuntimeEvent::TextDelta {
                request: RequestId::new("r"),
                attempt: AttemptId::new("a"),
                text: "The turn's answer.".to_owned(),
            }),
            event(RuntimeEvent::ProviderAttemptOutputCommitted {
                request: RequestId::new("r"),
                attempt: AttemptId::new("a"),
            }),
            event_at(
                Timestamp(1_842),
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ),
        ];
        let bytes = serde_json::to_vec(&events).unwrap();
        let replayed: Vec<EventEnvelope> = serde_json::from_slice(&bytes).unwrap();
        let mut live = App::new("model", "project");
        let mut journal = App::new("model", "project");
        let mut history = App::new("model", "project");
        for event in &events {
            live.apply(event);
        }
        for event in &replayed {
            journal.apply_recovered(event);
        }
        history
            .transcript
            .replace_from_history(&[Message::assistant(vec![ContentPart::Text {
                text: "The turn's answer.".to_owned(),
            }])]);
        assert_eq!(live.transcript.blocks(), journal.transcript.blocks());
        assert_eq!(live.transcript.blocks(), history.transcript.blocks());
        for width in [44, 80, 100] {
            for theme in [Theme::new(), Theme::new().without_color()] {
                let live_lines = transcript_lines(&live, theme, width);
                let journal_lines = transcript_lines(&journal, theme, width);
                let history_lines = transcript_lines(&history, theme, width);
                assert_eq!(live_lines[..live_lines.len() - 2], journal_lines);
                assert_eq!(journal_lines, history_lines);
            }
        }
        // Replacing even an equal-length transcript invalidates a live anchor.
        live.transcript
            .replace_from_history(&[Message::assistant(vec![ContentPart::Text {
                text: "The turn's answer.".to_owned(),
            }])]);
        assert!(!render(&live, 100, 32, Theme::new()).contains("Worked"));
    }

    #[test]
    fn working_indicator_replaces_raw_reasoning_until_the_turn_finishes() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::TurnStarted));

        let waiting = render(&app, 74, 12, Theme::new());
        assert!(
            waiting.contains("✻ Working… (0s · esc to interrupt)"),
            "{waiting}"
        );
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
            .push_notice(NoticeKind::Monitor, "a background event arrived");
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
    fn retry_backoff_and_active_retry_rows_keep_exact_progress_without_color() {
        for (width, height) in [(44, 18), (74, 24)] {
            let theme = Theme::new().without_color().without_motion();
            let mut app = App::new("gpt-5.3", "~/work/api");
            app.apply(&event(RuntimeEvent::TurnStarted));
            app.turn_usage.output = crate::status::TokenCount::estimated(1_200);
            app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
                attempt: AttemptId::new("attempt-1"),
                index: Some(0),
                max_attempts: Some(3),
                finish: agent_runtime_core::provider::FinishReason::Error,
                retryable: true,
                error: Some(agent_runtime_core::provider::ProviderError::new(
                    agent_runtime_core::provider::ProviderErrorKind::Server,
                    "upstream 503",
                )),
                retry_delay_ms: Some(200),
            }));

            let backoff = render(&app, width, height, theme);
            assert!(backoff.contains("Retrying 2/3…"), "{width}x{height}: {backoff}");
            assert!(backoff.contains("backoff <1s"), "{width}x{height}: {backoff}");
            assert!(backoff.contains("↓ ~1.2k"), "{width}x{height}: {backoff}");
            assert!(
                backoff.lines().any(|line| line.starts_with("● Retrying 2/3… (")
                    && line.contains("backoff <1s")
                    && line.contains("esc")
                    && line.ends_with(')')),
                "{width}x{height}: {backoff}"
            );
            assert!(
                backoff.contains("retrying 2/3 in 200ms"),
                "{width}x{height}: {backoff}"
            );

            app.apply(&event(RuntimeEvent::ProviderAttemptStarted {
                request: RequestId::new("request-2"),
                attempt: AttemptId::new("attempt-2"),
                index: 1,
                model: "gpt-5.3".to_owned(),
            }));
            let active = render(&app, width, height, theme);
            assert!(active.contains("Retrying 2/3…"), "{width}x{height}: {active}");
            assert!(active.contains("↑"), "{width}x{height}: {active}");
            assert!(!active.contains("backoff"), "{width}x{height}: {active}");
            assert!(active.contains("↓ ~1.2k"), "{width}x{height}: {active}");
            assert!(
                active.lines().any(|line| line.starts_with("● Retrying 2/3… (")
                    && line.contains("↑")
                    && line.contains("esc")
                    && line.ends_with(')')),
                "{width}x{height}: {active}"
            );
        }
    }

    #[test]
    fn retry_success_clears_progress_and_exhaustion_never_claims_another_retry() {
        let mut succeeded = App::new("gpt-5.3", "~/work/api");
        succeeded.apply(&event(RuntimeEvent::TurnStarted));
        succeeded.apply(&event(RuntimeEvent::ProviderAttemptFinished {
            attempt: AttemptId::new("attempt-1"),
            index: Some(0),
            max_attempts: Some(3),
            finish: agent_runtime_core::provider::FinishReason::Error,
            retryable: true,
            error: Some(agent_runtime_core::provider::ProviderError::new(
                agent_runtime_core::provider::ProviderErrorKind::Server,
                "upstream 503",
            )),
            retry_delay_ms: Some(0),
        }));
        succeeded.apply(&event(RuntimeEvent::ProviderAttemptFinished {
            attempt: AttemptId::new("attempt-2"),
            index: Some(1),
            max_attempts: Some(3),
            finish: agent_runtime_core::provider::FinishReason::Stop,
            retryable: false,
            error: None,
            retry_delay_ms: None,
        }));
        succeeded.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        }));
        let success_screen = render(&succeeded, 74, 18, Theme::new().without_motion());
        assert!(!success_screen.contains("Retrying"), "{success_screen}");
        assert!(!success_screen.contains("backoff"), "{success_screen}");

        let mut exhausted = App::new("gpt-5.3", "~/work/api");
        exhausted.apply(&event(RuntimeEvent::TurnStarted));
        exhausted.apply(&event(RuntimeEvent::ProviderAttemptFinished {
            attempt: AttemptId::new("attempt-3"),
            index: Some(2),
            max_attempts: Some(3),
            finish: agent_runtime_core::provider::FinishReason::Error,
            retryable: true,
            error: Some(agent_runtime_core::provider::ProviderError::new(
                agent_runtime_core::provider::ProviderErrorKind::Server,
                "upstream 503",
            )),
            retry_delay_ms: None,
        }));
        let exhausted_screen = render(&exhausted, 74, 18, Theme::new().without_motion());
        assert!(
            exhausted_screen.contains("failed after 3/3 attempts: Server: upstream 503"),
            "{exhausted_screen}"
        );
        assert!(!exhausted_screen.contains("retrying"), "{exhausted_screen}");
    }

    #[test]
    fn legacy_retry_render_stays_generic_at_narrow_width() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
            attempt: AttemptId::new("legacy-attempt"),
            index: None,
            max_attempts: None,
            finish: agent_runtime_core::provider::FinishReason::Error,
            retryable: true,
            error: Some(agent_runtime_core::provider::ProviderError::new(
                agent_runtime_core::provider::ProviderErrorKind::Server,
                "legacy outage",
            )),
            retry_delay_ms: None,
        }));
        let screen = render(&app, 44, 18, Theme::new().without_motion());
        assert!(screen.contains("attempt failed"), "{screen}");
        assert!(screen.contains("legacy outage"), "{screen}");
        assert!(!screen.contains("/3"), "{screen}");
        assert!(!screen.contains("backoff"), "{screen}");
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
                tool_screen.contains("● Search(") && !tool_screen.contains(" · ok"),
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
            NoticeKind::Help,
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
    fn a_tool_row_states_non_success_in_words_not_only_color() {
        for status in [
            ToolStatus::Failed,
            ToolStatus::Denied,
            ToolStatus::Unreported,
        ] {
            let mut app = conversation();
            app.transcript.complete_tool_call("c1", status);
            let screen = render(&app, 74, 16, Theme::new().without_color());
            assert!(
                screen.contains(&format!("● Read(src/retry.rs) {}", status.label())),
                "{screen}"
            );
        }
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
            screen.contains("● Search(\"[redacted]\" · src/ [31m tests)"),
            "{screen}"
        );
        assert!(
            screen.contains("● third_party(arguments hidden) failed"),
            "{screen}"
        );
        assert!(!screen.contains("TOP_SECRET_PATTERN"), "{screen}");
        assert!(!screen.contains("TOP_SECRET_UNKNOWN"), "{screen}");
        assert!(!screen.contains("TOP_SECRET_RESULT"), "{screen}");
        assert!(!screen.contains('\u{1b}'), "{screen:?}");
        assert!(!screen.contains('\u{202e}'), "{screen:?}");
        assert!(screen.contains("⎿  Completed"), "{screen}");
    }

    #[test]
    fn an_unknown_context_renders_as_a_question_mark_not_zero() {
        let app = App::new("gpt-5.3", "~/work/api");
        let screen = render(&app, 74, 16, Theme::new());
        assert!(screen.contains("unknown ctx"), "{screen}");
        assert!(!screen.contains("0 ctx"), "{screen}");
    }

    #[test]
    fn goal_status_yields_to_shortcuts_at_narrow_widths() {
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
                assert_eq!(screen.contains(&expected), width != 44, "{screen}");
                assert_eq!(screen.contains("?/100 tok"), width != 44, "{screen}");
                assert!(screen.contains("? for shortcuts"), "{screen}");
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

        app.transcript.push_notice(NoticeKind::NamedMonitor("build".to_owned()), "error[E0433]");
        assert!(render(&app, 74, 24, Theme::new()).contains("monitor:build"));
    }

    #[test]
    fn local_results_render_inline_across_supported_sizes() {
        use smith_client::diff_report::{DiffLine, DiffLineKind, DiffOutcome, DiffReport};

        let mut app = App::new("gpt-5.3", "~/work/api");
        app.show_local_report(LocalResult::Diff(Box::new(DiffReport {
            title: "diff · all uncommitted".to_owned(),
            outcome: DiffOutcome::Patch(vec![DiffLine {
                kind: DiffLineKind::Context,
                text: "No changes in this scope.\nBinary file exists; content omitted.".to_owned(),
            }]),
        })));
        assert!(app.overlay.is_none());
        for (width, height) in [(44, 12), (74, 20), (120, 30)] {
            let screen = render(&app, width, height, Theme::new().without_color());
            assert!(screen.contains("/diff · all uncommitted"), "{screen}");
            assert!(screen.contains("No changes"), "{screen}");
            assert!(screen.contains("Binary file"), "{screen}");
            assert!(screen.contains("> Ask Smith to do anything"), "{screen}");
        }
    }

    #[test]
    fn typed_diff_kinds_choose_style_without_title_or_prefix_parsing() {
        use smith_client::diff_report::{DiffLine, DiffLineKind, DiffOutcome, DiffReport};

        let theme = Theme::new();
        for (kind, text, tone) in [
            (
                DiffLineKind::Addition,
                "-removal-looking content",
                Tone::Success,
            ),
            (
                DiffLineKind::Removal,
                "+addition-looking content",
                Tone::Danger,
            ),
            (DiffLineKind::Metadata, "plain file header", Tone::Dim),
            (DiffLineKind::Hunk, "plain hunk header", Tone::Code),
            (
                DiffLineKind::Context,
                "@@ hunk-looking content",
                Tone::Default,
            ),
        ] {
            let mut app = App::new("gpt-5.3", "~/work/api");
            app.show_local_report(LocalResult::Diff(Box::new(DiffReport {
                title: "arbitrary title".to_owned(),
                outcome: DiffOutcome::Patch(vec![DiffLine {
                    kind,
                    text: text.to_owned(),
                }]),
            })));
            let lines = transcript_lines(&app, theme, 8);
            assert_eq!(lines[0].to_string(), "/arbitrary title");
            let body = &lines[1..];
            assert_eq!(
                body.iter().map(ToString::to_string).collect::<String>(),
                text
            );
            assert!(body.len() > 1, "expected a wrapped line");
            assert!(body.iter().all(|line| {
                line.width() <= 8
                    && line
                        .spans
                        .iter()
                        .all(|span| span.style == theme.style(tone))
            }));
        }
    }

    #[test]
    fn typed_diff_empty_and_error_states_keep_their_markers_and_wrapping() {
        use smith_client::diff_report::{DiffOutcome, DiffReport};

        for (outcome, marker, message) in [
            (DiffOutcome::Empty, "●", DiffReport::EMPTY_MESSAGE),
            (
                DiffOutcome::Error("Git inspection is unavailable.".to_owned()),
                "■",
                "Git inspection is unavailable.",
            ),
        ] {
            let mut app = App::new("gpt-5.3", "~/work/api");
            app.show_local_report(LocalResult::Diff(Box::new(DiffReport {
                title: "diff".to_owned(),
                outcome,
            })));
            let lines = transcript_lines(&app, Theme::new().without_color(), 12);
            let body = &lines[1..];
            assert!(body[0].to_string().starts_with(&format!("{marker} ")));
            assert!(
                body.iter()
                    .skip(1)
                    .all(|line| line.to_string().starts_with("  "))
            );
            assert_eq!(
                body.iter()
                    .map(|line| line.to_string().chars().skip(2).collect::<String>())
                    .collect::<String>(),
                message,
            );
            assert!(body.iter().all(|line| line.width() <= 12));
        }
    }

    #[test]
    fn typed_recovery_notices_and_errors_keep_the_previous_transcript_presentation() {
        use smith_client::recovery_report::{RecoveryAction, RecoveryApplied, RecoveryReport};

        for report in [
            RecoveryReport::PreviewError {
                action: RecoveryAction::Undo,
                message: "no Smith turn has attributable changes".to_owned(),
            },
            RecoveryReport::PreviewError {
                action: RecoveryAction::Redo,
                message: "no exact redo candidate exists".to_owned(),
            },
            RecoveryReport::PreviewError {
                action: RecoveryAction::Revert,
                message: "Git-backed change inspection is unavailable outside a Git worktree"
                    .to_owned(),
            },
            RecoveryReport::RevertUsage,
            RecoveryReport::Applied(RecoveryApplied::Undo),
            RecoveryReport::Applied(RecoveryApplied::Redo),
            RecoveryReport::Applied(RecoveryApplied::Revert {
                scope: "path#1".to_owned(),
            }),
            RecoveryReport::ApplyError {
                action: RecoveryAction::Undo,
                message: "undo refused\nmore detail".to_owned(),
            },
            RecoveryReport::ApplyError {
                action: RecoveryAction::Redo,
                message: "redo refused\nmore detail".to_owned(),
            },
            RecoveryReport::ApplyError {
                action: RecoveryAction::Revert,
                message: "revert refused\nmore detail".to_owned(),
            },
            RecoveryReport::Cancelled(RecoveryAction::Undo),
            RecoveryReport::Cancelled(RecoveryAction::Redo),
            RecoveryReport::Cancelled(RecoveryAction::Revert),
        ] {
            let content = smith_client::recovery_report::render_plain(&report);
            let mut legacy = App::new("gpt-5.3", "~/work/api");
            match &report {
                RecoveryReport::PreviewError {
                    action: RecoveryAction::Redo,
                    ..
                } => {
                    legacy.show_local_report(LocalResult::Message(Box::new(
                        smith_client::message_report::MessageReport::Error {
                            title: "redo".to_owned(),
                            message: content,
                        },
                    )));
                }
                RecoveryReport::Applied(_) | RecoveryReport::Cancelled(_) => {
                    legacy
                        .transcript
                        .push_notice(report.action().notice_kind(), content);
                }
                _ => legacy.transcript.push_error(content),
            }
            let mut typed = App::new("gpt-5.3", "~/work/api");
            typed
                .transcript
                .push_local(LocalResult::Recovery(Box::new(report)));
            for width in [44, 100] {
                for theme in [Theme::new(), Theme::new().without_color()] {
                    assert_eq!(
                        transcript_lines(&typed, theme, width),
                        transcript_lines(&legacy, theme, width)
                    );
                    assert_eq!(
                        render(&typed, width, 24, theme),
                        render(&legacy, width, 24, theme)
                    );
                }
            }
        }
    }

    #[test]
    fn typed_recovery_previews_preserve_unstyled_source_and_ignore_display_prefixes() {
        use smith_client::diff_report::{DiffLine, DiffLineKind};
        use smith_client::recovery_report::{RevertOrigin, RevertPreview};

        let patch = vec![
            DiffLine {
                kind: DiffLineKind::Addition,
                text: "origin: unknown\r\n".to_owned(),
            },
            DiffLine {
                kind: DiffLineKind::Context,
                text: "\n".to_owned(),
            },
            DiffLine {
                kind: DiffLineKind::Metadata,
                text: "+source without a final newline".to_owned(),
            },
        ];
        assert_eq!(
            render_recovery_patch(&patch),
            vec![
                Line::from("origin: unknown"),
                Line::default(),
                Line::from("+source without a final newline"),
            ]
        );
        let report = RevertPreview {
            scope: "redo#1".to_owned(),
            fingerprint: "exact-preview".to_owned(),
            origin: RevertOrigin::Smith,
            patch,
        };
        assert_eq!(
            render_revert_preview(&report),
            vec![
                Line::from("origin: Smith"),
                Line::default(),
                Line::from("origin: unknown"),
                Line::default(),
                Line::from("+source without a final newline"),
            ]
        );
    }

    #[test]
    fn typed_review_notices_and_errors_keep_the_previous_transcript_presentation() {
        use smith_client::review_report::{ReviewReport, ReviewStartReport};

        for report in [
            ReviewReport::Empty,
            ReviewReport::Error(
                "Git-backed change inspection is unavailable outside a Git worktree\nmore detail"
                    .to_owned(),
            ),
            ReviewReport::Start(ReviewStartReport::Unavailable),
            ReviewReport::Start(ReviewStartReport::Started {
                child: "child-1".to_owned(),
            }),
            ReviewReport::Start(ReviewStartReport::Queued {
                child: "child-2".to_owned(),
            }),
            ReviewReport::Start(ReviewStartReport::AtCapacity {
                running: 2,
                limit: 2,
            }),
            ReviewReport::Start(ReviewStartReport::Failed("provider unavailable".to_owned())),
        ] {
            let content = smith_client::review_report::render_plain(&report);
            let mut legacy = App::new("gpt-5.3", "~/work/api");
            if matches!(
                &report,
                ReviewReport::Empty
                    | ReviewReport::Start(
                        ReviewStartReport::Started { .. } | ReviewStartReport::Queued { .. }
                    )
            ) {
                legacy.transcript.push_notice(NoticeKind::Review, content);
            } else {
                legacy.transcript.push_error(content);
            }
            let mut typed = App::new("gpt-5.3", "~/work/api");
            typed.transcript.push_local(LocalResult::Review(Box::new(report)));
            for width in [44, 100] {
                for theme in [Theme::new(), Theme::new().without_color()] {
                    assert_eq!(
                        transcript_lines(&typed, theme, width),
                        transcript_lines(&legacy, theme, width),
                    );
                    assert_eq!(
                        render(&typed, width, 24, theme),
                        render(&legacy, width, 24, theme),
                    );
                }
            }
        }
    }

    #[test]
    fn typed_review_confirmation_does_not_recover_structure_from_source_text() {
        use smith_client::diff_report::{DiffLine, DiffLineKind};
        use smith_client::review_report::{ReviewPreview, ReviewReport};

        let lines = render_review_preview(&ReviewPreview {
            scope: "path#1".to_owned(),
            title: "an arbitrary inspection title".to_owned(),
            patch: vec![
                DiffLine {
                    kind: DiffLineKind::Addition,
                    text: "origin: unknown\r\n".to_owned(),
                },
                DiffLine {
                    kind: DiffLineKind::Metadata,
                    text: "+source with a misleading prefix".to_owned(),
                },
            ],
        });
        assert_eq!(
            lines,
            [
                "scope: an arbitrary inspection title",
                "provider-backed: yes",
                "workspace authority: read-only",
                ReviewReport::AUTHORITY_MESSAGE,
                "",
                "origin: unknown",
                "+source with a misleading prefix",
            ]
            .into_iter()
            .map(Line::from)
            .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn wrapped_local_result_continuations_keep_the_content_indent() {
        let mut report = status_report();
        report.session = "a long session description that needs several lines".to_owned();
        let lines = render_status_card(&report, 44, Theme::new().without_color());
        let screen = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            screen.lines().all(|line| line.starts_with("  ")),
            "a wrapped continuation escaped the local-result indent:\n{screen}"
        );
        assert!(
            screen
                .lines()
                .filter(|line| line.starts_with(&" ".repeat(22)))
                .count() >= 2,
            "{screen}"
        );
    }

    #[test]
    fn typed_status_stays_bounded_across_supported_widths() {
        let mut app = App::new("glm-4.7", "~/work/api");
        let mut report = status_report();
        report.usage = "~98% input left (~1.1k used / 68.9k budget)".to_owned();
        app.show_local_report(LocalResult::Status(Box::new(report)));

        for width in [44, 74, 120] {
            let lines = transcript_lines(&app, Theme::new().without_color(), width);
            let screen = lines
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(screen.contains("/status"), "{width} columns:\n{screen}");
            assert!(screen.contains("~98% input"), "{width} columns:\n{screen}");
            assert!(
                lines
                    .iter()
                    .all(|line| line.width() <= usize::from(width)),
                "{width} columns overflowed:\n{screen}"
            );
        }
    }

    #[test]
    fn typed_diagnostics_preserves_wrapping_and_inline_text() {
        use smith_client::diagnostics_report::{DiagnosticsReport, DiagnosticsRow, DiagnosticsSection};

        let report = DiagnosticsReport {
            sections: vec![
                DiagnosticsSection {
                    heading: "Session".to_owned(),
                    rows: vec![
                        DiagnosticsRow::Field {
                            label: "profile".to_owned(),
                            value: "dev".to_owned(),
                        },
                        DiagnosticsRow::Field {
                            label: "goal".to_owned(),
                            value: "Fix `tool:read` and `write`\nFollow **these steps**".to_owned(),
                        },
                    ],
                },
                DiagnosticsSection {
                    heading: "Context".to_owned(),
                    rows: vec![
                        DiagnosticsRow::Field {
                            label: "context".to_owned(),
                            value: "one complete sentence with words that wrap cleanly".to_owned(),
                        },
                        DiagnosticsRow::Field {
                            label: "  tool schema".to_owned(),
                            value: "~500".to_owned(),
                        },
                        DiagnosticsRow::Line("**literal: text** and `inline code`".to_owned()),
                        DiagnosticsRow::Line("Free **text** with `inline code`".to_owned()),
                    ],
                },
            ],
        };
        let mut app = App::new("example-model", "~/work/api");
        app.show_local_report(LocalResult::Diagnostics(Box::new(report)));
        for width in [44, 80, 100] {
            let theme = Theme::new();
            let typed = transcript_lines(&app, theme, width);
            let text = typed.iter().map(ToString::to_string).collect::<Vec<_>>();
            assert_eq!(text[0], "● /diagnostics");
            assert_eq!(text[1], "  Session");
            assert_eq!(text[2], "  profile        dev");
            assert_eq!(text[3], "  goal           Fix `tool:read` and `write`");
            assert_eq!(text[4], "                 Follow **these steps**");
            assert_eq!(text[5], "");
            assert_eq!(text[6], "  Context");
            assert!(
                text.iter().any(|line| line == "    tool schema  ~500"),
                "{text:?}"
            );
            assert!(
                text.iter()
                    .any(|line| line == "  literal: text and inline code"),
                "{text:?}"
            );
            assert!(
                text.iter()
                    .any(|line| line == "  Free text with inline code"),
                "{text:?}"
            );
            if width == 44 {
                assert_eq!(text[7], "  context        one complete sentence with");
                assert_eq!(text[8], "                 words that wrap cleanly");
            } else {
                assert_eq!(
                    text[7],
                    "  context        one complete sentence with words that wrap cleanly"
                );
            }
            assert!(
                typed.iter().all(|line| line.width() <= usize::from(width)),
                "{text:?}"
            );
            assert!(
                typed[1]
                    .spans
                    .iter()
                    .any(|span| span.style == theme.style(Tone::Heading))
            );
            assert!(
                typed[3]
                    .spans
                    .iter()
                    .skip(3)
                    .all(|span| span.style == theme.style(Tone::Default))
            );
            let free = typed
                .iter()
                .find(|line| line.to_string().contains("literal: text"))
                .unwrap();
            assert!(
                free.spans
                    .iter()
                    .any(|span| span.style.add_modifier.contains(Modifier::BOLD))
            );
        }
    }

    #[test]
    fn shell_reports_keep_free_text_presentation_and_explicit_outcomes() {
        use crate::transcript::Block;
        use smith_client::local_result::LocalResultState;
        use smith_client::shell_report::ShellReport;

        let mut app = App::new("example-model", "~/work/api");
        app.show_local_report(LocalResult::Shell(Box::new(ShellReport::new(
            "session: `literal`\n**free text** with `code`\n@@ not a patch heading\n+not an addition",
            false,
        ))));
        let theme = Theme::new();
        let lines = transcript_lines(&app, theme, 100);
        assert_eq!(
            lines.iter().map(ToString::to_string).collect::<Vec<_>>(),
            [
                "/shell",
                "session: literal",
                "free text with code",
                "@@ not a patch heading",
                "+not an addition",
            ],
        );
        assert!(
            lines[1]
                .spans
                .iter()
                .any(|span| span.style == theme.style(Tone::Code))
        );
        assert!(
            lines[4]
                .spans
                .iter()
                .all(|span| span.style == theme.style(Tone::Default))
        );

        for (output, is_error, marker, expected, state) in [
            (
                "failure: `literal`",
                true,
                "■",
                "failure: `literal`",
                LocalResultState::Error,
            ),
            (" \n", false, "●", "No output.", LocalResultState::Empty),
            (" \n", true, "●", "No output.", LocalResultState::Empty),
        ] {
            let mut app = App::new("example-model", "~/work/api");
            app.show_local_report(LocalResult::Shell(Box::new(ShellReport::new(
                output, is_error,
            ))));
            let Block::Local(result) = &app.transcript.blocks()[0] else {
                panic!("expected a shell report");
            };
            assert_eq!(result.state(), state);
            let lines = transcript_lines(&app, theme, 100);
            assert_eq!(lines[0].to_string(), "/shell");
            assert_eq!(lines[1].to_string(), format!("{marker} {expected}"));
        }
    }

    #[test]
    fn message_reports_do_not_select_the_status_card_by_title() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.show_local_report(LocalResult::Message(Box::new(
            smith_client::message_report::MessageReport::Notice {
                title: "status".to_owned(),
                message: "session: text stays text".to_owned(),
            },
        )));
        let screen = render(&app, 74, 24, Theme::new().without_color());
        assert!(screen.contains("session: text stays text"), "{screen}");
        assert!(!screen.contains('╭'), "{screen}");
    }

    #[test]
    fn focused_context_view_keeps_the_grid_and_legend_inline() {
        let mut app = App::new("glm-4.7", "~/work/api");
        use smith_client::context_report::{
            ContextCapacity, ContextCategory, ContextCategoryKind, ContextCompaction, ContextReport,
            ContextUsage,
        };
        app.show_local_report(LocalResult::Context(Box::new(ContextReport {
            available_windows: Vec::new(),
            summary: "glm-4.7 · ~2k / 123.9k input tokens · ~98% left".to_owned(),
            usage: ContextUsage::Estimated,
            categories: vec![
                ContextCategory {
                    kind: ContextCategoryKind::System,
                    label: "system instructions".to_owned(),
                    tokens: 200,
                    value: "~200 (0.1%)".to_owned(),
                },
                ContextCategory {
                    kind: ContextCategoryKind::Tool,
                    label: "tool schemas".to_owned(),
                    tokens: 500,
                    value: "~500 (0.4%)".to_owned(),
                },
                ContextCategory {
                    kind: ContextCategoryKind::History,
                    label: "history".to_owned(),
                    tokens: 1_300,
                    value: "~1.3k (1.0%)".to_owned(),
                },
            ],
            free_input: ContextCapacity {
                tokens: 121_904,
                value: "~121.9k (98.3%)".to_owned(),
            },
            reserve: ContextCapacity {
                tokens: 4_096,
                value: "4k (3.2%)".to_owned(),
            },
            model_window: "128k total · 123.9k input budget".to_owned(),
            counting: "estimated · 3 segments".to_owned(),
            compaction: ContextCompaction::Enabled {
                recovery_target: "74.3k".to_owned(),
            },
            tool_context: "offload above 8192 serialized bytes · artifact pages up to 2048 bytes"
                .to_owned(),
            provider_input: "?".to_owned(),
            cache_read: "?".to_owned(),
            cache: "state unknown · CH ? · misses 0 · re-billed 0 · guarantee ? · maintenance calls 0"
                .to_owned(),
            reasoning: "provider default · effort provider default · provider/model default".to_owned(),
            reasoning_controls: "unsupported · switch unavailable · efforts none · resolved model catalog (presence only)"
                .to_owned(),
        })));

        assert!(app.overlay.is_none(), "context output must stay inline");
        for width in [44, 74, 120] {
            let lines = transcript_lines(&app, Theme::new().without_color(), width);
            let screen = lines
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(screen.contains("/context"), "{width} columns:\n{screen}");
            assert!(
                screen.contains("Estimated usage by category"),
                "{width} columns:\n{screen}"
            );
            assert!(
                screen.lines().any(|line| line.contains("■ ◆ ● ● ·")),
                "{width} columns:\n{screen}"
            );
            assert!(
                lines.iter().all(|line| line.width() <= usize::from(width)),
                "{width} columns overflowed:\n{screen}"
            );
        }
    }

    #[test]
    fn empty_error_and_oversized_local_results_name_their_state() {
        use smith_client::message_report::MessageReport;

        let mut empty = App::new("gpt-5.3", "~/work/api");
        empty.show_local_report(LocalResult::Message(Box::new(MessageReport::Empty {
            title: "agents".to_owned(),
            message: String::new(),
        })));
        let empty_screen = render(&empty, 74, 12, Theme::new().without_color());
        assert!(empty_screen.contains("/agents"), "{empty_screen}");
        assert!(empty_screen.contains("● No output."), "{empty_screen}");
        assert!(empty_screen.contains("No output."), "{empty_screen}");

        let mut error = App::new("gpt-5.3", "~/work/api");
        error.show_local_report(LocalResult::Message(Box::new(MessageReport::Error {
            title: "diff".to_owned(),
            message: "Git inspection is unavailable.".to_owned(),
        })));
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
        oversized.show_local_report(LocalResult::Message(Box::new(MessageReport::Notice {
            title: "diff".to_owned(),
            message: "x".repeat(MAX_LOCAL_RESULT_BYTES + 1),
        })));
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

        insta_like(&screen, &["old_string: once();", "y  Yes"]);
        assert!(
            !screen.contains("change  "),
            "an unreviewable edit must not claim a diff:\n{screen}"
        );
    }

    #[test]
    fn running_tool_call_displays_elapsed_time() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("c1"),
            name: "shell".to_owned(),
            argument_keys: vec!["command".to_owned(), "cwd".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: Some(serde_json::json!({
                "command": "cargo test",
                "cwd": "."
            })),
        }));

        let screen = render(&app, 74, 16, Theme::new().without_color());
        assert!(
            screen.contains("● Bash(cargo test) running 0s"),
            "{screen}"
        );
    }

    #[tokio::test]
    async fn approval_waiting_is_rendered_in_the_tool_and_working_rows() {
        let mut app = App::new("gpt-5.3", "/repo");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("approval-evidence"),
            name: "shell".to_owned(),
            argument_keys: vec!["command".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: Some(serde_json::json!({"command": "git status --short"})),
        }));
        app.present_approval(approval_evidence_prompt("git status --short", false).await);
        app.work_details = true;
        let lines = transcript_lines(&app, Theme::new().without_color(), 100);
        let tool = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            tool.contains("● Bash(git status --short) waiting for approval"),
            "{tool}"
        );
        assert!(!tool.contains("running"), "{tool}");
        let work = working_line(&app, Theme::new().without_color(), 140).to_string();
        assert!(work.contains("tool shell · waiting for approval"), "{work}");
        assert!(!work.contains("running"), "{work}");
    }

    #[test]
    fn denied_tool_result_renders_the_same_live_from_history_and_from_events() {
        let arguments = serde_json::json!({"command": "git status --short"});
        let reason = "approval declined: the user declined";
        let requested = event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("c1"),
            name: "shell".to_owned(),
            argument_keys: vec!["command".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        });
        let completed = event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("c1"),
            name: "shell".to_owned(),
            is_error: true,
        });
        let history = vec![
            Message::assistant(vec![ContentPart::ToolCall(ToolCall {
                id: ToolCallId::new("c1"),
                name: "shell".to_owned(),
                arguments: arguments.clone(),
            })]),
            Message::tool_result(ToolResultBlock {
                call_id: ToolCallId::new("c1"),
                name: "shell".to_owned(),
                content: vec![ContentPart::text(reason)],
                is_error: true,
            }),
        ];
        let mut live = App::new("m", "p");
        live.apply(&requested);
        // The user's decision reaches the row before the completion event.
        live.transcript.complete_tool_call("c1", ToolStatus::Denied);
        live.set_tool_result_preview("c1", reason);
        live.apply(&completed);
        let mut history_replay = App::new("m", "p");
        history_replay.transcript.replace_from_history(&history);
        assert_eq!(
            history_replay.transcript.tool_status("c1"),
            Some(ToolStatus::Denied)
        );
        let mut event_replay = App::new("m", "p");
        event_replay.apply_recovered(&requested);
        event_replay.apply_recovered(&completed);
        for app in [&mut live, &mut history_replay, &mut event_replay] {
            app.set_tool_display(
                "c1",
                smith_tools::project_tool_call_display("shell", &arguments).expect("shell display"),
            );
            app.set_tool_result_preview("c1", reason);
            assert_eq!(app.transcript.tool_status("c1"), Some(ToolStatus::Denied));
        }
        for expanded in [false, true] {
            live.work_details = expanded;
            history_replay.work_details = expanded;
            event_replay.work_details = expanded;
            for width in [100, 80, 44] {
                let screen = render(&live, width, 16, Theme::new().without_color());
                assert!(
                    screen.contains("● Bash(git status --short) denied"),
                    "{screen}"
                );
                assert!(
                    screen.contains("⎿  approval declined: the user declined"),
                    "{screen}"
                );
                assert!(
                    !screen.contains("failed") && !screen.contains("● approval"),
                    "{screen}"
                );
                assert_eq!(
                    screen,
                    render(&history_replay, width, 16, Theme::new().without_color())
                );
                assert_eq!(
                    screen,
                    render(&event_replay, width, 16, Theme::new().without_color())
                );
            }
        }
    }

    #[test]
    fn generate_image_rows_show_progress_saved_path_and_provider_errors() {
        let theme = Theme::new().without_color().without_motion();
        let success_args = serde_json::json!({
            "prompt": "A quiet lake at sunrise",
            "reference_paths": ["assets/shore.png"]
        });
        let mut success = App::new("gpt-5.3", "~/work/api");
        success.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("image-success"),
            name: "generate_image".to_owned(),
            argument_keys: vec!["prompt".to_owned(), "reference_paths".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("image arguments"),
            arguments: Some(success_args.clone()),
        }));
        let display = smith_tools::project_tool_call_display("generate_image", &success_args)
            .expect("reviewed generate_image projection");
        success.set_tool_display("image-success", display.clone());

        let running = render(&success, 100, 20, theme);
        assert!(running.contains("Generate Image("), "{running}");
        assert!(running.contains("A quiet lake at sunrise"), "{running}");
        assert!(running.contains("1 reference"), "{running}");
        assert!(running.contains(" running 0s"), "{running}");

        success.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("image-success"),
            name: "generate_image".to_owned(),
            is_error: false,
        }));
        // Completion re-projects from canonical arguments, as the host does.
        success.set_tool_display("image-success", display);
        success.set_tool_result_preview(
            "image-success",
            "Saved ~/.smith/generated_images/session/image-success.png (1024x1024)",
        );

        let completed = render(&success, 100, 20, theme);
        assert!(completed.contains("Generate Image("), "{completed}");
        assert!(
            !completed.contains("running") && !completed.contains(" · ok"),
            "{completed}"
        );
        assert!(
            completed.contains("Saved ~/.smith/generated_images/session/image-success.png (1024x1024)"),
            "{completed}"
        );

        let failure_args = serde_json::json!({"prompt": "Watercolor meadow"});
        let mut failure = App::new("gpt-5.3", "~/work/api");
        failure.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("image-failure"),
            name: "generate_image".to_owned(),
            argument_keys: vec!["prompt".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("image arguments"),
            arguments: Some(failure_args.clone()),
        }));
        failure.set_tool_display(
            "image-failure",
            smith_tools::project_tool_call_display("generate_image", &failure_args)
                .expect("reviewed generate_image projection"),
        );
        failure.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("image-failure"),
            name: "generate_image".to_owned(),
            is_error: true,
        }));
        failure.set_tool_result_preview("image-failure", "Image provider error: request timed out");

        let failed = render(&failure, 100, 20, theme);
        assert!(failed.contains("Generate Image("), "{failed}");
        assert!(failed.contains(" failed"), "{failed}");
        assert!(
            failed.contains("Image provider error: request timed out"),
            "{failed}"
        );
    }

    // -- Reviewed redundant-row suppression (tool-call-display group 2) ----

    #[test]
    fn successful_write_todos_and_registry_search_rows_are_suppressed_without_a_blank_line_artifact()
     {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.transcript.push_user("plan the work");
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("todos-1"),
            name: "write_todos".to_owned(),
            argument_keys: vec!["items".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        app.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("todos-1"),
            name: "write_todos".to_owned(),
            is_error: false,
        }));
        app.transcript
            .set_tool_result_preview("todos-1", "5 items recorded");
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("search-1"),
            name: "registry.search".to_owned(),
            argument_keys: vec!["query".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        app.set_tool_display(
            "search-1",
            smith_tools::project_tool_call_display(
                "registry.search",
                &serde_json::json!({"query": "browser automation"}),
            )
            .expect("reviewed registry.search projection"),
        );
        app.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("search-1"),
            name: "registry.search".to_owned(),
            is_error: false,
        }));
        app.transcript
            .set_tool_result_preview("search-1", "browser-tool card");
        app.transcript.push_text_delta("Plan set.");
        app.transcript.close_open();

        let lines = transcript_lines(&app, Theme::new(), 74);
        let texts = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(
            !texts.iter().any(|text| text.contains("write_todos")
                || text.contains("Registry Search")
                || text.contains("5 items recorded")
                || text.contains("browser-tool card")),
            "both rows and their previews are suppressed: {texts:#?}"
        );
        let user_index = texts
            .iter()
            .position(|text| text.contains("plan the work"))
            .expect("the user row survives");
        let reply_index = texts
            .iter()
            .position(|text| text.contains("Plan set."))
            .expect("the assistant reply survives");
        // Exactly one blank separator between the two surviving blocks: no
        // doubled or leading blank line was left behind by suppressing the
        // two tool rows between them.
        assert_eq!(
            reply_index,
            user_index + 2,
            "a suppression must not leave a blank-line artifact: {texts:#?}"
        );
        assert_eq!(texts[user_index + 1], "", "{texts:#?}");
    }

    #[test]
    fn a_failed_denied_or_unreported_suppressed_call_still_renders() {
        let mut app = App::new("gpt-5.3", "~/work/api");

        // Failed: `write_todos` has no reviewed schema, so it renders on its
        // honest fallback, but it renders.
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("todos-failed"),
            name: "write_todos".to_owned(),
            argument_keys: vec!["items".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        app.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("todos-failed"),
            name: "write_todos".to_owned(),
            is_error: true,
        }));

        // Denied: matched by tool name, exactly as a real approval denial
        // resolves it.
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("search-denied"),
            name: "registry.search".to_owned(),
            argument_keys: vec!["query".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        app.set_tool_display(
            "search-denied",
            smith_tools::project_tool_call_display(
                "registry.search",
                &serde_json::json!({"query": "browser automation"}),
            )
            .expect("reviewed registry.search projection"),
        );
        app.transcript
            .complete_tool_call_by_name("registry.search", ToolStatus::Denied);

        // Unreported: the conversation ended mid-call.
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("wait-unreported"),
            name: "agent".to_owned(),
            argument_keys: vec!["action".into(), "child_id".into()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        app.set_tool_display(
            "wait-unreported",
            smith_tools::project_tool_call_display(
                "agent",
                &serde_json::json!({"action": "wait", "child_id": "child-1"}),
            )
            .expect("reviewed wait projection"),
        );
        app.transcript.settle_running_tool_calls(ToolStatus::Unreported);

        let screen = render(&app, 100, 20, Theme::new().without_color());
        assert!(
            screen.contains("write_todos(arguments hidden)") && screen.contains("failed"),
            "a failed suppressed call still renders: {screen}"
        );
        assert!(
            screen.contains("Registry Search(\"browser automation\")") && screen.contains("denied"),
            "a denied suppressed call still renders: {screen}"
        );
        assert!(
            screen.contains("Agent(wait · child-1)") && screen.contains(ToolStatus::Unreported.label()),
            "an unreported suppressed call still renders: {screen}"
        );
    }

    #[test]
    fn suppression_matches_between_the_live_and_resumed_paths() {
        let history = vec![
            Message::assistant(vec![ContentPart::ToolCall(ToolCall {
                id: ToolCallId::new("todos-1"),
                name: "write_todos".to_owned(),
                arguments: serde_json::json!({"items": []}),
            })]),
            Message::tool_result(ToolResultBlock {
                call_id: ToolCallId::new("todos-1"),
                name: "write_todos".to_owned(),
                content: vec![ContentPart::text("ok")],
                is_error: false,
            }),
        ];
        let mut resumed = App::new("gpt-5.3", "~/work/api");
        resumed.transcript.replace_from_history(&history);

        let mut live = App::new("gpt-5.3", "~/work/api");
        live.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("todos-1"),
            name: "write_todos".to_owned(),
            argument_keys: vec!["items".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        live.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("todos-1"),
            name: "write_todos".to_owned(),
            is_error: false,
        }));

        let live_screen = render(&live, 74, 16, Theme::new().without_color());
        let resumed_screen = render(&resumed, 74, 16, Theme::new().without_color());
        assert!(!live_screen.contains("write_todos"), "{live_screen}");
        assert!(!resumed_screen.contains("write_todos"), "{resumed_screen}");
    }

    #[test]
    fn agent_follow_up_and_list_rows_are_never_suppressed() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("follow-1"),
            name: "agent".to_owned(),
            argument_keys: vec!["action".into(), "child_id".into(), "task".into()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        app.set_tool_display(
            "follow-1",
            smith_tools::project_tool_call_display(
                "agent",
                &serde_json::json!({
                    "action": "follow_up",
                    "child_id": "child-1",
                    "task": "keep going"
                }),
            )
            .expect("reviewed follow_up projection"),
        );
        app.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("follow-1"),
            name: "agent".to_owned(),
            is_error: false,
        }));

        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("list-1"),
            name: "agent".to_owned(),
            argument_keys: vec!["action".into()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        app.set_tool_display(
            "list-1",
            smith_tools::project_tool_call_display("agent", &serde_json::json!({"action": "list"}))
                .expect("reviewed list projection"),
        );
        app.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("list-1"),
            name: "agent".to_owned(),
            is_error: false,
        }));

        let screen = render(&app, 74, 16, Theme::new().without_color());
        assert!(
            screen.contains("Agent(follow_up · child-1 · \"keep going\")"),
            "{screen}"
        );
        assert!(screen.contains("Agent(list)"), "{screen}");
    }

    // -- The agent row adopts its child's identity (tool-call-display group 3) --

    #[test]
    fn a_spawn_row_adopts_its_childs_identity_and_survives_the_completion_reprojection() {
        use agent_runtime_core::delegation::WorkspacePolicy;
        use agent_runtime_core::ids::ChildId;

        let mut app = App::new("gpt-5.3", "~/work/api");
        app.status.set_agent("build");

        let spawn_args = serde_json::json!({
            "action": "spawn",
            "task": "explore the autoloads and data layer",
            "tools": "read_only",
            "workspace": "shared"
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
            ],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        // Exactly what `tui_driver::run_tui`'s root-events branch does: note
        // the pending spawn, then set the display — in that order, since the
        // queue push only ever happens once, at request time.
        app.note_pending_spawn("spawn-1", &display);
        app.set_tool_display("spawn-1", display);

        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: ChildId::new("child-9"),
            workspace: WorkspacePolicy::SharedProject,
            max_turns: u32::MAX,
            max_tokens: None,
            deadline_ms: None,
        }));

        let screen = render(&app, 220, 16, Theme::new().without_color());
        // The workspace appears exactly once, from the projector, which reads
        // it off the call's own argument. Enrichment adds only what the row
        // does not already say — the child id, and a turn ceiling when the
        // child has one.
        assert!(
            screen.contains(
                "Agent(spawn · \"explore the autoloads and data layer\" · tools read only \
                 · workspace shared · child-9 · profile build (inherited))"
            ),
            "{screen}"
        );
        // The match above is the complete parenthesized invocation, from
        // `Agent(` to its closing paren, so it also pins the absence of a
        // second workspace qualifier. `describe_workspace`'s own spelling
        // (`shared project workspace`) still appears elsewhere on screen —
        // the delegated-work panel row carries it, which is a different
        // surface and its own fact.
        assert!(
            !screen.contains("up to"),
            "an unbounded child must not claim a turn ceiling: {screen}"
        );
        // No second row for the same spawn.
        assert_eq!(screen.matches("Agent(spawn").count(), 1, "{screen}");

        // The trap: the host re-projects `display` from canonical arguments
        // when the tool completes. That must not drop the enrichment.
        app.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("spawn-1"),
            name: "agent".to_owned(),
            is_error: false,
        }));
        let reprojected = smith_tools::project_tool_call_display("agent", &spawn_args)
            .expect("reviewed spawn projection");
        app.set_tool_display("spawn-1", reprojected);

        let after_completion = render(&app, 220, 16, Theme::new().without_color());
        assert!(
            after_completion.contains("child-9")
                && after_completion.contains("shared project workspace")
                && after_completion.contains("profile build (inherited)"),
            "enrichment must survive the tool-completed re-projection: {after_completion}"
        );
    }

    #[test]
    fn a_spawn_that_selected_a_profile_is_not_double_labelled_and_keeps_its_turn_ceiling() {
        use agent_runtime_core::delegation::WorkspacePolicy;
        use agent_runtime_core::ids::ChildId;

        let mut app = App::new("gpt-5.3", "~/work/api");
        let spawn_args = serde_json::json!({
            "action": "spawn",
            "task": "build the feature",
            "tools": "all",
            "workspace": "shared",
            "profile": "explore"
        });
        let display = smith_tools::project_tool_call_display("agent", &spawn_args)
            .expect("reviewed spawn projection");
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("spawn-2"),
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
        app.note_pending_spawn("spawn-2", &display);
        app.set_tool_display("spawn-2", display);
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: ChildId::new("child-explore"),
            workspace: WorkspacePolicy::SharedProject,
            max_turns: 6,
            max_tokens: None,
            deadline_ms: None,
        }));

        let screen = render(&app, 220, 16, Theme::new().without_color());
        assert_eq!(
            screen.matches("profile explore").count(),
            1,
            "the projector's own profile qualifier must not be duplicated: {screen}"
        );
        assert!(
            !screen.contains("(inherited)"),
            "a selected profile is not inherited: {screen}"
        );
        assert!(screen.contains("child-explore"), "{screen}");
        assert!(screen.contains("up to 6 turns"), "{screen}");
    }

    #[test]
    fn two_servers_same_tool_name_render_distinguishably() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        for (call, name) in [
            ("docs-1", "mcp__docs__search"),
            ("wiki-1", "mcp__wiki__search"),
        ] {
            app.apply(&event(RuntimeEvent::ToolCallRequested {
                call: ToolCallId::new(call),
                name: name.to_owned(),
                argument_keys: vec!["query".to_owned()],
                argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
                arguments: None,
            }));
            app.apply(&event(RuntimeEvent::ToolCallCompleted {
                call: ToolCallId::new(call),
                name: name.to_owned(),
                is_error: false,
            }));
        }

        let screen = render(&app, 100, 20, Theme::new().without_color());
        assert!(
            screen.contains("mcp__docs__search") && screen.contains("mcp__wiki__search"),
            "each row names the server it belongs to, so the same tool name on two \
             servers cannot be confused: {screen}"
        );
    }

    #[test]
    fn a_remote_tool_row_hides_its_arguments_and_says_so() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("remote-1"),
            name: "mcp__docs__search".to_owned(),
            argument_keys: vec!["query".to_owned(), "api_token".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));

        let screen = render(&app, 100, 20, Theme::new().without_color());
        assert!(screen.contains("mcp__docs__search"), "{screen}");
        assert!(
            screen.contains("arguments hidden"),
            "the withholding is stated rather than left to be inferred: {screen}"
        );
    }

    #[test]
    fn resumed_remote_tool_row_matches_the_live_row() {
        // A server names its own argument fields, so Smith cannot tell which
        // of them carry secrets — and must not render one from persisted
        // history either.
        let history = vec![
            Message::assistant(vec![ContentPart::ToolCall(ToolCall {
                id: ToolCallId::new("remote-1"),
                name: "mcp__docs__search".to_owned(),
                arguments: serde_json::json!({
                    "query": "boundaries",
                    "api_token": "value-that-must-not-render"
                }),
            })]),
            Message::tool_result(ToolResultBlock {
                call_id: ToolCallId::new("remote-1"),
                name: "mcp__docs__search".to_owned(),
                content: vec![ContentPart::text("ok")],
                is_error: false,
            }),
        ];
        let mut resumed = App::new("gpt-5.3", "~/work/api");
        resumed.transcript.replace_from_history(&history);

        let mut live = App::new("gpt-5.3", "~/work/api");
        live.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("remote-1"),
            name: "mcp__docs__search".to_owned(),
            argument_keys: vec!["api_token".to_owned(), "query".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        live.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("remote-1"),
            name: "mcp__docs__search".to_owned(),
            is_error: false,
        }));

        let live_screen = render(&live, 100, 20, Theme::new().without_color());
        let resumed_screen = render(&resumed, 100, 20, Theme::new().without_color());
        for screen in [&live_screen, &resumed_screen] {
            assert!(screen.contains("mcp__docs__search"), "{screen}");
            assert!(screen.contains("arguments hidden"), "{screen}");
            assert!(!screen.contains("value-that-must-not-render"), "{screen}");
        }
    }

    #[test]
    fn transcript_roles_and_wrapped_rows_hang_under_the_text() {
        for width in [44, 80, 100] {
            let mut app = App::new("m", "p");
            let prose = "The retry policy keeps the cancellation path responsive while repeated provider failures are retried with bounded backoff.";
            app.transcript
                .push_user(format!("{prose}\nsecond user line"));
            app.transcript
                .push_text_delta(&format!("{prose}\nsecond assistant line"));
            app.transcript.close_open();
            let lines = transcript_lines(&app, Theme::new(), width);
            let text = lines.iter().map(ToString::to_string).collect::<Vec<_>>();
            assert!(text[0].starts_with("> The retry"), "{text:#?}");
            assert!(
                text.iter().any(|line| line.starts_with("● The retry")),
                "{text:#?}"
            );
            assert!(
                text.iter().any(|line| line == "  second user line"),
                "{text:#?}"
            );
            assert!(
                text.iter().any(|line| line == "  second assistant line"),
                "{text:#?}"
            );
            assert!(
                text.iter()
                    .filter(|line| !line.is_empty()
                        && !line.starts_with('>')
                        && !line.starts_with('●'))
                    .all(|line| line.starts_with("  ")),
                "{text:#?}"
            );
            assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
        }
    }

    #[test]
    fn one_tool_row_nests_four_lines_and_expands_all_detail() {
        for width in [44, 80, 100] {
            let mut app = App::new("m", "p");
            app.transcript.push_tool_call(
                "bash-1",
                "shell",
                Some(&serde_json::json!({"command": "ls -la"})),
                &[],
            );
            app.transcript.complete_tool_call("bash-1", ToolStatus::Ok);
            app.set_tool_result_preview(
                "bash-1",
                (1..=20)
                    .map(|line| format!("line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            let folded = transcript_lines(&app, Theme::new(), width);
            let text = folded.iter().map(ToString::to_string).collect::<Vec<_>>();
            assert_eq!(
                text,
                [
                    "● Bash(ls -la)",
                    "  ⎿  line 1",
                    "     line 2",
                    "     line 3",
                    "     line 4",
                    "     … +16 lines (ctrl+o to expand)"
                ]
            );
            assert_eq!(folded[0].spans[0].style.fg, Some(Color::Green));
            app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
            let expanded = transcript_lines(&app, Theme::new(), width);
            assert_eq!(expanded.len(), 21);
            assert_eq!(expanded.last().unwrap().to_string(), "     line 20");
            assert_eq!(
                expanded
                    .iter()
                    .filter(|line| line.to_string().contains("Bash("))
                    .count(),
                1
            );
            app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
            assert_eq!(transcript_lines(&app, Theme::new(), width), folded);
        }
    }

    #[test]
    fn running_failed_and_denied_tool_markers_keep_their_status() {
        for (status, color, word) in [
            (ToolStatus::WaitingForApproval, None, "waiting for approval"),
            (ToolStatus::Running, None, "running"),
            (ToolStatus::Failed, Some(Color::Red), "failed"),
            (ToolStatus::Denied, Some(Color::Red), "denied"),
        ] {
            let mut app = App::new("m", "p");
            app.transcript.push_tool_call(
                "c",
                "shell",
                None,
                &["command".into(), "cwd".into(), "timeout_ms".into()],
            );
            if status != ToolStatus::Running {
                app.transcript.complete_tool_call("c", status);
            }
            let lines = transcript_lines(&app, Theme::new(), 100);
            assert_eq!(lines[0].spans[0].style.fg, color);
            if matches!(status, ToolStatus::Running | ToolStatus::WaitingForApproval) {
                assert!(lines[0].spans[0].style.add_modifier.contains(Modifier::DIM));
            }
            let text = lines[0].to_string();
            assert!(text.starts_with("● Bash(arguments hidden)"), "{text}");
            assert!(text.contains(word), "{text}");
            for protected in ["command", "cwd", "timeout_ms", "details unavailable"] {
                assert!(!text.contains(protected), "{text}");
            }
            assert!(
                transcript_lines(&app, Theme::new().without_color(), 100)[0]
                    .to_string()
                    .contains(word)
            );
        }
    }

    #[test]
    fn reads_and_updates_have_one_line_summaries_with_expandable_detail() {
        let mut app = App::new("m", "p");
        app.transcript.push_tool_call(
            "read",
            "read",
            Some(&serde_json::json!({"path": "src/retry.rs"})),
            &[],
        );
        app.transcript.complete_tool_call("read", ToolStatus::Ok);
        app.set_tool_result_preview("read", "1  first\n2  second\n3  third");
        app.transcript.push_tool_call("edit", "edit", Some(&serde_json::json!({"path": "src/retry.rs", "old_string": "before", "new_string": "one\ntwo\nthree\nfour"})), &[]);
        app.transcript.complete_tool_call("edit", ToolStatus::Ok);
        app.set_tool_result_preview("edit", "edited `src/retry.rs` (1 replacement(s))");
        let folded = transcript_lines(&app, Theme::new(), 100)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(
            folded,
            [
                "● Read(src/retry.rs)",
                "  ⎿  Read 3 lines",
                "",
                "● Update(src/retry.rs)",
                "  ⎿  Updated src/retry.rs with 4 additions and 1 removal"
            ]
        );
        app.toggle_work_details();
        let expanded = transcript_lines(&app, Theme::new(), 100)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert!(expanded.iter().any(|line| line == "  ⎿  1  first"));
        assert!(expanded.iter().any(|line| line == "     3  third"));
    }

    #[test]
    fn user_shell_echo_and_runtime_call_are_one_row_with_a_nested_result() {
        let mut app = App::new("m", "p");
        app.composer.replace("!ls -la");
        assert!(matches!(
            app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(crate::app::Action::RunShell { .. })
        ));
        let echo = app.transcript.latest_shell_echo().unwrap();
        app.track_shell_shortcut(TurnId::new("shell-turn"), echo);
        let mut requested = event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("shell-call"),
            name: "shell".into(),
            argument_keys: vec!["command".into(), "cwd".into()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("protected"),
            arguments: None,
        });
        requested.turn = Some(TurnId::new("shell-turn"));
        app.apply(&requested);
        assert_eq!(app.transcript.blocks().len(), 1);
        app.transcript.bind_shell_shortcut(echo, "shell-call");
        app.transcript
            .complete_tool_call("shell-call", ToolStatus::Ok);
        app.set_tool_result_preview("shell-call", "total 8\nfile one\nfile two");
        for width in [44, 80, 100] {
            let text = transcript_lines(&app, Theme::new().without_color(), width)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            assert_eq!(
                text,
                ["! ls -la", "  ⎿  total 8", "     file one", "     file two"]
            );
            assert!(!text.concat().contains("Bash("));
            assert!(!text.concat().contains("/shell"));
            assert!(!text.concat().contains("changes"));
        }
    }

    #[test]
    fn live_and_history_replay_render_the_same_changed_rows() {
        let cases = [
            (
                "shell",
                serde_json::json!({"command": "ls -la"}),
                (1..=20)
                    .map(|line| format!("line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            (
                "read",
                serde_json::json!({"path": "src/retry.rs"}),
                "1  first\n2  second\n3  third".to_owned(),
            ),
            (
                "edit",
                serde_json::json!({"path": "src/retry.rs", "old_string": "before", "new_string": "one\ntwo\nthree\nfour"}),
                "edited `src/retry.rs` (1 replacement(s))".to_owned(),
            ),
            (
                "search",
                serde_json::json!({"pattern": "needle", "path": "src"}),
                "src/retry.rs:42\nsrc/retry.rs:43".to_owned(),
            ),
            (
                "list",
                serde_json::json!({"path": "src"}),
                "retry.rs\nmain.rs".to_owned(),
            ),
            (
                "advisor",
                serde_json::json!({}),
                "Cover cancellation too.".to_owned(),
            ),
            (
                "third_party",
                serde_json::json!({"SECRET_ARGUMENT_NAME": "SECRET_VALUE"}),
                "reviewed result".to_owned(),
            ),
            (
                "write_todos",
                serde_json::json!({"items": []}),
                "recorded".to_owned(),
            ),
            (
                "agent",
                serde_json::json!({"action": "wait", "child_id": "child-1"}),
                "ready".to_owned(),
            ),
            (
                "agent",
                serde_json::json!({"action": "spawn", "task": "review the retry policy"}),
                "child started".to_owned(),
            ),
        ];
        for (name, arguments, output) in cases {
            for is_error in [false, true] {
                let result = if is_error {
                    "permission denied"
                } else {
                    &output
                };
                let user = "Inspect the retry path and explain how cancellation behaves after repeated provider failures.";
                let answer =
                    "The retry policy keeps cancellation responsive while it waits for the provider.";
                let history = [
                    Message::user(user),
                    Message::assistant(vec![
                        ContentPart::Text {
                            text: answer.to_owned(),
                        },
                        ContentPart::ToolCall(ToolCall {
                            id: ToolCallId::new("call"),
                            name: name.to_owned(),
                            arguments: arguments.clone(),
                        }),
                    ]),
                    Message::tool_result(ToolResultBlock {
                        call_id: ToolCallId::new("call"),
                        name: name.to_owned(),
                        content: vec![ContentPart::text(result)],
                        is_error,
                    }),
                ];
                let mut live = App::new("m", "p");
                live.transcript.push_user(user);
                live.transcript.push_text_delta(answer);
                let keys = arguments
                    .as_object()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>();
                live.apply(&event(RuntimeEvent::ToolCallRequested {
                    call: ToolCallId::new("call"),
                    name: name.to_owned(),
                    argument_keys: keys,
                    argument_fingerprint: agent_runtime_registry::Fingerprint::of("protected"),
                    arguments: None,
                }));
                live.apply(&event(RuntimeEvent::ToolCallCompleted {
                    call: ToolCallId::new("call"),
                    name: name.to_owned(),
                    is_error,
                }));
                let mut resumed = App::new("m", "p");
                resumed.transcript.replace_from_history(&history);
                for app in [&mut live, &mut resumed] {
                    if let Some(display) = smith_tools::project_tool_call_display(name, &arguments) {
                        app.set_tool_display("call", display);
                    }
                    app.set_tool_result_preview("call", result);
                }
                for expanded in [false, true] {
                    live.work_details = expanded;
                    resumed.work_details = expanded;
                    for width in [44, 80, 100] {
                        for theme in [Theme::new(), Theme::new().without_color()] {
                            let lines = transcript_lines(&live, theme, width);
                            assert_eq!(
                                lines,
                                transcript_lines(&resumed, theme, width),
                                "{name}, error={is_error}, expanded={expanded}, width={width}"
                            );
                            let text = lines
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join("\n");
                            assert!(
                                !text.contains("SECRET_ARGUMENT_NAME")
                                    && !text.contains("SECRET_VALUE"),
                                "{text}"
                            );
                            assert!(!text.contains("details unavailable"), "{text}");
                        }
                    }
                }
            }
        }

        for (call, is_error, output) in [
            (
                Some("shortcut-call"),
                false,
                (1..=20)
                    .map(|line| format!(" M crates/smith-cli/src/a_long_file_name_{line}.rs"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            (Some("failed-call"), true, "command failed".to_owned()),
            (
                None,
                true,
                "shell action failed: requires an idle session".to_owned(),
            ),
            (
                Some("denied-call"),
                true,
                "approval declined: the user declined".to_owned(),
            ),
            (Some("quiet-call"), false, String::new()),
        ] {
            let history = [
                Message::user("Inspect the worktree"),
                Message::assistant(vec![ContentPart::text("Later answer")]),
            ];
            let mut live = App::new("m", "p");
            live.transcript.push_user("Inspect the worktree");
            let echo = live.transcript.push_shell_shortcut("git status --short");
            let result = live
                .transcript
                .finish_shell_shortcut(echo, call, is_error, &output);
            live.transcript.push_text_delta("Later answer");
            live.transcript.close_open();
            let mut resumed = App::new("m", "p");
            resumed
                .transcript
                .replace_from_history_with_shell_shortcuts(
                    &history,
                    &[crate::transcript::RestoredShellShortcut {
                        anchor: 1,
                        call: call.map(str::to_owned),
                        command: "git status --short".to_owned(),
                        is_error,
                        result,
                    }],
                );
            for expanded in [false, true] {
                assert_eq!(live.work_details, expanded);
                assert_eq!(resumed.work_details, expanded);
                for width in [44, 80, 100] {
                    for theme in [Theme::new(), Theme::new().without_color()] {
                        let lines = transcript_lines(&live, theme, width);
                        assert_eq!(
                            lines,
                            transcript_lines(&resumed, theme, width),
                            "shortcut {call:?}, expanded={expanded}, width={width}"
                        );
                        let text = lines
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("\n");
                        assert_eq!(text.matches("! git status --short").count(), 1, "{text}");
                        if call == Some("shortcut-call") {
                            assert_eq!(text.contains("ctrl+o to expand"), !expanded, "{text}");
                        }
                    }
                }
                for app in [&mut live, &mut resumed] {
                    app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
                }
            }
        }
    }

    #[test]
    fn journal_replay_keeps_external_tool_rows_and_results_identical() {
        for ok in [true, false] {
            let output = (1..=8)
                .map(|line| format!("agent output {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            let events = [
                event(RuntimeEvent::ExternalToolInvoked {
                    id: "external-bash".to_owned(),
                    name: "Bash".to_owned(),
                    detail: serde_json::json!({"command": "echo hello"}),
                }),
                event(RuntimeEvent::ExternalToolCompleted {
                    id: "external-bash".to_owned(),
                    ok,
                    detail: serde_json::Value::String(output),
                }),
            ];
            let bytes = serde_json::to_vec(&events).expect("journal events");
            let replayed: Vec<EventEnvelope> =
                serde_json::from_slice(&bytes).expect("replayable events");
            let mut live = App::new("m", "p");
            let mut replay = App::new("m", "p");
            for event in &events {
                live.apply(event);
            }
            for event in &replayed {
                replay.apply(event);
            }
            assert_eq!(live.transcript.blocks(), replay.transcript.blocks());
            for expanded in [false, true] {
                live.work_details = expanded;
                replay.work_details = expanded;
                for width in [44, 80, 100] {
                    for theme in [Theme::new(), Theme::new().without_color()] {
                        assert_eq!(
                            transcript_lines(&live, theme, width),
                            transcript_lines(&replay, theme, width),
                            "ok={ok}, expanded={expanded}, width={width}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn shell_admission_errors_are_nested_under_the_exact_user_echo() {
        let mut app = App::new("m", "p");
        let echo = app.transcript.push_shell_shortcut("ls -la");
        let _ = app.transcript.finish_shell_shortcut(
            echo,
            None,
            true,
            "shell action failed: requires an idle session",
        );
        let text = transcript_lines(&app, Theme::new().without_color(), 100)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(
            text,
            [
                "! ls -la failed",
                "  ⎿  shell action failed: requires an idle session"
            ]
        );
    }
