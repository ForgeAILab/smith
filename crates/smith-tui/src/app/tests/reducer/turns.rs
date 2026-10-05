use super::*;

#[test]
fn goal_events_reduce_identically_without_duplicating_transcript_history() {
    let update = event(RuntimeEvent::GoalUpdated {
        cause: GoalUpdateCause::TurnCommit,
        sensitivity: PlanSensitivity::Public,
        goal: Some(goal_projection(GoalStatus::Active, Some(17))),
    });
    let mut live = app();
    let mut replayed = app();
    live.apply(&update);
    replayed.apply(&update);

    assert_eq!(live.status.goal, replayed.status.goal);
    assert_eq!(
        live.status.render_goal_footer().as_deref(),
        Some("goal active · 17/100 tok")
    );
    assert!(live.transcript.blocks().is_empty());

    let cleared = event(RuntimeEvent::GoalUpdated {
        cause: GoalUpdateCause::Cleared,
        sensitivity: PlanSensitivity::Public,
        goal: None,
    });
    live.apply(&cleared);
    assert!(live.status.goal.is_none());
}

#[test]
fn streaming_text_lands_in_the_transcript_with_usage_in_the_header() {
    let mut app = app();
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(text_delta("The retry ")));
    app.apply(&event(text_delta("policy")));
    assert_eq!(app.speculative_text(), Some("The retry policy"));
    assert!(app.transcript.is_empty());
    app.apply(&event(commit_output()));
    app.apply(&event(RuntimeEvent::Usage {
        record: UsageRecord {
            source: UsageSource::ProviderAttempt,
            provenance: Provenance::default(),
            delta: UsageDelta::new().with(CounterKind::InputUncached, 12_400),
        },
    }));
    app.apply(&event(RuntimeEvent::TurnCompleted {
        finish: TurnFinish::Completed,
        visible_output: true,
    }));

    assert_eq!(app.transcript.len(), 1);
    assert_eq!(
        app.transcript.blocks()[0],
        Block::Assistant {
            text: "The retry policy".into(),
            open: false
        }
    );
    assert_eq!(app.turn_summary.as_deref(), Some("Worked"));
    assert!(!app.transcript.blocks().iter().any(
        |block| matches!(block, Block::Notice { kind: source, .. } if source.label() == "work")
    ));
    assert_eq!(app.status.context.render(), "12.4k");
    assert_eq!(app.status.activity, Activity::Idle);
}

#[test]
fn an_interrupted_turn_says_so() {
    let mut app = app();
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(text_delta("partial")));
    app.apply(&event(discard_output()));
    app.apply(&event(RuntimeEvent::TurnCompleted {
        finish: TurnFinish::Cancelled {
            reason: CancelReason::UserRequested,
        },
        visible_output: true,
    }));

    assert_eq!(app.status.activity, Activity::Idle);
    assert!(matches!(
        app.transcript.blocks().last(),
        Some(Block::Notice { .. })
    ));
}

#[test]
fn a_completed_turn_uses_canonical_duration_and_clears_live_timing() {
    let mut app = app();
    app.apply(&event_at(Timestamp(1_000), RuntimeEvent::TurnStarted));
    app.live_turn.turn_started_at = Instant::now().checked_sub(Duration::from_secs(65));
    assert!(
        app.turn_elapsed()
            .is_some_and(|elapsed| elapsed.as_secs() >= 65)
    );

    app.apply(&event_at(
        Timestamp(66_000),
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));

    assert!(app.turn_elapsed().is_none());
    assert_eq!(app.live_turn.turn_started_timestamp, None);
    assert!(app.transcript.is_empty());
    assert_eq!(app.turn_summary.as_deref(), Some("Worked for 1m 05s"));
}

#[test]
fn output_flow_belongs_to_the_active_root_turn_and_resets_at_the_next_start() {
    let mut app = app();
    app.apply(&turn_event("turn-1", RuntimeEvent::TurnStarted));
    app.apply(&turn_event(
        "turn-1",
        usage_event(UsageDelta::new().with(CounterKind::InputUncached, 100)),
    ));
    assert_eq!(app.turn_usage.output, crate::status::TokenCount::UNKNOWN);
    app.apply(&turn_event(
        "turn-1",
        usage_event(UsageDelta::new().with(CounterKind::Output, 1_200)),
    ));
    app.apply(&turn_event(
        "another-turn",
        usage_event(UsageDelta::new().with(CounterKind::Output, 900)),
    ));
    app.apply_child(
        "child-1",
        &turn_event(
            "child-turn",
            usage_event(UsageDelta::new().with(CounterKind::Output, 500)),
        ),
    );
    assert_eq!(
        app.turn_usage.output,
        crate::status::TokenCount::reported(1_200)
    );
    app.apply(&turn_event(
        "turn-1",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: false,
        },
    ));
    app.apply(&turn_event(
        "turn-1",
        usage_event(UsageDelta::new().with(CounterKind::Output, 500)),
    ));
    assert_eq!(
        app.turn_usage.output,
        crate::status::TokenCount::reported(1_200)
    );
    app.apply(&turn_event("turn-2", RuntimeEvent::TurnStarted));
    assert_eq!(app.turn_usage.output, crate::status::TokenCount::UNKNOWN);
    assert_eq!(app.visible_turn_summary(), None);
}

