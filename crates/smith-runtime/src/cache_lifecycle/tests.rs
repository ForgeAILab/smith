use super::*;
use agent_runtime::registry::{Fingerprint, RegistryRevision};
use agent_runtime_core::cache::{CacheEndpointIdentity, CacheRefreshCause, SyntheticConformance};
use agent_runtime_core::event::EventEnvelope;
use agent_runtime_core::ids::{AttemptId, EventId, RequestId, SessionId, TurnId};
use std::collections::BTreeSet;

fn identity(label: &str) -> CacheIdentity {
    CacheIdentityBuilderExt::build(label)
}

struct CacheIdentityBuilderExt;
impl CacheIdentityBuilderExt {
    fn build(label: &str) -> CacheIdentity {
        CacheIdentity::builder(
            "provider",
            agent_runtime_core::provider::ModelId::new("model"),
            CacheEndpointIdentity::new(
                Fingerprint::of("endpoint"),
                RegistryRevision::new("endpoint-r1"),
            ),
            RegistryRevision::new("adapter-r1"),
            Fingerprint::of("profile"),
        )
        .tokenizer_revision(RegistryRevision::new("tok-r1"))
        .request_adapter_revision(RegistryRevision::new("request-r1"))
        .cache_control(agent_runtime_core::provider::PromptCacheControl::Implicit)
        .provider_key(Fingerprint::of(label))
        .stable_prefix(vec![agent_runtime_core::cache::CacheIdentityFragment::new(
            "system",
            Fingerprint::of("system"),
        )])
        .build()
    }
}

fn envelope(seq: u64, timestamp: u64, payload: RuntimeEvent) -> EventEnvelope {
    EventEnvelope::new(
        seq,
        EventId::new(format!("event-{seq}")),
        SessionId::new("session"),
        Some(TurnId::new("turn")),
        Timestamp(timestamp),
        payload,
    )
}

fn adaptive_contract() -> ProviderCacheContract {
    let mut maintenance = BTreeSet::new();
    maintenance.insert(ProviderAttemptPurpose::CacheKeepalive);
    maintenance.insert(ProviderAttemptPurpose::CacheHandoffCheckpoint);
    ProviderCacheContract {
        behavior: ProviderCacheBehavior::ImplicitPrefix,
        evidence: agent_runtime_core::cache::CacheEvidenceCapabilities {
            stream: true,
            ..Default::default()
        },
        maintenance,
        conformance: Some(SyntheticConformance::complete()),
        ..Default::default()
    }
}

fn scheduler_input(id: CacheIdentity, now: u64) -> CacheSchedulerInput {
    let mut input = CacheSchedulerInput::new(id, Timestamp(now));
    input.continuation_source = true;
    input.parent_parked = true;
    input.parked_since = Some(Timestamp(1));
    input.host_synthetic_spend_allowed = true;
    input.contract = adaptive_contract();
    input.planned_input_tokens = 100;
    input.model_input_limit = 10_000;
    input.same_provider_and_model = true;
    input
}

fn ready_lease(id: CacheIdentity) -> CacheLease {
    let mut lease = CacheLease::from_plan(id, true, false);
    lease.record_real_parent_request(Timestamp(1));
    lease
}

#[test]
fn first_plan_is_eligible_but_omitted_evidence_stays_unknown() {
    let id = identity("a");
    let mut reducer = CacheLifecycleReducer::default();
    assert_eq!(
        reducer.install_plan(id.clone(), true, false, 40_000, Timestamp(1)),
        CacheLifecycleEffect::LeaseCreated
    );
    assert_eq!(
        reducer.current().unwrap().status,
        CacheLeaseStatus::Eligible
    );
    reducer.apply(&envelope(
        1,
        2,
        RuntimeEvent::CacheStateChanged {
            request: RequestId::new("r"),
            attempt: AttemptId::new("a"),
            cache_plan: Fingerprint::of("plan"),
            cache_identity: Some(id),
            state: CacheState::Unknown,
            expected_read_tokens: Some(40_000),
            observed_read_tokens: None,
            observed_write_tokens: None,
            missed_tokens: None,
            confidence: agent_runtime_core::event::EstimationConfidence::Exact,
        },
    ));
    assert_eq!(reducer.current().unwrap().status, CacheLeaseStatus::Unknown);
    assert_eq!(reducer.current().unwrap().observed_read_tokens, None);
    assert_eq!(reducer.current().unwrap().last_miss_at, None);
}

