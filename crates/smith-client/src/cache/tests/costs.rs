use super::*;

#[test]
fn known_misses_are_kept_when_another_attributed_attempt_has_unknown_usage() {
    let mut projection = CacheProjection::default();
    projection.apply(&usage(1, "turn", 100, 0, false));
    for (seq, attempt, missed) in [(2, "attempt", 100), (3, "unknown", 200)] {
        projection.apply(&envelope(
            seq,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new(attempt),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::MissObserved,
                expected_read_tokens: Some(missed),
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(missed),
                confidence: EstimationConfidence::Exact,
            },
        ));
    }
    projection.apply(&envelope(
        4,
        "turn",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));

    let summary = projection.latest_completed().expect("summary");
    assert_eq!(summary.cache_read_percent, Some(0));
    assert_eq!(summary.missed_tokens, Some(300));
    assert_eq!(summary.miss_count, 2);
    assert_eq!(summary.rebilled_tokens, 300);
}

#[test]
fn legacy_unattributed_zero_is_preserved_without_creating_cache_diagnostics() {
    let mut projection = CacheProjection::default();
    projection.apply(&envelope(
        1,
        "turn",
        RuntimeEvent::CacheObservation {
            request: None,
            attempt: None,
            cache_plan: None,
            cache_identity: None,
            read_tokens: Some(0),
            write_tokens: None,
        },
    ));
    projection.apply(&envelope(
        2,
        "turn",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));

    assert_eq!(projection.legacy_read(), Some(0));
    assert_eq!(projection.session_observed_read(), Some(0));
    let summary = projection.latest_completed().expect("summary");
    assert_eq!(summary.cache_read_percent, None);
    assert_eq!(summary.missed_tokens, None);
    assert_eq!(summary.miss_count, 0);
}

#[test]
fn missing_rates_are_required_only_for_positive_paid_categories() {
    let mut projection = CacheProjection::default();
    projection.apply(&usage(1, "turn", 100, 0, false));
    projection.apply(&envelope(
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
    ));
    projection.apply(&envelope(
        3,
        "turn",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));
    let summary = projection.latest_completed().expect("summary");
    let priced = projection.with_price(
        summary,
        CachePrice {
            input: Some(100_000),
            cache_read: Some(50_000),
            cache_write: None,
        },
    );
    assert_eq!(priced.extra_cost_micro_usd, Some(5));

    let mut write_projection = CacheProjection::default();
    write_projection.apply(&envelope(
        1,
        "turn",
        RuntimeEvent::Usage {
            record: UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance {
                    request: Some(RequestId::new("request")),
                    attempt: Some(AttemptId::new("attempt")),
                    ..Provenance::default()
                },
                delta: UsageDelta::new().with(CounterKind::CacheWrite, 100),
            },
        },
    ));
    write_projection.apply(&envelope(
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
    ));
    write_projection.apply(&envelope(
        3,
        "turn",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));
    let summary = write_projection.latest_completed().expect("summary");
    assert_eq!(
        write_projection
            .with_price(
                summary,
                CachePrice {
                    input: Some(100_000),
                    cache_read: Some(50_000),
                    cache_write: None,
                },
            )
            .extra_cost_micro_usd,
        None
    );
}