#[test]
fn success_summaries_never_move_to_a_later_turn_or_survive_a_non_success_terminal() {
    let mut app = app();
    app.apply(&event_at(Timestamp(1_000), RuntimeEvent::TurnStarted));
    app.transcript.push_text_delta("First answer.");
    app.apply(&event_at(
        Timestamp(1_000),
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));
    assert_eq!(app.visible_turn_summary(), Some("Worked for <1ms"));

    app.transcript
        .push_notice(NoticeKind::Monitor, "a later block");
    assert_eq!(app.visible_turn_summary(), None);
    app.apply(&event(RuntimeEvent::TurnStarted));
    assert_eq!(app.turn_summary, None);
    app.live_turn.turn_started_at = Instant::now().checked_sub(Duration::from_secs(12));
    app.apply(&event(RuntimeEvent::TurnCompleted {
        finish: TurnFinish::Failed,
        visible_output: false,
    }));
    assert_eq!(app.visible_turn_summary(), None);
    assert!(matches!(app.transcript.blocks().last(),
        Some(Block::Notice { kind: source, text }) if source.label() == "turn" && text.starts_with("Failed after 12s")
    ));
}

#[test]
fn a_success_without_visible_text_keeps_an_honest_subsecond_notice() {
    let mut app = app();
    app.apply(&event_at(Timestamp(1_000), RuntimeEvent::TurnStarted));
    app.apply(&event_at(
        Timestamp(1_842),
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: false,
        },
    ));

    assert_eq!(app.status.activity, Activity::Idle);
    assert!(app.turn_elapsed().is_none());
    assert_eq!(app.turn_summary.as_deref(), Some("Worked for 842ms"));
    let rendered = format!("{:?}", app.transcript.blocks());
    assert!(!rendered.contains("reasoning only"), "{rendered}");
}

#[test]
fn unavailable_or_backward_canonical_timing_never_fabricates_duration() {
    for (started, completed) in [
        (Timestamp::ZERO, Timestamp(5_000)),
        (Timestamp(7_000), Timestamp(6_000)),
    ] {
        let mut app = app();
        app.apply(&event_at(started, RuntimeEvent::TurnStarted));
        app.apply(&event_at(
            completed,
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        assert!(app.transcript.is_empty());
        assert_eq!(app.turn_summary.as_deref(), Some("Worked"));
    }
}

#[test]
fn an_error_event_becomes_a_visible_error_block() {
    let mut app = app();
    app.apply(&event(RuntimeEvent::Error {
        error: RuntimeError::config("no provider is configured"),
    }));
    match &app.transcript.blocks()[0] {
        Block::Error { message } => assert!(message.contains("no provider")),
        other => panic!("expected an error block, got {other:?}"),
    }
}

#[test]
fn provider_phase_tracks_the_round_trip_and_clears_at_the_attempt_end() {
    let mut app = app();
    assert!(app.provider_phase().is_none());

    app.apply(&event(RuntimeEvent::ProviderAttemptStarted {
        request: RequestId::new("request-fixture"),
        attempt: AttemptId::new("attempt-fixture"),
        index: 0,
        model: "gpt-5.3".to_owned(),
    }));
    assert_eq!(
        app.provider_phase().map(|(phase, _)| phase),
        Some(ProviderPhase::Sending)
    );

    app.apply(&event(RuntimeEvent::ReasoningDelta {
        request: RequestId::new("request-fixture"),
        attempt: AttemptId::new("attempt-fixture"),
        text: "weighing options".to_owned(),
        redacted: false,
    }));
    assert_eq!(
        app.provider_phase().map(|(phase, _)| phase),
        Some(ProviderPhase::Thinking)
    );

    app.apply(&event(text_delta("the answer")));
    assert_eq!(
        app.provider_phase().map(|(phase, _)| phase),
        Some(ProviderPhase::Responding)
    );

    app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
        attempt: AttemptId::new("attempt-fixture"),
        index: None,
        max_attempts: None,
        finish: agent_runtime_core::provider::FinishReason::Stop,
        retryable: false,
        error: None,
        retry_delay_ms: None,
    }));
    assert!(
        app.provider_phase().is_none(),
        "a finished attempt leaves no live phase to display"
    );
}

