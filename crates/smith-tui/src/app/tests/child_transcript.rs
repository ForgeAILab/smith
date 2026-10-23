fn child_event_at_sequence(sequence: u64, payload: RuntimeEvent) -> EventEnvelope {
    let mut envelope = event(payload);
    envelope.seq = sequence;
    envelope.id = EventId::new(format!("child-event-{sequence}"));
    envelope
}

#[test]
fn canonical_child_history_matches_the_root_conversation_and_rejects_overlap() {
    use agent_runtime_core::content::{Message, ToolCall, ToolResultBlock};

    let history = vec![
        Message::system("internal instructions"),
        Message::user("Review the retry policy"),
        Message::assistant(vec![
            ContentPart::text("I found the policy."),
            ContentPart::ToolCall(ToolCall {
                id: ToolCallId::new("read-policy"),
                name: "read".to_owned(),
                arguments: serde_json::json!({"path": "retry.rs"}),
            }),
        ]),
        Message::tool_result(ToolResultBlock {
            call_id: ToolCallId::new("read-policy"),
            name: "read".to_owned(),
            content: vec![ContentPart::text("The retained tool result")],
            is_error: false,
        }),
    ];
    let mut app = app();
    app.transcript.replace_from_history(&history);
    app.restore_child_transcript("child", &history, 10, &[]);
    app.set_child_tool_result_preview("child", "read-policy", "The retained tool result");
    app.transcript
        .set_tool_result_preview("read-policy", "The retained tool result");
    assert_eq!(app.child_blocks("child"), app.transcript.blocks());
    assert!(app.child_history_loaded("child"));
    assert_eq!(app.child_blocks("child").len(), 3);

    app.apply_child(
        "child",
        &child_event_at_sequence(
            9,
            RuntimeEvent::ExternalText {
                text: "overlapping old text".to_owned(),
            },
        ),
    );
    assert_eq!(app.child_blocks("child"), app.transcript.blocks());
    let tail = child_event_at_sequence(
        10,
        RuntimeEvent::ExternalText {
            text: "New live reply".to_owned(),
        },
    );
    app.apply_child("child", &tail);
    app.apply_child("child", &tail);
    assert_eq!(app.child_blocks("child").len(), 4);
    assert!(matches!(app.child_blocks("child").last(),
            Some(Block::Assistant { text, .. }) if text == "New live reply"));
}

#[test]
fn pending_provider_seed_and_live_tail_commit_once_across_a_history_boundary() {
    use agent_runtime_core::content::Message;

    let mut app = app();
    app.restore_child_transcript("child", &[Message::user("Investigate")], 6, &[]);
    app.restore_child_pending_event("child", &child_event_at_sequence(4, text_delta("Before ")));
    assert_eq!(app.child_speculative_text("child"), Some("Before "));
    app.apply_child("child", &child_event_at_sequence(4, text_delta("Before ")));
    app.apply_child("child", &child_event_at_sequence(6, text_delta("after")));
    app.apply_child("child", &child_event_at_sequence(7, commit_output()));
    assert_eq!(app.child_speculative_text("child"), None);
    assert!(matches!(app.child_blocks("child").last(),
            Some(Block::Assistant { text, .. }) if text == "Before after"));

    let history = [
        Message::user("Investigate"),
        Message::assistant(vec![ContentPart::text("Before after")]),
    ];
    app.restore_child_transcript("child", &history, 8, &[]);
    app.apply_child("child", &child_event_at_sequence(7, commit_output()));
    assert_eq!(app.child_blocks("child").len(), 2);
    assert!(
        !app.child_blocks("child")
            .iter()
            .any(|block| matches!(block, Block::Error { .. }))
    );
}

#[test]
fn refreshing_canonical_history_retains_provider_web_activity_once() {
    use agent_runtime_core::content::Message;

    let history = [Message::user("Search the docs")];
    let search = child_event_at_sequence(
        3,
        RuntimeEvent::HostedSearch {
            action: None,
            query: Some("retry documentation".to_owned()),
            sources: Vec::new(),
        },
    );
    let mut app = app();
    app.restore_child_transcript("child", &history, 4, &[]);
    app.restore_child_pending_event("child", &search);
    for boundary in [5, 8] {
        app.restore_child_transcript("child", &history, boundary, &[]);
        app.restore_child_pending_event("child", &search);
    }
    let history = [
        Message::user("Search the docs"),
        Message::assistant(vec![ContentPart::text("The documentation confirms it.")]),
    ];
    app.restore_child_transcript("child", &history, 10, &[]);
    let blocks = app.child_blocks("child");
    assert_eq!(blocks.len(), 3);
    assert!(matches!(&blocks[0], Block::User { .. }));
    assert!(
        matches!(&blocks[1], Block::HostedSearch { query, .. } if query == "retry documentation")
    );
    assert!(matches!(&blocks[2], Block::Assistant { .. }));
}

