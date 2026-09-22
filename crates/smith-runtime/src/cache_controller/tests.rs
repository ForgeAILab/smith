//! Existing focused tests for this module family.

use super::*;
use agent_runtime_core::event::TurnFinish;
use agent_runtime_core::ids::{EventId, SessionId, TurnId};
use agent_runtime_core::provider::CacheIdentity;
use smith_config::resolve::{ResolvedCachePolicy, Source, Sourced};

fn envelope(seq: u64, payload: RuntimeEvent) -> EventEnvelope {
    EventEnvelope::new(
        seq,
        EventId::new(format!("event-{seq}")),
        SessionId::new("session"),
        Some(TurnId::new("turn")),
        Timestamp(seq),
        payload,
    )
}

fn test_identity() -> CacheIdentity {
    CacheIdentity::builder(
        "provider",
        agent_runtime_core::provider::ModelId::new("model"),
        agent_runtime_core::cache::CacheEndpointIdentity::from_opaque(
            "endpoint",
            RegistryRevision::new("endpoint-r1"),
        ),
        RegistryRevision::new("adapter-r1"),
        Fingerprint::of("profile"),
    )
    .build()
}

fn test_config(policy: CacheMaintenancePolicy) -> CacheControllerConfig {
    CacheControllerConfig {
        policy,
        synthetic_spend: SyntheticCacheSpendAuthority::Deny,
        contract: ProviderCacheContract::default(),
        model_input_limit: 8_192,
        model_output_limit: 1_024,
        provider: "provider".to_owned(),
        model: "model".to_owned(),
        semantic_summary_provider: Some("summary-provider".to_owned()),
        semantic_summary_model: Some("summary-model".to_owned()),
        endpoint_identity: None,
        profile_identity: Fingerprint::of("profile"),
        requested_maintenance: CacheMaintenanceMode::Observe,
        narrowing_reason: None,
    }
}

fn sourced<T>(value: T) -> Sourced<T> {
    Sourced::new(value, Source::built_in("cache-controller-test"))
}

fn resolved_cache_policy() -> ResolvedCachePolicy {
    ResolvedCachePolicy {
        requested_maintenance: sourced(ConfigMaintenanceMode::Adaptive),
        effective_maintenance: sourced(ConfigMaintenanceMode::Adaptive),
        narrowing_reason: None,
        inactivity_limit_ms: sourced(1_000),
        max_hold_while_child_ms: sourced(1_000),
        max_maintenance_calls: sourced(1),
        max_maintenance_input_tokens: sourced(0),
        max_maintenance_output_tokens: sourced(256),
        maintenance_deadline_ms: sourced(1_000),
        keepalive_margin_ms: sourced(0),
        keepalive_jitter_percent: sourced(0),
        handoff_checkpoint: sourced(true),
        idle_compaction: sourced(true),
        resume_capsule: sourced(true),
    }
}

#[test]
fn missing_resume_capsule_narrows_handoff_and_idle_compaction() {
    let config = CacheControllerConfig::from_resolved(
        &resolved_cache_policy(),
        CacheControllerResolvedInputs {
            synthetic_spend: SyntheticCacheSpendAuthority::Allow,
            contract: ProviderCacheContract::default(),
            model_input_limit: 8_192,
            model_output_limit: 1_024,
            provider: "provider".to_owned(),
            model: "model".to_owned(),
            endpoint_identity: None,
            profile_identity: Fingerprint::of("profile"),
            semantic_summary_provider: None,
            semantic_summary_model: None,
            attempt_marker_available: false,
        },
    )
    .expect("test policy is valid");

    assert!(!config.policy.handoff_checkpoint);
    assert!(!config.policy.idle_compaction);
    let reason = config.narrowing_reason.expect("narrowing reason");
    assert!(reason.contains("handoff checkpoint disabled"));
    assert!(reason.contains("idle compaction disabled"));
}