#[test]
fn unterminated_speculative_output_is_discarded_at_the_turn_boundary() {
    let mut app = app();
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(text_delta("orphaned draft")));
    assert_eq!(app.speculative_attempt_count(), 1);

    app.apply(&event(RuntimeEvent::TurnCompleted {
        finish: TurnFinish::Failed,
        visible_output: false,
    }));

    assert_eq!(app.speculative_attempt_count(), 0);
    assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Notice { kind: source, text }
                if source.label() == "integrity" && text == "discarded 1 unterminated speculative provider attempt at turn completion"
        )));
    assert!(!format!("{:?}", app.transcript.blocks()).contains("orphaned draft"));
}

#[test]
fn a_harness_turn_renders_the_installed_agent_s_answer_and_its_own_tools() {
    // A harness turn runs on an installed coding agent: no provider
    // attempt is started, so nothing about it arrives through the
    // speculative path the direct loop uses. Before these events were
    // folded, the whole turn rendered as an empty "Worked for 5s".
    let mut app = app();
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(RuntimeEvent::ExternalSessionStarted {
        session: "thread-1".to_owned(),
    }));
    app.apply(&event(RuntimeEvent::ExternalReasoning {
        text: "checking the file".to_owned(),
    }));
    app.apply(&event(RuntimeEvent::ExternalToolInvoked {
        id: "call-1".to_owned(),
        name: "Read".to_owned(),
        detail: serde_json::json!({"file_path": "src/main.rs"}),
    }));
    app.apply(&event(RuntimeEvent::ExternalToolCompleted {
        id: "call-1".to_owned(),
        ok: true,
        detail: serde_json::json!("1\tfn main() {}"),
    }));
    app.apply(&event(RuntimeEvent::ExternalToolInvoked {
        id: "call-2".to_owned(),
        name: "SomeToolThisBuildDoesNotKnow".to_owned(),
        detail: serde_json::json!({"whatever": "shape"}),
    }));
    app.apply(&event(RuntimeEvent::ExternalToolCompleted {
        id: "call-2".to_owned(),
        ok: true,
        detail: serde_json::Value::Null,
    }));
    app.apply(&event(RuntimeEvent::ExternalText {
        text: "the version ".to_owned(),
    }));
    app.apply(&event(RuntimeEvent::ExternalText {
        text: "is 3".to_owned(),
    }));
    app.apply(&event(RuntimeEvent::TurnCompleted {
        finish: TurnFinish::Completed,
        visible_output: true,
    }));

    // Nothing was held back: an installed agent's output arrives already
    // committed, with no attempt to commit or discard it.
    assert!(app.speculative_text().is_none());
    let blocks: Vec<String> = app
        .transcript
        .blocks()
        .iter()
        .map(|block| match block {
            Block::Assistant { text, .. } => format!("assistant: {text}"),
            Block::Reasoning { text, .. } => format!("reasoning: {text}"),
            Block::Tool {
                name,
                status,
                display,
                result_preview,
                enrichment,
                ..
            } => format!(
                "tool: {name} {} {} {} [{}]",
                status.label(),
                display
                    .as_deref()
                    .map(smith_tools::ToolCallDisplay::invocation)
                    .unwrap_or_else(|| "-".to_owned()),
                result_preview.clone().unwrap_or_else(|| "-".to_owned()),
                enrichment.join(" ")
            ),
            Block::Notice { kind: source, text } => format!("{} · {text}", source.label()),
            other => format!("{other:?}"),
        })
        .collect();
    // The agent's reported detail is decoded into the same shape a
    // built-in call renders with, marked as the agent's own; a tool with
    // no reviewed projection keeps the value-free row.
    assert_eq!(
        blocks,
        vec![
            "reasoning: checking the file".to_owned(),
            "tool: Read ok Read(src/main.rs) 1 fn main() {} [agent]".to_owned(),
            "tool: SomeToolThisBuildDoesNotKnow ok - - []".to_owned(),
            "assistant: the version is 3".to_owned(),
        ]
    );
}