#[test]
fn retained_native_tool_observations_recover_between_task_and_final_reply() {
    use agent_runtime_core::content::Message;

    let history = [
        Message::user("Inspect the parser"),
        Message::assistant(vec![ContentPart::text("The parser is correct.")]),
    ];
    let observations = vec![
        (
            1,
            child_event_at_sequence(
                3,
                RuntimeEvent::ExternalToolInvoked {
                    id: "external-call".to_owned(),
                    name: "Read".to_owned(),
                    detail: serde_json::json!({"file_path": "parser.rs"}),
                },
            ),
        ),
        (
            1,
            child_event_at_sequence(
                4,
                RuntimeEvent::ExternalToolCompleted {
                    id: "external-call".to_owned(),
                    ok: true,
                    detail: serde_json::json!({"text": "Native tool result"}),
                },
            ),
        ),
    ];
    let mut app = app();
    app.restore_child_transcript_with_observations("child", &history, 8, &[], &observations);
    app.restore_child_transcript_with_observations("child", &history, 8, &[], &observations);
    let blocks = app.child_blocks("child");
    assert_eq!(blocks.len(), 3);
    assert!(matches!(&blocks[0], Block::User { text } if text == "Inspect the parser"));
    assert!(
        matches!(&blocks[1], Block::Tool { call_id, status: ToolStatus::Ok, result_preview: Some(result), .. }
        if call_id == "external-call" && result == "Native tool result")
    );
    assert!(
        matches!(&blocks[2], Block::Assistant { text, .. } if text == "The parser is correct.")
    );
    app.apply_child("child", &observations[0].1);
    app.apply_child("child", &observations[1].1);
    assert_eq!(app.child_blocks("child").len(), 3);
}

#[test]
fn retained_hosted_search_and_pending_seed_share_one_row() {
    use agent_runtime_core::content::Message;

    let search = child_event_at_sequence(
        3,
        RuntimeEvent::HostedSearch {
            action: None,
            query: Some("parser documentation".to_owned()),
            sources: Vec::new(),
        },
    );
    let mut app = app();
    app.restore_child_transcript_with_observations(
        "child",
        &[Message::user("Find docs")],
        4,
        &[],
        &[(1, search.clone())],
    );
    app.restore_child_pending_event("child", &search);
    assert_eq!(
        app.child_blocks("child")
            .iter()
            .filter(|block| matches!(block, Block::HostedSearch { .. }))
            .count(),
        1
    );
}

#[test]
fn history_refresh_does_not_drop_unseen_usage_or_count_replayed_usage_twice() {
    let mut app = app();
    app.restore_child_transcript("child", &[], 20, &[]);
    let usage = child_event_at_sequence(
        15,
        RuntimeEvent::Usage {
            record: UsageRecord {
                source: UsageSource::ProviderAttempt,
                delta: UsageDelta::new().with(CounterKind::InputUncached, 100),
                provenance: Provenance::default(),
            },
        },
    );
    app.apply_child("child", &usage);
    app.apply_child("child", &usage);
    assert_eq!(
        app.session_usage()
            .delegated_totals
            .get(&CounterKind::InputUncached),
        Some(&100)
    );
}

#[test]
fn child_inspection_restores_root_and_child_scroll_without_touching_the_draft() {
    let mut app = app();
    app.composer.replace("Root draft");
    app.following = false;
    app.scroll_limit = 100;
    app.scroll_back = 35;
    app.scroll_to_block = Some(4);
    app.result_scroll_revision = Some(9);
    app.inspect_child("child");
    assert_eq!(app.scroll_back, 0);
    app.scroll_limit = 60;
    app.scroll_up(20);
    app.apply_child(
        "child",
        &event(RuntimeEvent::ExternalText {
            text: "More output".to_owned(),
        }),
    );
    assert!(!app.following, "new child output respects manual scrolling");
    assert_eq!(app.scroll_back, 20);
    app.leave_child_inspection();
    assert_eq!(app.composer.text(), "Root draft");
    assert!(!app.following);
    assert_eq!((app.scroll_limit, app.scroll_back), (100, 35));
    assert_eq!(app.scroll_to_block, Some(4));
    assert_eq!(app.result_scroll_revision, Some(9));
    app.inspect_child("child");
    assert_eq!((app.scroll_limit, app.scroll_back), (60, 20));
    assert!(!app.following);
    app.replace_children(std::iter::empty());
    assert_eq!(app.inspected_child, None);
    assert_eq!((app.scroll_limit, app.scroll_back), (100, 35));
}

