use super::*;

#[test]
fn delegated_usage_from_a_live_child_stream_stays_separate_from_the_root_counters() {
    let mut app = app();
    app.apply(&event(RuntimeEvent::Usage {
        record: UsageRecord {
            source: UsageSource::ProviderAttempt,
            provenance: Provenance::default(),
            delta: UsageDelta::new()
                .with(CounterKind::InputUncached, 1_000)
                .with(CounterKind::Output, 50),
        },
    }));

    app.apply_child(
        "child-1",
        &event(usage_event(
            UsageDelta::new()
                .with(CounterKind::InputUncached, 300)
                .with(CounterKind::Output, 20),
        )),
    );
    app.apply_child(
        "child-2",
        &event(usage_event(
            UsageDelta::new()
                .with(CounterKind::InputUncached, 100)
                .with(CounterKind::Output, 5),
        )),
    );
    // A second qualifying record from the same child must not
    // double-count it as a contributor.
    app.apply_child(
        "child-1",
        &event(usage_event(
            UsageDelta::new()
                .with(CounterKind::InputUncached, 50)
                .with(CounterKind::Output, 15),
        )),
    );

    let usage = app.session_usage();
    assert_eq!(usage.totals[&CounterKind::InputUncached], 1_000);
    assert_eq!(usage.totals[&CounterKind::Output], 50);
    assert_eq!(usage.delegated_totals[&CounterKind::InputUncached], 450);
    assert_eq!(usage.delegated_totals[&CounterKind::Output], 40);
    assert_eq!(
        usage.delegated_contributors, 2,
        "each contributing child is counted once"
    );
    assert_eq!(usage.total_tokens(), 1_050, "total_tokens stays root-only");
    assert_eq!(
        usage.merged_total_tokens(),
        1_540,
        "the merged figure is the explicit combined total"
    );
}

#[test]
fn synthetic_child_usage_is_session_spend_not_delegated_turn_usage() {
    let mut app = app();
    app.apply_child(
        "child-1",
        &event(RuntimeEvent::Usage {
            record: UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance {
                    attempt_purpose: Some(ProviderAttemptPurpose::CacheHandoffCheckpoint),
                    ..Provenance::default()
                },
                delta: UsageDelta::new()
                    .with(CounterKind::InputCached, 700)
                    .with(CounterKind::Output, 20),
            },
        }),
    );

    let usage = app.session_usage();
    assert_eq!(usage.turns, 0);
    assert!(usage.totals.is_empty());
    assert!(usage.delegated_totals.is_empty());
    assert_eq!(usage.delegated_contributors, 0);
    assert_eq!(usage.synthetic_totals[&CounterKind::InputCached], 700);
    assert_eq!(
        usage.synthetic_by_purpose[&ProviderAttemptPurpose::CacheHandoffCheckpoint]
            [&CounterKind::Output],
        20
    );
}

#[test]
fn an_output_only_delegated_record_reports_no_usable_evidence_and_no_contributor() {
    // Mirrors `Status::record_usage`'s own rule: without an input
    // counter there is nothing usable to attribute to the session, so
    // the reporting child is not even counted as a contributor.
    let mut app = app();
    app.apply_child(
        "child-1",
        &event(usage_event(UsageDelta::new().with(CounterKind::Output, 40))),
    );

    let usage = app.session_usage();
    assert!(usage.delegated_totals.is_empty());
    assert_eq!(usage.delegated_contributors, 0);
}

#[test]
fn a_dormant_recovered_child_contributes_no_delegated_usage() {
    // A resumed session recovers a durable child whose work happened in
    // an earlier process: it gets a panel row via `ChildProgress`, but
    // `App::apply_child` — the only path that can ever feed the
    // delegated totals — is never called for it, because it has no live
    // stream in this process.
    let mut app = app();
    app.apply(&event(RuntimeEvent::ChildProgress {
        child: ChildId::new("child-recovered"),
        phase: ChildPhase::Recovered {
            child_session: SessionId::new("child-session-recovered"),
            state: ChildRecoveryState::Idle,
            resumable: false,
        },
    }));

    assert!(app.children.contains_key("child-recovered"));
    let usage = app.session_usage();
    assert!(usage.delegated_totals.is_empty());
    assert_eq!(usage.delegated_contributors, 0);
    assert!(usage.is_empty(), "nothing was ever observed this process");
}

#[test]
fn session_usage_counts_root_prompts_instead_of_tool_provider_attempts() {
    let mut resumed = app();
    resumed.status.restore_turn_count(2);
    for (turn, expected) in [("turn-3", 3), ("turn-4", 4)] {
        resumed.apply(&turn_event(turn, RuntimeEvent::TurnStarted));
        for index in 0..4 {
            resumed.apply(&turn_event(
                turn,
                usage_event(UsageDelta::new().with(CounterKind::InputUncached, 100)),
            ));
            if index < 3 {
                let call = format!("call-{turn}-{index}");
                resumed.apply(&turn_event(turn, tool_requested(&call, "read")));
                resumed.apply(&turn_event(turn, tool_completed(&call, "read", false)));
            }
        }
        resumed.apply(&turn_event(
            turn,
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        let usage = resumed.session_usage();
        assert_eq!(usage.turns, expected);
        assert!(
            usage
                .render()
                .expect("usage")
                .starts_with(&format!("{expected} turns ·"))
        );
    }
    let mut fresh = app();
    fresh.apply(&turn_event("turn-1", RuntimeEvent::TurnStarted));
    for _ in 0..4 {
        fresh.apply(&turn_event(
            "turn-1",
            usage_event(UsageDelta::new().with(CounterKind::InputUncached, 100)),
        ));
    }
    assert!(
        fresh
            .session_usage()
            .render()
            .expect("usage")
            .starts_with("1 turn ·")
    );
    fresh.apply(&turn_event("turn-2", RuntimeEvent::TurnStarted));
    assert!(
        fresh
            .session_usage()
            .render()
            .expect("usage")
            .starts_with("2 turns ·")
    );
    fresh.apply(&turn_event(
        "internal",
        RuntimeEvent::InternalTurnStarted {
            source: agent_runtime_core::content::InternalTurnSource {
                kind: "goal".into(),
                id: "goal".into(),
                revision: agent_runtime_registry::RegistryRevision::new("test"),
                sensitivity: agent_runtime_core::content::InternalTurnSensitivity::Public,
                goal: None,
            },
        },
    ));
    assert_eq!(fresh.session_usage().turns, 2);
}
