use super::*;

#[test]
fn live_findings_capability_notices_appear_only_in_expanded_detail() {
    use agent_runtime_core::manifest::ActivatedCapability;
    use agent_runtime_registry::{RegistryId, RegistryRevision};

    let activation = event(RuntimeEvent::CapabilitiesActivated {
        epoch: 2,
        activation: vec![ActivatedCapability::new(
            RegistryId::tool("read"),
            RegistryRevision::new("read-1"),
        )],
    });
    for width in [42, 100] {
        let mut app = App::new("model", "project");
        app.apply(&activation);
        assert!(transcript_lines(&app, Theme::new(), width).is_empty());
        app.transcript.push_user("before");
        app.apply(&activation);
        app.transcript.push_text_delta("after");
        app.transcript.close_open();
        let collapsed = transcript_lines(&app, Theme::new(), width);
        assert_eq!(
            collapsed
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["> before", "", "● after"]
        );
        let collapsed_screen = render(&app, width, 24, Theme::new());
        assert!(
            !collapsed_screen.contains("activation epoch"),
            "{collapsed_screen}"
        );

        app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        let expanded = transcript_lines(&app, Theme::new(), width)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            expanded
                .matches("capabilities · activation epoch 2:")
                .count(),
            2,
            "{expanded}"
        );
        let expanded_screen = render(&app, width, 24, Theme::new());
        assert!(
            expanded_screen.contains("activation epoch 2:"),
            "{expanded_screen}"
        );

        app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(transcript_lines(&app, Theme::new(), width), collapsed);
        let collapsed_screen = render(&app, width, 24, Theme::new());
        assert!(
            !collapsed_screen.contains("activation epoch"),
            "{collapsed_screen}"
        );
        assert_eq!(
            app.status.capabilities.activation,
            Some((2, vec!["tool:read".to_owned()]))
        );
    }
}

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
                1 => app
                    .transcript
                    .push_notice(NoticeKind::Monitor, "later notice"),
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
        assert!(
            backoff.contains("Retrying 2/3…"),
            "{width}x{height}: {backoff}"
        );
        assert!(
            backoff.contains("backoff <1s"),
            "{width}x{height}: {backoff}"
        );
        assert!(backoff.contains("↓ ~1.2k"), "{width}x{height}: {backoff}");
        assert!(
            backoff
                .lines()
                .any(|line| line.starts_with("● Retrying 2/3… (")
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
        assert!(
            active.contains("Retrying 2/3…"),
            "{width}x{height}: {active}"
        );
        assert!(active.contains("↑"), "{width}x{height}: {active}");
        assert!(!active.contains("backoff"), "{width}x{height}: {active}");
        assert!(active.contains("↓ ~1.2k"), "{width}x{height}: {active}");
        assert!(
            active
                .lines()
                .any(|line| line.starts_with("● Retrying 2/3… (")
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

    app.transcript
        .push_notice(NoticeKind::NamedMonitor("build".to_owned()), "error[E0433]");
    assert!(render(&app, 74, 24, Theme::new()).contains("monitor:build"));
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
                .filter(|line| !line.is_empty() && !line.starts_with('>') && !line.starts_with('●'))
                .all(|line| line.starts_with("  ")),
            "{text:#?}"
        );
        assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
    }
}
