use super::*;

#[test]
fn failed_retry_remains_in_billed_denominator_and_rebilling() {
    let mut projection = CacheProjection::default();
    projection.apply(&usage(1, "turn", 100, 0, true));
    projection.apply(&usage(2, "turn", 100, 0, false));
    for (seq, attempt) in [(3, "retry"), (4, "attempt")] {
        projection.apply(&envelope(
            seq,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new(attempt),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::MissObserved,
                expected_read_tokens: Some(100),
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(100),
                confidence: EstimationConfidence::Exact,
            },
        ));
    }
    projection.apply(&envelope(
        5,
        "turn",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Failed,
            visible_output: true,
        },
    ));
    let summary = projection.latest_completed().expect("summary");
    assert_eq!(summary.cache_read_percent, Some(0));
    assert_eq!(summary.miss_count, 2);
    assert_eq!(summary.rebilled_tokens, 200);
}

#[test]
fn a_pre_response_retry_does_not_erase_later_cache_evidence() {
    let mut projection = CacheProjection::default();
    for (seq, attempt, index) in [(1, "transport", 0), (2, "served", 1)] {
        projection.apply(&envelope(
            seq,
            "turn",
            RuntimeEvent::ProviderAttemptStarted {
                request: RequestId::new("request"),
                attempt: AttemptId::new(attempt),
                index,
                model: "model".to_owned(),
            },
        ));
    }
    projection.apply(&envelope(
        3,
        "turn",
        RuntimeEvent::Usage {
            record: UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance {
                    request: Some(RequestId::new("request")),
                    attempt: Some(AttemptId::new("served")),
                    ..Provenance::default()
                },
                delta: UsageDelta::new()
                    .with(CounterKind::InputUncached, 20)
                    .with(CounterKind::InputCached, 80),
            },
        },
    ));
    projection.apply(&envelope(
        4,
        "turn",
        RuntimeEvent::CacheStateChanged {
            request: RequestId::new("request"),
            attempt: AttemptId::new("served"),
            cache_plan: Fingerprint::of("plan"),
            cache_identity: None,
            state: CacheState::WarmObserved,
            expected_read_tokens: Some(80),
            observed_read_tokens: Some(80),
            observed_write_tokens: None,
            missed_tokens: Some(0),
            confidence: EstimationConfidence::Exact,
        },
    ));
    projection.apply(&envelope(
        5,
        "turn",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));

    let summary = projection.latest_completed().expect("summary");
    assert_eq!(summary.state, CacheVisibilityState::WarmObserved);
    assert_eq!(summary.expected_read_tokens, Some(80));
    assert_eq!(summary.observed_read_tokens, Some(80));
    assert_eq!(summary.confidence, Some(EstimationConfidence::Exact));
    assert_eq!(summary.cache_read_percent, Some(80));
    assert_eq!(projection.session_observed_read(), Some(80));
}

#[test]
fn a_retry_inherits_its_logical_requests_idle_context() {
    let mut projection = CacheProjection::default();
    projection.apply(&envelope(
        1,
        "previous",
        RuntimeEvent::ProviderAttemptStarted {
            request: RequestId::new("previous-request"),
            attempt: AttemptId::new("previous-attempt"),
            index: 0,
            model: "model".to_owned(),
        },
    ));
    for (seq, attempt, index) in [(12, "first-attempt", 0), (13, "retry-attempt", 1)] {
        projection.apply(&envelope(
            seq,
            "turn",
            RuntimeEvent::ProviderAttemptStarted {
                request: RequestId::new("retry-request"),
                attempt: AttemptId::new(attempt),
                index,
                model: "model".to_owned(),
            },
        ));
    }
    projection.apply(&envelope(
        14,
        "turn",
        RuntimeEvent::CacheStateChanged {
            request: RequestId::new("retry-request"),
            attempt: AttemptId::new("retry-attempt"),
            cache_plan: Fingerprint::of("plan"),
            cache_identity: None,
            state: CacheState::MissObserved,
            expected_read_tokens: Some(20_000),
            observed_read_tokens: Some(0),
            observed_write_tokens: None,
            missed_tokens: Some(20_000),
            confidence: EstimationConfidence::Exact,
        },
    ));
    projection.apply(&envelope(
        15,
        "turn",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));

    assert_eq!(
        projection.latest_completed().expect("summary").idle_minutes,
        Some(11)
    );
}