#[test]
fn child_progress_age_uses_observed_events_and_silence_keeps_the_same_fact() {
    let mut app = app();
    let start = std::time::Instant::now();
    app.restore_child_transcript("child", &[], 3, &[]);
    assert_eq!(app.child_progress_age_at("child", start), None);
    app.apply_child_at(
        "child",
        &child_event_at_sequence(
            3,
            RuntimeEvent::ProviderAttemptStarted {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                index: 0,
                model: "provider-model".to_owned(),
            },
        ),
        start,
    );
    let later = start + Duration::from_secs(90);
    assert_eq!(
        app.child_progress_age_at("child", later),
        Some(Duration::from_secs(90))
    );
    assert_eq!(
        app.child_last_activity("child"),
        Some("provider request started")
    );
    app.apply_child_at(
        "child",
        &child_event_at_sequence(4, tool_requested("read", "read")),
        later,
    );
    assert_eq!(
        app.child_last_activity("child"),
        Some("tool requested read")
    );
    assert_eq!(
        app.child_progress_age_at("child", later + Duration::from_secs(2)),
        Some(Duration::from_secs(2))
    );
}

#[test]
fn child_panel_tracks_provider_progress_after_a_reviewed_tool_and_pending_seed() {
    use agent_runtime_core::content::Message;

    let mut app = app();
    app.restore_child("child", ChildState::Running, None);
    app.apply_child(
        "child",
        &child_event_at_sequence(1, tool_requested("read-call", "read")),
    );
    let display =
        smith_tools::project_tool_call_display("read", &serde_json::json!({"path": "retry.rs"}))
            .expect("reviewed read display");
    app.set_child_tool_display("child", "read-call", display.clone());
    app.apply_child(
        "child",
        &child_event_at_sequence(2, tool_completed("read-call", "read", false)),
    );
    let reviewed = format!("ok {}", display.invocation());
    assert_eq!(
        app.children["child"].detail.as_deref(),
        Some(reviewed.as_str())
    );

    let started = child_event_at_sequence(
        3,
        RuntimeEvent::ProviderAttemptStarted {
            request: RequestId::new("request-fixture"),
            attempt: AttemptId::new("attempt-fixture"),
            index: 0,
            model: "model".to_owned(),
        },
    );
    app.apply_child("child", &started);
    assert_eq!(
        app.children["child"].detail.as_deref(),
        Some("provider request started")
    );
    app.set_child_tool_display("child", "read-call", display);
    assert_eq!(
        app.children["child"].detail.as_deref(),
        Some("provider request started"),
        "late enrichment must not replace newer provider activity with the old completed tool"
    );

    app.restore_child_transcript("child", &[Message::user("Keep reviewing")], 4, &[]);
    app.children.get_mut("child").unwrap().detail = Some(reviewed);
    app.restore_child_pending_event("child", &started);
    assert_eq!(
        app.children["child"].detail.as_deref(),
        Some("provider request started")
    );
    app.apply_child(
        "child",
        &child_event_at_sequence(4, text_delta("New output")),
    );
    assert_eq!(
        app.children["child"].detail.as_deref(),
        Some("receiving assistant output")
    );
    app.apply_child(
        "child",
        &child_event_at_sequence(
            5,
            RuntimeEvent::ReasoningDelta {
                request: RequestId::new("request-fixture"),
                attempt: AttemptId::new("attempt-fixture"),
                text: "Provider-visible reasoning".to_owned(),
                redacted: false,
            },
        ),
    );
    assert_eq!(
        app.children["child"].detail.as_deref(),
        Some("receiving provider reasoning")
    );
    app.apply_child(
        "child",
        &child_event_at_sequence(
            6,
            RuntimeEvent::HostedSearch {
                action: None,
                query: Some("retry docs".to_owned()),
                sources: Vec::new(),
            },
        ),
    );
    assert_eq!(
        app.children["child"].detail.as_deref(),
        Some("provider web activity")
    );
    app.restore_child(
        "child",
        ChildState::Completed,
        Some("Finished report".to_owned()),
    );
    app.restore_child_pending_event("child", &started);
    assert_eq!(
        app.children["child"].detail.as_deref(),
        Some("Finished report")
    );
}

#[test]
fn unavailable_child_history_preserves_previously_read_messages() {
    use agent_runtime_core::content::Message;

    let mut app = app();
    app.restore_child_transcript("child", &[Message::user("Retained task")], 3, &[]);
    app.set_child_history_unavailable("child", "child store cannot be read");
    assert_eq!(
        app.child_history_unavailable("child"),
        Some("child store cannot be read")
    );
    assert!(matches!(app.child_blocks("child"), [Block::User { text }] if text == "Retained task"));
    app.restore_child_transcript("child", &[Message::user("Retained task")], 3, &[]);
    assert_eq!(app.child_history_unavailable("child"), None);
}