#[test]
fn idle_timer_is_root_activity_only_and_stops_at_unsafe_boundaries() {
    let policy = CacheMaintenancePolicy {
        inactivity_limit_ms: 1_000,
        ..CacheMaintenancePolicy::default()
    };
    let config = test_config(policy);
    let mut state = ControllerState {
        last_meaningful_activity_at: Some(Timestamp(100)),
        ..ControllerState::default()
    };
    state.snapshot.idle_compaction.interval_id = Some("root-turn:1".to_owned());

    assert_eq!(
        idle_compaction_wait_ms(&config, &state, Timestamp(1_099)),
        Some(1)
    );
    state.parent_turn_active = true;
    assert_eq!(
        idle_compaction_wait_ms(&config, &state, Timestamp(1_099)),
        None
    );
    state.parent_turn_active = false;
    state.snapshot.idle_compaction.attempted = true;
    assert_eq!(
        idle_compaction_wait_ms(&config, &state, Timestamp(1_099)),
        None
    );
}

#[test]
fn shutdown_and_optional_projection_share_one_atomic_admission_order() {
    let state = Arc::new(Mutex::new(ControllerState::default()));
    let admitted = OptionalProjectionAdmission::try_begin(&state)
        .expect("projection wins when shutdown has not started");
    {
        let mut current = state.lock().expect("cache controller state poisoned");
        assert!(current.optional_projection_in_flight);
        current.shutting_down = true;
    }
    assert!(
        OptionalProjectionAdmission::try_begin(&state).is_none(),
        "shutdown prevents every later projection admission"
    );
    drop(admitted);
    let current = state.lock().expect("cache controller state poisoned");
    assert!(current.shutting_down);
    assert!(!current.optional_projection_in_flight);
}

#[test]
fn idle_admission_is_once_only_and_retires_old_lease_before_io() {
    let identity = test_identity();
    let policy = CacheMaintenancePolicy {
        inactivity_limit_ms: 1_000,
        ..CacheMaintenancePolicy::default()
    };
    let config = test_config(policy);
    let state = Arc::new(Mutex::new(ControllerState::default()));
    {
        let mut state = state.lock().expect("cache controller state poisoned");
        state
            .snapshot
            .lifecycle
            .install_plan(identity.clone(), true, false, 0, Timestamp(0));
        state.last_meaningful_activity_at = Some(Timestamp(100));
        state.snapshot.idle_compaction.interval_id = Some("root-turn:1".to_owned());
        state.snapshot.scheduled_for = Some(Timestamp(900));
    }

    assert_eq!(
        admit_idle_compaction(&config, &state, None, Timestamp(1_100), false),
        Some("root-turn:1".to_owned())
    );
    let state_after = state.lock().expect("cache controller state poisoned");
    assert!(state_after.snapshot.idle_compaction.attempted);
    assert_eq!(state_after.snapshot.scheduled_for, None);
    assert!(
        state_after
            .snapshot
            .lifecycle
            .lease(&identity)
            .expect("historical lease")
            .retired
    );
    drop(state_after);
    assert_eq!(
        admit_idle_compaction(&config, &state, None, Timestamp(1_101), false),
        None
    );
}

#[test]
fn continuation_boundary_is_the_minimum_inactivity_and_child_hold() {
    let identity = test_identity();
    let mut lease = crate::cache_lifecycle::CacheLease::from_plan(identity, true, false);
    lease.record_real_parent_request(Timestamp(100));
    let policy = CacheMaintenancePolicy {
        inactivity_limit_ms: 1_000,
        max_hold_while_child_ms: 400,
        ..CacheMaintenancePolicy::default()
    };

    assert_eq!(
        hard_continuation_boundary(&lease, policy, Some(Timestamp(200)), true),
        Some(Timestamp(600))
    );
    assert_eq!(
        hard_continuation_boundary(&lease, policy, Some(Timestamp(200)), false),
        Some(Timestamp(1_100))
    );
}