#[test]
fn explicit_zero_miss_uses_runtime_state_and_suspends_without_retry() {
    let id = identity("a");
    let mut reducer = CacheLifecycleReducer::default();
    reducer.install_plan(id.clone(), true, true, 40_000, Timestamp(1));
    let effect = reducer.apply(&envelope(
        1,
        2,
        RuntimeEvent::CacheStateChanged {
            request: RequestId::new("r"),
            attempt: AttemptId::new("a"),
            cache_plan: Fingerprint::of("plan"),
            cache_identity: Some(id.clone()),
            state: CacheState::MissObserved,
            expected_read_tokens: Some(40_000),
            observed_read_tokens: Some(0),
            observed_write_tokens: Some(40_000),
            missed_tokens: Some(40_000),
            confidence: agent_runtime_core::event::EstimationConfidence::Exact,
        },
    ));
    assert_eq!(effect, CacheLifecycleEffect::Suspended);
    let lease = reducer.current().unwrap();
    assert_eq!(lease.status, CacheLeaseStatus::Suspended);
    assert_eq!(lease.observed_status, Some(CacheLeaseStatus::MissObserved));
    assert_eq!(lease.observed_read_tokens, Some(0));
    assert_eq!(lease.maintenance_calls, 0);
}

#[test]
fn positive_read_and_typed_expiry_are_evidence_not_elapsed_inference() {
    let id = identity("a");
    let mut reducer = CacheLifecycleReducer::default();
    reducer.install_plan(id.clone(), true, true, 100, Timestamp(1));
    let mut evidence = CacheAvailabilityEvidence::stream(
        id.clone(),
        RequestId::new("r"),
        AttemptId::new("a"),
        0,
        Some(100),
        Some(0),
    )
    .with_refresh_cause(CacheRefreshCause::Read)
    .with_guaranteed_until(Timestamp(10));
    reducer.apply(&envelope(
        1,
        2,
        RuntimeEvent::CacheAvailabilityEvidenceRecorded {
            evidence: evidence.clone(),
        },
    ));
    assert_eq!(
        reducer.current().unwrap().status,
        CacheLeaseStatus::WarmObserved
    );
    assert_eq!(
        reducer
            .current()
            .unwrap()
            .effective_guaranteed_until(Timestamp(9)),
        Some(Timestamp(10))
    );
    assert_eq!(
        reducer
            .current()
            .unwrap()
            .effective_guaranteed_until(Timestamp(10)),
        None
    );
    assert_eq!(
        reducer.current().unwrap().status,
        CacheLeaseStatus::WarmObserved
    );
    evidence.kind = CacheEvidenceKind::Expired;
    evidence.source = agent_runtime_core::cache::CacheEvidenceSource::CacheScopedError;
    evidence.request = Some(RequestId::new("r"));
    evidence.attempt = Some(AttemptId::new("a"));
    evidence.exists = Some(false);
    evidence.guaranteed_until = None;
    evidence.refresh_cause = None;
    reducer.apply(&envelope(
        2,
        11,
        RuntimeEvent::CacheAvailabilityEvidenceRecorded { evidence },
    ));
    assert_eq!(
        reducer.current().unwrap().status,
        CacheLeaseStatus::Suspended
    );
    assert_eq!(
        reducer.current().unwrap().observed_status,
        Some(CacheLeaseStatus::ExpiredObserved)
    );
}

#[test]
fn activity_and_cache_touch_clocks_are_independent() {
    let id = identity("a");
    let mut lease = CacheLease::from_plan(id, true, false);
    lease.record_parent_tool_activity(Timestamp(5));
    assert_eq!(lease.last_meaningful_activity_at, Some(Timestamp(5)));
    assert_eq!(lease.last_cache_touch_at, None);
    lease.record_maintenance_call(Timestamp(6));
    assert_eq!(lease.last_cache_touch_at, Some(Timestamp(6)));
    assert_eq!(lease.last_meaningful_activity_at, Some(Timestamp(5)));
}

#[test]
fn identity_change_retires_without_transferring_warmth_or_budget() {
    let a = identity("a");
    let b = identity("b");
    let mut reducer = CacheLifecycleReducer::default();
    reducer.install_plan(a.clone(), true, false, 10, Timestamp(1));
    reducer.apply(&envelope(
        1,
        2,
        RuntimeEvent::CacheStateChanged {
            request: RequestId::new("r"),
            attempt: AttemptId::new("a"),
            cache_plan: Fingerprint::of("plan-a"),
            cache_identity: Some(a.clone()),
            state: CacheState::WarmObserved,
            expected_read_tokens: Some(10),
            observed_read_tokens: Some(10),
            observed_write_tokens: Some(0),
            missed_tokens: None,
            confidence: agent_runtime_core::event::EstimationConfidence::Exact,
        },
    ));
    reducer.current_mut().unwrap().begin_parked_interval("p");
    reducer
        .current_mut()
        .unwrap()
        .record_maintenance_call(Timestamp(3));
    reducer.install_plan(b.clone(), true, true, 10, Timestamp(4));
    assert_eq!(reducer.current().unwrap().identity, b);
    assert_eq!(reducer.current().unwrap().status, CacheLeaseStatus::Unknown);
    assert_eq!(reducer.current().unwrap().maintenance_calls, 0);
    assert_eq!(
        reducer.lease(&a).unwrap().status,
        CacheLeaseStatus::Suspended
    );
    assert!(reducer.lease(&a).unwrap().retired);
}