#[test]
fn duplicate_replay_does_not_double_count() {
    let mut projection = CacheProjection::default();
    let events = vec![
        usage(1, "turn", 100, 0, false),
        envelope(
            2,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::MissObserved,
                expected_read_tokens: Some(100),
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(100),
                confidence: EstimationConfidence::Exact,
            },
        ),
        envelope(
            3,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ),
    ];
    projection.replay(events.clone());
    projection.replay(events);
    assert_eq!(projection.session_miss_count(), 1);
    assert_eq!(projection.session_rebilled_tokens(), 100);
}

#[test]
fn internal_turns_do_not_replace_the_latest_root_turn() {
    let mut projection = CacheProjection::default();
    projection.internal_turns.insert("internal".to_owned());
    projection.apply(&usage(1, "internal", 100, 0, false));
    projection.apply(&envelope(
        2,
        "internal",
        RuntimeEvent::CacheStateChanged {
            request: RequestId::new("request"),
            attempt: AttemptId::new("attempt"),
            cache_plan: Fingerprint::of("plan"),
            cache_identity: None,
            state: CacheState::MissObserved,
            expected_read_tokens: Some(100),
            observed_read_tokens: Some(0),
            observed_write_tokens: None,
            missed_tokens: Some(100),
            confidence: EstimationConfidence::Exact,
        },
    ));
    projection.apply(&envelope(
        3,
        "internal",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: false,
        },
    ));
    assert!(projection.latest_completed().is_none());
    assert_eq!(projection.session_miss_count(), 1);

    projection.apply(&usage(4, "root", 100, 100, false));
    projection.apply(&envelope(
        5,
        "root",
        RuntimeEvent::CacheStateChanged {
            request: RequestId::new("request"),
            attempt: AttemptId::new("attempt"),
            cache_plan: Fingerprint::of("plan"),
            cache_identity: None,
            state: CacheState::WarmObserved,
            expected_read_tokens: Some(100),
            observed_read_tokens: Some(100),
            observed_write_tokens: None,
            missed_tokens: Some(0),
            confidence: EstimationConfidence::Exact,
        },
    ));
    projection.apply(&envelope(
        6,
        "root",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));
    let summary = projection.latest_completed().expect("root summary");
    assert_eq!(summary.turn, "root");
    assert_eq!(summary.cache_read_percent, Some(100));
    assert_eq!(projection.session_miss_count(), 1);
}

#[test]
fn idle_context_is_only_rendered_for_one_miss_bearing_request() {
    let mut projection = CacheProjection::default();
    projection.apply(&envelope(
        1,
        "previous",
        RuntimeEvent::ProviderAttemptStarted {
            request: RequestId::new("previous-request"),
            attempt: AttemptId::new("attempt-previous"),
            index: 0,
            model: "model".to_owned(),
        },
    ));
    projection.apply(&envelope(
        2,
        "previous",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));
    projection.apply(&envelope(
        11,
        "one-request",
        RuntimeEvent::ProviderAttemptStarted {
            request: RequestId::new("one-request"),
            attempt: AttemptId::new("attempt-one"),
            index: 0,
            model: "model".to_owned(),
        },
    ));
    projection.apply(&envelope(
        12,
        "one-request",
        RuntimeEvent::CacheStateChanged {
            request: RequestId::new("one-request"),
            attempt: AttemptId::new("attempt-one"),
            cache_plan: Fingerprint::of("plan"),
            cache_identity: None,
            state: CacheState::MissObserved,
            expected_read_tokens: Some(20_000),
            observed_read_tokens: Some(0),
            observed_write_tokens: None,
            missed_tokens: Some(20_000),
            confidence: EstimationConfidence::Exact,
        },
    ));
    projection.apply(&envelope(
        13,
        "one-request",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));
    assert_eq!(
        projection
            .latest_completed()
            .expect("one-request summary")
            .idle_minutes,
        Some(10)
    );

    let mut multiple = CacheProjection::default();
    for (seq, request, attempt) in [
        (1, "first-request", "attempt-first"),
        (2, "second-request", "attempt-second"),
    ] {
        multiple.apply(&envelope(
            seq,
            "multiple",
            RuntimeEvent::ProviderAttemptStarted {
                request: RequestId::new(request),
                attempt: AttemptId::new(attempt),
                index: 0,
                model: "model".to_owned(),
            },
        ));
        multiple.apply(&envelope(
            seq + 2,
            "multiple",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new(request),
                attempt: AttemptId::new(attempt),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::MissObserved,
                expected_read_tokens: Some(20_000),
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(20_000),
                confidence: EstimationConfidence::Exact,
            },
        ));
    }
    multiple.apply(&envelope(
        5,
        "multiple",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));
    assert_eq!(
        multiple
            .latest_completed()
            .expect("multiple summary")
            .idle_minutes,
        None
    );
}