#[test]
fn accepted_runtime_commit_warnings_remain_completed_and_metadata_is_bounded() {
    let mut initial = ControllerState::default();
    let parent_identity = test_identity();
    initial
        .snapshot
        .lifecycle
        .install_plan(parent_identity.clone(), true, false, 0, Timestamp(0));
    initial.snapshot.idle_compaction_provider = Some("summary-provider".to_owned());
    initial.snapshot.idle_compaction_model = Some("summary-model".to_owned());
    let state = Arc::new(Mutex::new(initial));
    let oversized = "é".repeat(MAX_METADATA_BYTES);
    let usage = UsageDelta::new().with(CounterKind::Output, 17);

    record_idle_outcome(
        &state,
        IdleOutcomeRecord {
            outcome: IdleCompactionOutcome::Completed,
            reason: Some(oversized.clone()),
            started_at: Timestamp(10),
            finished_at: Timestamp(25),
            model: Some(oversized.clone()),
            revision: Some(RegistryRevision::new(oversized)),
            usage: &usage,
        },
    );

    let snapshot = state.lock().expect("cache controller state poisoned");
    assert_eq!(
        snapshot.snapshot.idle_compaction_outcome,
        Some(IdleCompactionOutcome::Completed)
    );
    assert_eq!(snapshot.snapshot.idle_compaction_latency_ms, Some(15));
    assert_eq!(snapshot.snapshot.idle_compaction_usage.output, 17);
    assert!(
        snapshot
            .snapshot
            .idle_compaction_reason
            .as_ref()
            .is_some_and(|reason| reason.len() <= MAX_METADATA_BYTES)
    );
    assert!(
        snapshot
            .snapshot
            .idle_compaction_model
            .as_ref()
            .is_some_and(|model| model.len() <= MAX_METADATA_BYTES)
    );
    assert!(
        snapshot
            .snapshot
            .idle_compaction_revision
            .as_ref()
            .is_some_and(|revision| revision.as_str().len() <= MAX_METADATA_BYTES)
    );
    let attempt = snapshot
        .snapshot
        .synthetic_attempts
        .last()
        .expect("idle attempt projection");
    assert_eq!(attempt.purpose, ProviderAttemptPurpose::IdleCompaction);
    assert_eq!(attempt.provider, "summary-provider");
    assert!(attempt.model.len() <= MAX_METADATA_BYTES);
    assert_eq!(attempt.cache_identity, None);
    assert_eq!(
        snapshot.snapshot.lifecycle.current_identity.as_ref(),
        Some(&parent_identity)
    );
    assert_eq!(attempt.latency_ms, 15);
    assert_eq!(attempt.usage.output, 17);
    assert_eq!(
        attempt.counter_provenance[&CounterKind::Output],
        SyntheticCounterProvenance::ProviderReported
    );
    assert_eq!(attempt.cost_provenance, SyntheticCostProvenance::Unknown);
    assert!(attempt.status.len() <= MAX_METADATA_BYTES);
}

#[test]
fn cache_operation_projection_keeps_typed_usage_latency_and_identity() {
    let identity = test_identity();
    let state = Arc::new(Mutex::new(ControllerState::default()));
    let result = CacheOperationResult {
        operation: CacheOperationId::new("operation-1"),
        request: None,
        attempt: Some(AttemptId::new("attempt-1")),
        identity: identity.clone(),
        purpose: ProviderAttemptPurpose::CacheKeepalive,
        outcome: CacheOperationOutcome::Completed,
        state: agent_runtime_core::event::CacheState::WarmObserved,
        evidence: None,
        metrics: BTreeMap::new(),
        rejection_reason: None,
        terminal_reason: None,
        captured_output: None,
    };
    let usage = UsageDelta::new()
        .with(CounterKind::InputCached, 30_000)
        .with(CounterKind::Output, 2);

    record_cache_attempt_projection(&state, &result, Some(&usage), Timestamp(10), Timestamp(42));

    let state = state.lock().expect("cache controller state poisoned");
    let attempt = state
        .snapshot
        .synthetic_attempts
        .last()
        .expect("cache attempt projection");
    assert_eq!(attempt.operation.as_deref(), Some("operation-1"));
    assert_eq!(attempt.attempt.as_deref(), Some("attempt-1"));
    assert_eq!(attempt.provider, "provider");
    assert_eq!(attempt.model, "model");
    assert_eq!(
        attempt.cache_identity.as_deref(),
        Some(identity.digest().as_str())
    );
    assert_eq!(attempt.usage.input_cached, 30_000);
    assert_eq!(attempt.usage.output, 2);
    assert_eq!(attempt.latency_ms, 32);
    assert_eq!(attempt.status, "completed");
}