#[test]
fn retired_identity_history_is_bounded_without_evicting_current() {
    let mut reducer = CacheLifecycleReducer::default();
    for index in 0..(MAX_CACHE_LEASES + 7) {
        reducer.install_plan(
            identity(&format!("identity-{index}")),
            true,
            index != 0,
            10,
            Timestamp(index as u64),
        );
    }

    assert_eq!(reducer.leases.len(), MAX_CACHE_LEASES);
    assert_eq!(
        reducer.current().map(|lease| lease.identity.clone()),
        Some(identity(&format!("identity-{}", MAX_CACHE_LEASES + 6)))
    );
    assert!(reducer.leases.iter().filter(|lease| !lease.retired).count() <= 1);
    assert!(reducer.lease(&identity("identity-0")).is_none());
}

#[test]
fn one_call_budget_shared_by_handoff_and_keepalive_and_cost_is_ignored() {
    let id = identity("a");
    let mut lease = ready_lease(id.clone());
    lease.begin_parked_interval("p1");
    lease.record_meaningful_activity(Timestamp(1));
    let scheduler = CacheScheduler::new(CacheMaintenancePolicy {
        maintenance: CacheMaintenanceMode::Adaptive,
        ..Default::default()
    })
    .unwrap();
    let mut input = scheduler_input(id.clone(), 2_000);
    input.estimated_cost_micro_usd = Some(1);
    let low = scheduler.evaluate(&lease, &input);
    input.estimated_cost_micro_usd = Some(u128::MAX);
    let high = scheduler.evaluate(&lease, &input);
    assert_eq!(low, high);
    assert_eq!(low.action, Some(CacheMaintenanceAction::HandoffCheckpoint));
    assert!(low.is_dispatch());
    lease.record_maintenance_call(Timestamp(2_000));
    let exhausted = scheduler.evaluate(&lease, &input);
    assert_eq!(
        exhausted.reason,
        Some(MaintenanceSuppressionReason::CallBudgetExhausted)
    );
}

#[test]
fn scheduler_gates_authority_conformance_and_provider_limits() {
    let id = identity("a");
    let lease = ready_lease(id.clone());
    let scheduler = CacheScheduler::new(CacheMaintenancePolicy {
        maintenance: CacheMaintenanceMode::Adaptive,
        ..Default::default()
    })
    .unwrap();
    let mut input = scheduler_input(id, 2_000);
    input.host_synthetic_spend_allowed = false;
    assert_eq!(
        scheduler.evaluate(&lease, &input).reason,
        Some(MaintenanceSuppressionReason::MissingHostAuthority)
    );
    input.host_synthetic_spend_allowed = true;
    input.contract.conformance = None;
    assert_eq!(
        scheduler.evaluate(&lease, &input).reason,
        Some(MaintenanceSuppressionReason::MissingConformance)
    );
    input.contract = adaptive_contract();
    input.limits.provider_input_tokens = Some(1);
    assert_eq!(
        scheduler.evaluate(&lease, &input).reason,
        Some(MaintenanceSuppressionReason::InputBudgetExceeded)
    );
    input.limits.provider_input_tokens = None;
    input.limits.provider_total_tokens = Some(355);
    assert_eq!(
        scheduler.evaluate(&lease, &input).reason,
        Some(MaintenanceSuppressionReason::InputBudgetExceeded)
    );
    input.limits.provider_total_tokens = None;
    input.limits.session_total_tokens = Some(355);
    assert_eq!(
        scheduler.evaluate(&lease, &input).reason,
        Some(MaintenanceSuppressionReason::InputBudgetExceeded)
    );
}

#[test]
fn guarantee_can_suppress_known_window_but_never_invents_expiry() {
    let id = identity("a");
    let mut lease = ready_lease(id.clone());
    lease.guaranteed_until = Some(Timestamp(10_000));
    lease.record_meaningful_activity(Timestamp(1));
    let scheduler = CacheScheduler::new(CacheMaintenancePolicy {
        maintenance: CacheMaintenanceMode::Adaptive,
        ..Default::default()
    })
    .unwrap();
    let mut input = scheduler_input(id, 2_000);
    input.continuation_expected_by = Some(Timestamp(9_000));
    assert_eq!(
        scheduler.evaluate(&lease, &input).reason,
        Some(MaintenanceSuppressionReason::GuaranteedRetention)
    );
    input.now = Timestamp(10_000);
    input.continuation_expected_by = None;
    assert!(scheduler.evaluate(&lease, &input).is_dispatch());
    assert_eq!(lease.status, CacheLeaseStatus::Eligible);
}

