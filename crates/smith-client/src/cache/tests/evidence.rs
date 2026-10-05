use super::*;

#[test]
fn exact_identity_and_expiry_remain_distinct_in_the_turn_projection() {
    let identity = cache_identity();
    let expected_digest = identity.digest().to_string();
    let mut projection = CacheProjection::default();
    projection.apply(&envelope(
        1,
        "turn",
        RuntimeEvent::CacheStateChanged {
            request: RequestId::new("request"),
            attempt: AttemptId::new("attempt"),
            cache_plan: Fingerprint::of("plan"),
            cache_identity: Some(identity),
            state: CacheState::Expired,
            expected_read_tokens: Some(100),
            observed_read_tokens: Some(0),
            observed_write_tokens: None,
            missed_tokens: Some(100),
            confidence: EstimationConfidence::Exact,
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

    let summary = projection.latest_completed().expect("summary");
    assert_eq!(summary.state, CacheVisibilityState::Expired);
    assert_eq!(
        summary.cache_identity.as_deref(),
        Some(expected_digest.as_str())
    );
}

#[test]
fn canonical_operation_lifecycle_and_guarantee_project_once() {
    let identity = cache_identity();
    let operation = CacheOperationId::new("cache-operation");
    let request = RequestId::new("request");
    let attempt = AttemptId::new("attempt");
    let mut projection = CacheProjection::default();

    projection.apply(&envelope(
        1,
        "turn",
        RuntimeEvent::CacheOperationPrepared {
            operation: operation.clone(),
            request: Some(request.clone()),
            identity: identity.clone(),
            purpose: ProviderAttemptPurpose::CacheKeepalive,
        },
    ));
    let started = envelope(
        2,
        "turn",
        RuntimeEvent::CacheOperationStarted {
            operation: operation.clone(),
            request: Some(request.clone()),
            attempt: Some(attempt.clone()),
            identity: identity.clone(),
            purpose: ProviderAttemptPurpose::CacheKeepalive,
        },
    );
    projection.apply(&started);
    projection.apply(&started);
    projection.apply(&envelope(
        3,
        "turn",
        RuntimeEvent::CacheAvailabilityEvidenceRecorded {
            evidence: CacheAvailabilityEvidence::stream(
                identity.clone(),
                request.clone(),
                attempt.clone(),
                0,
                Some(10),
                Some(0),
            )
            .with_guaranteed_until(Timestamp(9_000)),
        },
    ));
    projection.apply(&envelope(
        4,
        "turn",
        RuntimeEvent::CacheOperationCompleted {
            operation,
            request: Some(request),
            attempt: Some(attempt),
            identity: identity.clone(),
            purpose: ProviderAttemptPurpose::CacheKeepalive,
            outcome: CacheOperationOutcome::Completed,
            reason: None,
            metrics: BTreeMap::from([("latency_ms".to_owned(), 12)]),
        },
    ));

    let lifecycle = projection.lifecycle();
    assert_eq!(lifecycle.maintenance_calls_used, 1);
    assert_eq!(lifecycle.guaranteed_until_ms, Some(9_000));
    assert_eq!(
        lifecycle.cache_identity.as_deref(),
        Some(identity.digest().as_str())
    );
    let operation = lifecycle.last_operation.as_ref().expect("operation");
    assert_eq!(operation.disposition, CacheOperationDisposition::Completed);
    assert_eq!(operation.outcome, Some(CacheOperationOutcome::Completed));
    assert_eq!(operation.metrics.get("latency_ms"), Some(&12));
}

#[test]
fn explicit_zero_is_zero_not_unknown() {
    let mut projection = CacheProjection::default();
    projection.apply(&envelope(
        1,
        "turn",
        RuntimeEvent::ProviderAttemptStarted {
            request: RequestId::new("request"),
            attempt: AttemptId::new("attempt"),
            index: 0,
            model: "model".to_owned(),
        },
    ));
    projection.apply(&usage(2, "turn", 100, 0, false));
    projection.apply(&envelope(
        3,
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
        4,
        "turn",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));
    let summary = projection.latest_completed().expect("summary");
    assert_eq!(summary.cache_read_percent, Some(0));
    assert_eq!(summary.missed_tokens, Some(100));
    assert_eq!(summary.rebilled_tokens, 100);
}

#[test]
fn first_eligible_zero_does_not_become_a_miss() {
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
            state: CacheState::Eligible,
            expected_read_tokens: None,
            observed_read_tokens: Some(0),
            observed_write_tokens: None,
            missed_tokens: Some(0),
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
    assert_eq!(summary.state, CacheVisibilityState::Eligible);
    assert_eq!(summary.cache_read_percent, Some(0));
    assert_eq!(summary.missed_tokens, Some(0));
    assert_eq!(summary.miss_count, 0);
    assert_eq!(summary.rebilled_tokens, 0);
}

#[test]
fn a_full_hit_uses_provider_cached_input_for_ch() {
    let mut projection = CacheProjection::default();
    projection.apply(&usage(1, "turn", 100, 100, false));
    projection.apply(&envelope(
        2,
        "turn",
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
        3,
        "turn",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));
    assert_eq!(
        projection
            .latest_completed()
            .expect("summary")
            .cache_read_percent,
        Some(100)
    );
}

#[test]
fn a_write_only_observation_keeps_read_ch_at_zero() {
    let mut projection = CacheProjection::default();
    projection.apply(&envelope(
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
    projection.apply(&envelope(
        2,
        "turn",
        RuntimeEvent::CacheStateChanged {
            request: RequestId::new("request"),
            attempt: AttemptId::new("attempt"),
            cache_plan: Fingerprint::of("plan"),
            cache_identity: None,
            state: CacheState::WarmObserved,
            expected_read_tokens: None,
            observed_read_tokens: Some(0),
            observed_write_tokens: Some(100),
            missed_tokens: Some(0),
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
    assert_eq!(summary.observed_write_tokens, Some(100));
    assert_eq!(summary.cache_read_percent, Some(0));
}

#[test]
fn switching_identity_suspends_the_prior_root_projection_without_new_misses() {
    let mut projection = CacheProjection::default();
    projection.apply(&envelope(
        1,
        "identity",
        RuntimeEvent::ModelProfileResolved {
            provider: "old-provider".to_owned(),
            model: ModelId::new("old-model"),
            profile: Fingerprint::of("profile-a"),
        },
    ));
    projection.apply(&usage(2, "turn", 100, 0, false));
    projection.apply(&envelope(
        3,
        "turn",
        RuntimeEvent::CacheStateChanged {
            request: RequestId::new("request"),
            attempt: AttemptId::new("attempt"),
            cache_plan: Fingerprint::of("plan-a"),
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
        4,
        "turn",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));
    projection.suspend();
    let suspended = projection.latest_completed().expect("suspended summary");
    assert_eq!(suspended.state, CacheVisibilityState::Suspended);
    assert_eq!(suspended.cache_read_percent, None);
    assert_eq!(suspended.missed_tokens, None);
    assert_eq!(suspended.rebilled_tokens, 0);
    assert_eq!(projection.session_miss_count(), 1);
    assert_eq!(projection.session_rebilled_tokens(), 100);
    assert_eq!(
        projection
            .with_price(
                suspended,
                CachePrice {
                    input: Some(100_000),
                    cache_read: Some(50_000),
                    cache_write: Some(100_000),
                },
            )
            .extra_cost_micro_usd,
        None
    );

    projection.apply(&envelope(
        5,
        "new-turn",
        RuntimeEvent::ModelProfileResolved {
            provider: "new-provider".to_owned(),
            model: ModelId::new("new-model"),
            profile: Fingerprint::of("profile-b"),
        },
    ));
    assert_eq!(
        projection
            .latest_completed()
            .expect("suspended prior summary")
            .state,
        CacheVisibilityState::Suspended
    );
    assert_eq!(projection.session_miss_count(), 1);
}

#[test]
fn absent_observation_is_unknown_and_never_a_miss() {
    let mut projection = CacheProjection::default();
    projection.apply(&usage(1, "turn", 100, 0, false));
    projection.apply(&envelope(
        2,
        "turn",
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ));
    let summary = projection.latest_completed().expect("summary");
    assert_eq!(summary.cache_read_percent, None);
    assert_eq!(summary.missed_tokens, None);
    assert_eq!(summary.state, CacheVisibilityState::Unknown);
}

#[test]
fn canonical_miss_without_usage_keeps_miss_diagnostics_but_ch_is_unknown() {
    let mut projection = CacheProjection::default();
    projection.apply(&envelope(
        1,
        "turn",
        RuntimeEvent::CacheStateChanged {
            request: RequestId::new("request"),
            attempt: AttemptId::new("attempt"),
            cache_plan: Fingerprint::of("plan"),
            cache_identity: None,
            state: CacheState::MissObserved,
            expected_read_tokens: Some(105_000),
            observed_read_tokens: Some(0),
            observed_write_tokens: None,
            missed_tokens: Some(105_000),
            confidence: EstimationConfidence::Exact,
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
    let summary = projection.latest_completed().expect("summary");
    assert_eq!(summary.cache_read_percent, None);
    assert_eq!(summary.missed_tokens, Some(105_000));
    assert_eq!(summary.rebilled_tokens, 105_000);
    assert_eq!(summary.extra_cost_micro_usd, None);
}

#[test]
fn ch_uses_bounded_cached_usage_not_an_unbounded_raw_observation() {
    let mut projection = CacheProjection::default();
    projection.apply(&usage(1, "turn", 100, 20, false));
    projection.apply(&envelope(
        2,
        "turn",
        RuntimeEvent::CacheStateChanged {
            request: RequestId::new("request"),
            attempt: AttemptId::new("attempt"),
            cache_plan: Fingerprint::of("plan"),
            cache_identity: None,
            state: CacheState::WarmObserved,
            expected_read_tokens: Some(20),
            observed_read_tokens: Some(200),
            observed_write_tokens: None,
            missed_tokens: Some(0),
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
    assert_eq!(
        projection
            .latest_completed()
            .expect("summary")
            .cache_read_percent,
        Some(20)
    );
}