#[test]
fn schedule_stays_inside_hard_inactivity_boundary() {
    let identity = CacheIdentity::builder(
        "provider",
        agent_runtime_core::provider::ModelId::new("model"),
        agent_runtime_core::cache::CacheEndpointIdentity::from_opaque(
            "endpoint",
            RegistryRevision::new("endpoint-r1"),
        ),
        RegistryRevision::new("adapter-r1"),
        Fingerprint::of("profile"),
    )
    .build();
    let mut lease = crate::cache_lifecycle::CacheLease::from_plan(identity, true, false);
    lease.record_real_parent_request(Timestamp(1));
    let policy = CacheMaintenancePolicy {
        maintenance: CacheMaintenanceMode::Adaptive,
        ..Default::default()
    };
    let due = schedule_boundary(&lease, policy, Timestamp(1), Timestamp(2));
    assert!(due <= Timestamp(1).plus_millis(policy.inactivity_limit_ms));
}

#[test]
fn due_fail_closed_decision_clears_timer_and_cannot_busy_loop() {
    let mut state = ControllerState::default();
    state.snapshot.scheduled_for = Some(Timestamp(10));
    let decision = suppressed(
        CacheMaintenancePolicy::default(),
        MaintenanceSuppressionReason::MissingHostAuthority,
    );

    assert!(!record_decision(&mut state, decision, Timestamp(10)));
    assert_eq!(state.snapshot.scheduled_for, None);
    assert!(state.snapshot.interval_attempted);
    assert!(state.boundary_evaluated);
    assert_eq!(state.snapshot.interval_attempts, 0);
}

#[test]
fn configured_interval_limit_counts_preflight_reservations() {
    let mut state = ControllerState::default();
    state.snapshot.scheduled_for = Some(Timestamp(10));
    let decision = CacheSchedulerDecision {
        disposition: CacheSchedulerDisposition::Dispatch,
        action: Some(CacheMaintenanceAction::Keepalive),
        purpose: Some(ProviderAttemptPurpose::CacheKeepalive),
        reason: None,
        planned_input_tokens: 100,
        max_output_tokens: 1,
        deadline_ms: 1_000,
    };

    assert!(record_decision(&mut state, decision, Timestamp(10)));
    assert_eq!(state.snapshot.interval_attempts, 1);
    assert!(state.snapshot.operation_in_flight);
    assert_eq!(state.snapshot.scheduled_for, None);
}

#[test]
fn duplicate_and_out_of_order_events_do_not_mutate_the_capsule_projection() {
    let state = Arc::new(Mutex::new(ControllerState::default()));
    let capsule = ResumeCapsuleSlot::new(SessionId::new("session"), Timestamp::ZERO);
    let changes = ChangeRecorder::new(None);
    let completed = envelope(
        1,
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    );
    assert!(reduce_event(&state, Some(&capsule), &changes, &completed));
    let accepted = capsule.snapshot();
    assert_eq!(accepted.retained_recent_turns.len(), 1);

    assert!(!reduce_event(&state, Some(&capsule), &changes, &completed));
    assert!(!reduce_event(
        &state,
        Some(&capsule),
        &changes,
        &envelope(0, RuntimeEvent::SessionStarted),
    ));
    assert_eq!(capsule.snapshot(), accepted);
}