#[test]
fn idle_compaction_is_once_per_interval_and_does_not_retry() {
    let mut controller = IdleCompactionController::default();
    let policy = CacheMaintenancePolicy {
        maintenance: CacheMaintenanceMode::Adaptive,
        ..Default::default()
    };
    let input = IdleCompactionInput {
        now: Timestamp(DEFAULT_INACTIVITY_LIMIT_MS + 1),
        last_meaningful_activity_at: Some(Timestamp(1)),
        interval_id: "idle-1".to_owned(),
        safe_boundary: true,
        lifecycle_active: true,
        shutdown: false,
        child_active: true,
    };
    assert_eq!(
        controller.evaluate(policy, &input).disposition,
        IdleCompactionDisposition::Attempt
    );
    assert_eq!(
        controller.evaluate(policy, &input).reason,
        Some(CompactionSuppressionReason::AlreadyAttempted)
    );
    controller.reset("idle-2");
    assert_eq!(
        controller
            .evaluate(
                policy,
                &IdleCompactionInput {
                    interval_id: "idle-2".to_owned(),
                    ..input
                }
            )
            .disposition,
        IdleCompactionDisposition::Attempt
    );
}

#[test]
fn cold_resume_forbids_prewarm_until_real_request() {
    let id = identity("a");
    let mut lease = CacheLease::from_plan(id.clone(), true, false);
    lease.record_meaningful_activity(Timestamp(1));
    lease.cold_resume(Timestamp(2));
    let scheduler = CacheScheduler::new(CacheMaintenancePolicy {
        maintenance: CacheMaintenanceMode::Adaptive,
        ..Default::default()
    })
    .unwrap();
    let input = scheduler_input(id.clone(), 2_000);
    assert_eq!(
        scheduler.evaluate(&lease, &input).reason,
        Some(MaintenanceSuppressionReason::ColdResumeNoPrewarm)
    );
    lease.record_real_parent_request(Timestamp(2_001));
    assert!(lease.suspension_reason.is_none());
}

#[test]
fn runtime_usage_counts_only_actual_synthetic_attempts() {
    let id = identity("a");
    let mut reducer = CacheLifecycleReducer::default();
    reducer.install_plan(id.clone(), true, false, 100, Timestamp(1));
    let usage = agent_runtime_core::usage::UsageRecord {
        source: agent_runtime_core::usage::UsageSource::ProviderAttempt,
        provenance: agent_runtime_core::usage::Provenance {
            attempt: Some(AttemptId::new("a")),
            attempt_purpose: Some(ProviderAttemptPurpose::CacheKeepalive),
            cache_identity: Some(id.clone()),
            ..Default::default()
        },
        delta: agent_runtime_core::usage::UsageDelta::new()
            .with(CounterKind::InputCached, 90)
            .with(CounterKind::Output, 7),
    };
    reducer.apply(&envelope(1, 2, RuntimeEvent::Usage { record: usage }));
    let lease = reducer.current().unwrap();
    assert_eq!(lease.maintenance_input_tokens, 90);
    assert_eq!(lease.maintenance_output_tokens, 7);
}

#[test]
fn runtime_event_replay_is_idempotent() {
    let id = identity("a");
    let mut reducer = CacheLifecycleReducer::default();
    reducer.install_plan(id.clone(), true, false, 10, Timestamp(1));
    let event = envelope(
        1,
        2,
        RuntimeEvent::CacheOperationStarted {
            operation: agent_runtime_core::ids::CacheOperationId::new("op"),
            request: Some(RequestId::new("r")),
            attempt: Some(AttemptId::new("a")),
            identity: id,
            purpose: ProviderAttemptPurpose::CacheKeepalive,
        },
    );
    reducer.apply(&event);
    reducer.apply(&event);
    assert_eq!(reducer.current().unwrap().maintenance_calls, 1);
}

#[test]
fn jitter_is_deterministic_and_stays_before_boundary() {
    let a = CacheScheduler::jittered_due_time(Timestamp(100_000), 10_000, 10, 7);
    let b = CacheScheduler::jittered_due_time(Timestamp(100_000), 10_000, 10, 7);
    assert_eq!(a, b);
    assert!(a <= Timestamp(100_000));
}
