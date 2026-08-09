//! Host-owned adaptive cache lifecycle controller.
//!
//! Agent Runtime owns exact cache identities, immutable request construction,
//! provider admission, canonical events, and usage. This module owns Smith's
//! cancellable scheduling policy and resume-capsule projection. It never
//! reconstructs a prompt or emits a competing event vocabulary.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_runtime::cache::{CacheHandoffSuffix, CacheOperationResult};
use agent_runtime::context::Sensitivity;
use agent_runtime::harness::ProtectedSemanticSummary;
use agent_runtime::registry::{Fingerprint, RegistryRevision};
use agent_runtime::runtime::{IdleCompactionAdmission, SessionHandle};
use agent_runtime_core::artifact::{
    ArtifactProvenance, ArtifactRetention, ArtifactSensitivity, ArtifactStore, ArtifactWrite,
};
use agent_runtime_core::cancel::{CancelReason, Cancellation};
use agent_runtime_core::clock::{Clock, Deadline, Timestamp};
use agent_runtime_core::event::{CacheOperationOutcome, EventEnvelope, RuntimeEvent};
use agent_runtime_core::ids::{AttemptId, CacheOperationId, ChildId};
use agent_runtime_core::provider::{
    CacheAuthority, CacheEndpointIdentity, CacheIdentity, CacheOperationBudget,
    ProviderAttemptPurpose, ProviderCacheContract, RateLimitSnapshot,
};
use agent_runtime_core::usage::{CounterKind, UsageDelta};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use smith_config::model::CacheMaintenanceMode as ConfigMaintenanceMode;
use smith_config::resolve::{ResolvedCachePolicy, SyntheticCacheSpendAuthority};
use smith_tools::{ChangeRecorder, ToolMutation};

use crate::cache_lifecycle::{
    CacheLifecycleReducer, CacheMaintenanceAction, CacheMaintenanceMode, CacheMaintenancePolicy,
    CacheScheduler, CacheSchedulerDecision, CacheSchedulerDisposition, CacheSchedulerInput,
    IdleCompactionController, IdleCompactionDecision, IdleCompactionDisposition,
    IdleCompactionInput, MaintenanceSuppressionReason, SchedulerLimits,
};
use crate::delegation::{DelegationParkingMonitor, ParentParkingState, ParkingSnapshot};
use crate::resume_capsule::{
    ArtifactProjection, ChangedFileProjection, ChildLifecycleState, ChildResumeProjection,
    ChildTerminalOutcome, ExactGoalProjection, ExactPlanProjection, MAX_ARTIFACTS,
    MAX_CHANGED_FILES, MAX_CHILDREN, MAX_METADATA_BYTES, MAX_SUMMARY_BYTES, MAX_VALIDATIONS,
    RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE, RESUME_SUMMARY_ARTIFACT_PURPOSE,
    RESUME_SUMMARY_MEDIA_TYPE, RecentTurnProjection, RecentTurnRole, ResumeCacheWarmth,
    ResumeCapsule, ResumeCapsuleSlot, SummaryCoverage, SummaryUsage, ValidationProjection,
};

const HANDOFF_SUFFIX: &str = "Create a concise continuation checkpoint for the same session. Summarize current progress, verified state, unresolved work, and the next safe action. Do not call tools.";
const SHUTDOWN_DRAIN_MS: u64 = 500;
const MAX_SYNTHETIC_ATTEMPT_PROJECTIONS: usize = 32;

/// Terminal projection for Smith's separate ordinary idle-compaction lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdleCompactionOutcome {
    /// Runtime committed the compaction; optional capsule projection may be
    /// absent when a post-commit host step warns.
    Completed,
    /// The one attempt completed without a new summary (ineligible/fallback).
    NoSummary,
    /// A real user/internal turn or another operation won the final boundary.
    Busy,
    /// Shutdown/cancellation won the final boundary.
    Shutdown,
    /// Persistence or another non-retryable operation failed.
    Failed,
}

/// Provenance for one actual provider counter in a synthetic attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyntheticCounterProvenance {
    /// The provider returned this billing counter.
    ProviderReported,
}

/// Provenance for synthetic-attempt cost. Smith currently receives token
/// usage, not a provider bill, so unknown remains explicit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyntheticCostProvenance {
    /// No provider-reported or compatible calculated cost was available.
    #[default]
    Unknown,
    /// A provider supplied the amount directly.
    ProviderReported,
    /// Smith calculated the amount from an exact compatible price table.
    Calculated,
}

/// Bounded, content-free accounting record for one accepted synthetic cache
/// or idle-compaction attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticCacheAttemptProjection {
    /// Stable operation id for cache operations; idle compaction has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    /// Provider attempt id when Runtime allocated one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<String>,
    /// Typed Runtime purpose.
    pub purpose: ProviderAttemptPurpose,
    /// Provider attribution.
    pub provider: String,
    /// Model attribution.
    pub model: String,
    /// Exact Runtime cache identity digest for maintenance work. Ordinary idle
    /// compaction deliberately has no parent cache identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_identity: Option<String>,
    /// Actual disjoint provider counters; never estimates.
    pub usage: SummaryUsage,
    /// Provenance only for counters that were actually present.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counter_provenance: BTreeMap<CounterKind, SyntheticCounterProvenance>,
    /// Actual or calculated cost when known, with explicit provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_micro_usd: Option<u128>,
    /// Whether the cost was provider-reported, compatibly calculated, or
    /// unavailable.
    pub cost_provenance: SyntheticCostProvenance,
    /// Measured host wall-clock latency.
    pub latency_ms: u64,
    /// Bounded terminal status; no provider body or error text.
    pub status: String,
}

/// Last host scheduling state exposed to status and tests.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheControllerSnapshot {
    /// Resolved provider route owned by this controller.
    pub provider: String,
    /// Resolved model route owned by this controller.
    pub model: String,
    /// Requested maintenance mode before host/capability narrowing.
    pub requested_maintenance: CacheMaintenanceMode,
    /// Effective mode used by the scheduler.
    pub effective_maintenance: CacheMaintenanceMode,
    /// Redaction-safe explanation when policy was narrowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrowing_reason: Option<String>,
    /// Effective bounded scheduler policy.
    pub policy: CacheMaintenancePolicy,
    /// Whether host-only spend authority was granted.
    pub synthetic_spend_authorized: bool,
    /// Runtime-normalized model/adapter contract used at the last decision.
    pub provider_contract: ProviderCacheContract,
    /// Evidence-bearing exact-identity lifecycle.
    pub lifecycle: CacheLifecycleReducer,
    /// Latest pure scheduling decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<CacheSchedulerDecision>,
    /// Next due boundary, when one is scheduled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_for: Option<Timestamp>,
    /// Whether one Runtime cache dispatch currently owns the reservation.
    pub operation_in_flight: bool,
    /// Whether this parked interval already consumed its one scheduling
    /// boundary or at least one fail-closed/pre-I/O attempt.
    pub interval_attempted: bool,
    /// Number of bounded synthetic reservations consumed in this parked
    /// interval. This is separate from provider-admitted calls so a rejected
    /// preflight cannot spin or bypass the configured limit.
    pub interval_attempts: u32,
    /// Durable once-per-root-idle-interval ordinary compaction gate.
    pub idle_compaction: IdleCompactionController,
    /// Latest idle-compaction admission/suppression decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_compaction_decision: Option<IdleCompactionDecision>,
    /// Last final Runtime/host outcome for the admitted idle interval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_compaction_outcome: Option<IdleCompactionOutcome>,
    /// Redaction-safe fallback/error category for that outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_compaction_reason: Option<String>,
    /// Wall-clock duration of the last attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_compaction_latency_ms: Option<u64>,
    /// Actual separately attributed provider counters; never an estimate.
    #[serde(default)]
    pub idle_compaction_usage: SummaryUsage,
    /// Actual summary provider route, independent of the parent cache lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_compaction_provider: Option<String>,
    /// Actual summary model route identity, when a summary committed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_compaction_model: Option<String>,
    /// Summary body revision, without content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_compaction_revision: Option<RegistryRevision>,
    /// Bounded per-attempt accounting across keepalive, handoff, explicit
    /// resource work, and ordinary idle compaction.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub synthetic_attempts: Vec<SyntheticCacheAttemptProjection>,
    /// Current parked interval identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parked_interval: Option<u64>,
    /// Last bounded controller error category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Default)]
struct ControllerState {
    snapshot: CacheControllerSnapshot,
    planned_input_tokens: u32,
    last_meaningful_activity_at: Option<Timestamp>,
    parked_since: Option<Timestamp>,
    goal_active: bool,
    shutting_down: bool,
    optional_projection_in_flight: bool,
    synthetic_attempts: BTreeSet<AttemptId>,
    plan_has_comparable_predecessor: bool,
    parent_turn_active: bool,
    parent_idle_interval: u64,
    boundary_evaluated: bool,
    event_gap: bool,
    provider_rate_limits: Option<RateLimitSnapshot>,
    goal_remaining_tokens: Option<u64>,
}

/// Atomic ordering token between shutdown and post-Runtime optional
/// projection. Whichever operation acquires the controller-state mutex first
/// wins: shutdown prevents a new admission, while an already-admitted
/// projection is drained before Runtime's final session save.
struct OptionalProjectionAdmission {
    state: Arc<Mutex<ControllerState>>,
}

impl OptionalProjectionAdmission {
    fn try_begin(state: &Arc<Mutex<ControllerState>>) -> Option<Self> {
        let mut current = state.lock().expect("cache controller state poisoned");
        if current.shutting_down || current.optional_projection_in_flight {
            return None;
        }
        current.optional_projection_in_flight = true;
        drop(current);
        Some(Self {
            state: Arc::clone(state),
        })
    }
}

impl Drop for OptionalProjectionAdmission {
    fn drop(&mut self) {
        self.state
            .lock()
            .expect("cache controller state poisoned")
            .optional_projection_in_flight = false;
    }
}

/// Immutable controller inputs resolved before the session starts.
#[derive(Clone)]
pub(crate) struct CacheControllerConfig {
    /// Bounded Smith scheduler policy.
    pub policy: CacheMaintenancePolicy,
    /// Host-only synthetic-spend authority.
    pub synthetic_spend: SyntheticCacheSpendAuthority,
    /// Runtime-normalized model/adapter cache contract.
    pub contract: ProviderCacheContract,
    /// Exact resolved model input limit.
    pub model_input_limit: u32,
    /// Exact resolved model output limit.
    pub model_output_limit: u32,
    /// Provider label used only for handoff provenance.
    pub provider: String,
    /// Model label used only for handoff provenance.
    pub model: String,
    /// Independently attributed semantic-summary provider route.
    pub semantic_summary_provider: Option<String>,
    /// Independently attributed semantic-summary model/profile route.
    pub semantic_summary_model: Option<String>,
    /// Exact host-owned endpoint/tenant partition supplied to Runtime.
    pub endpoint_identity: Option<CacheEndpointIdentity>,
    /// Exact resolved model-profile fingerprint folded into Runtime identity.
    pub profile_identity: Fingerprint,
    /// Requested mode before host authority narrowing.
    pub requested_maintenance: CacheMaintenanceMode,
    /// Redaction-safe authority/capability narrowing explanation.
    pub narrowing_reason: Option<String>,
}

/// Runtime/model inputs that bind one resolved cache policy to a concrete
/// host session.
pub(crate) struct CacheControllerResolvedInputs {
    /// Host-only synthetic-spend authority.
    pub synthetic_spend: SyntheticCacheSpendAuthority,
    /// Runtime-normalized model/adapter cache contract.
    pub contract: ProviderCacheContract,
    /// Exact resolved model input limit.
    pub model_input_limit: u32,
    /// Exact resolved model output limit.
    pub model_output_limit: u32,
    /// Provider route label.
    pub provider: String,
    /// Model route label.
    pub model: String,
    /// Exact host-owned endpoint/tenant partition supplied to Runtime.
    pub endpoint_identity: Option<CacheEndpointIdentity>,
    /// Exact resolved model-profile fingerprint folded into Runtime identity.
    pub profile_identity: Fingerprint,
    /// Runtime's protected semantic-summary provider route, when installed.
    pub semantic_summary_provider: Option<String>,
    /// Runtime's protected semantic-summary model/profile route, when installed.
    pub semantic_summary_model: Option<String>,
    /// Whether the resume capsule can persist a once-only attempt marker.
    pub attempt_marker_available: bool,
}

impl std::fmt::Debug for CacheControllerConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CacheControllerConfig")
            .field("policy", &self.policy)
            .field("synthetic_spend", &self.synthetic_spend)
            .field("contract", &self.contract)
            .field("model_input_limit", &self.model_input_limit)
            .field("model_output_limit", &self.model_output_limit)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("semantic_summary_provider", &self.semantic_summary_provider)
            .field("semantic_summary_model", &self.semantic_summary_model)
            .field("endpoint_identity", &self.endpoint_identity)
            .field("profile_identity", &self.profile_identity)
            .field("requested_maintenance", &self.requested_maintenance)
            .field("narrowing_reason", &self.narrowing_reason)
            .finish()
    }
}

impl CacheControllerConfig {
    /// Converts provenance-carrying Smith configuration into the pure policy.
    pub(crate) fn from_resolved(
        cache: &ResolvedCachePolicy,
        inputs: CacheControllerResolvedInputs,
    ) -> Result<Self, &'static str> {
        let CacheControllerResolvedInputs {
            synthetic_spend,
            contract,
            model_input_limit,
            model_output_limit,
            provider,
            model,
            semantic_summary_provider,
            semantic_summary_model,
            endpoint_identity,
            profile_identity,
            attempt_marker_available,
        } = inputs;
        if semantic_summary_provider.is_some() != semantic_summary_model.is_some() {
            return Err("semantic-summary provider and model attribution must be paired");
        }
        let semantic_summary_available = semantic_summary_provider.is_some();
        let requested_maintenance = match cache.requested_maintenance.value {
            ConfigMaintenanceMode::Off => CacheMaintenanceMode::Off,
            ConfigMaintenanceMode::Observe => CacheMaintenanceMode::Observe,
            ConfigMaintenanceMode::Adaptive => CacheMaintenanceMode::Adaptive,
        };
        let maintenance = match cache.effective_maintenance.value {
            ConfigMaintenanceMode::Off => CacheMaintenanceMode::Off,
            ConfigMaintenanceMode::Observe => CacheMaintenanceMode::Observe,
            ConfigMaintenanceMode::Adaptive => CacheMaintenanceMode::Adaptive,
        };
        let mut policy = CacheMaintenancePolicy {
            maintenance,
            inactivity_limit_ms: cache.inactivity_limit_ms.value,
            max_hold_while_child_ms: cache.max_hold_while_child_ms.value,
            max_maintenance_calls: u32::from(cache.max_maintenance_calls.value),
            max_maintenance_input_tokens: cache.max_maintenance_input_tokens.value,
            max_maintenance_output_tokens: cache
                .max_maintenance_output_tokens
                .value
                .min(model_output_limit),
            maintenance_deadline_ms: cache.maintenance_deadline_ms.value,
            keepalive_margin_ms: cache.keepalive_margin_ms.value,
            keepalive_jitter_percent: cache.keepalive_jitter_percent.value,
            handoff_checkpoint: cache.handoff_checkpoint.value,
            idle_compaction: cache.idle_compaction.value,
        };
        policy.validate()?;
        let mut narrowing_reason = cache.narrowing_reason.clone();
        if cache.max_maintenance_output_tokens.value > model_output_limit {
            let output_reason = format!(
                "maintenance output narrowed to resolved model limit ({model_output_limit})"
            );
            narrowing_reason = Some(match narrowing_reason {
                Some(existing) => format!("{existing}; {output_reason}"),
                None => output_reason,
            });
        }
        if policy.idle_compaction && !semantic_summary_available {
            policy.idle_compaction = false;
            let summary_reason =
                "idle compaction disabled because no protected semantic-summary route is installed";
            narrowing_reason = Some(match narrowing_reason {
                Some(existing) => format!("{existing}; {summary_reason}"),
                None => summary_reason.to_owned(),
            });
        }
        if policy.idle_compaction && !attempt_marker_available {
            policy.idle_compaction = false;
            let marker_reason =
                "idle compaction disabled because the once-only resume marker is unavailable";
            narrowing_reason = Some(match narrowing_reason {
                Some(existing) => format!("{existing}; {marker_reason}"),
                None => marker_reason.to_owned(),
            });
        }
        if policy.handoff_checkpoint && !attempt_marker_available {
            policy.handoff_checkpoint = false;
            let handoff_reason = "handoff checkpoint disabled because the resume capsule/attempt marker is unavailable";
            narrowing_reason = Some(match narrowing_reason {
                Some(existing) => format!("{existing}; {handoff_reason}"),
                None => handoff_reason.to_owned(),
            });
        }
        Ok(Self {
            policy,
            synthetic_spend,
            contract,
            model_input_limit,
            model_output_limit,
            provider,
            model,
            semantic_summary_provider,
            semantic_summary_model,
            endpoint_identity,
            profile_identity,
            requested_maintenance,
            narrowing_reason,
        })
    }
}

/// Cancellable owner for Smith's one background cache lifecycle worker.
#[derive(Debug)]
pub(crate) struct CacheLifecycleController {
    state: Arc<Mutex<ControllerState>>,
    cancel: Cancellation,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl CacheLifecycleController {
    /// Starts event reduction, parking-aware scheduling, and capsule updates.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start(
        session: SessionHandle,
        config: CacheControllerConfig,
        clock: Arc<dyn Clock>,
        parking: Option<DelegationParkingMonitor>,
        capsule: Option<Arc<ResumeCapsuleSlot>>,
        artifacts: Option<Arc<dyn ArtifactStore>>,
        changes: Arc<ChangeRecorder>,
    ) -> Self {
        let mut initial = ControllerState::default();
        initial.snapshot.requested_maintenance = config.requested_maintenance;
        initial.snapshot.provider = config.provider.clone();
        initial.snapshot.model = config.model.clone();
        initial.snapshot.idle_compaction_provider = config.semantic_summary_provider.clone();
        initial.snapshot.idle_compaction_model = config.semantic_summary_model.clone();
        initial.snapshot.effective_maintenance = config.policy.maintenance;
        initial.snapshot.narrowing_reason = config.narrowing_reason.clone();
        initial.snapshot.policy = config.policy;
        initial.snapshot.synthetic_spend_authorized =
            config.synthetic_spend == SyntheticCacheSpendAuthority::Allow;
        initial.snapshot.provider_contract = config.contract.clone();
        if let Some(capsule) = capsule.as_deref() {
            let restored = capsule.snapshot();
            initial.last_meaningful_activity_at = restored.cache.last_meaningful_activity_at;
            initial.snapshot.idle_compaction.interval_id =
                restored.cache.idle_compaction_interval_id.clone();
            initial.snapshot.idle_compaction.attempted = restored.cache.idle_compaction_attempted;
            if let Some(identity) = restored.cache.prior_identity {
                initial.snapshot.lifecycle.restore_cold_identity(
                    identity,
                    restored.exact_state.watermark,
                    clock.now(),
                );
            }
        }
        let state = Arc::new(Mutex::new(initial));
        let cancel = Cancellation::new();
        let task = tokio::spawn(run_controller(
            session,
            config,
            clock,
            parking,
            capsule,
            artifacts,
            changes,
            state.clone(),
            cancel.clone(),
        ));
        Self {
            state,
            cancel,
            task: Mutex::new(Some(task)),
        }
    }

    /// Returns an inspectable, redaction-safe snapshot.
    pub(crate) fn snapshot(&self) -> CacheControllerSnapshot {
        self.state
            .lock()
            .expect("cache controller state poisoned")
            .snapshot
            .clone()
    }

    /// Prevents new scheduling, cancels an in-flight Runtime operation, and
    /// boundedly drains the single worker.
    pub(crate) async fn shutdown(&self) {
        self.stop_scheduling();
        let task = self
            .task
            .lock()
            .expect("cache controller task poisoned")
            .take();
        if let Some(mut task) = task
            && tokio::time::timeout(Duration::from_millis(SHUTDOWN_DRAIN_MS), &mut task)
                .await
                .is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }

    /// Freezes dispatch synchronously so the host can then cancel the Runtime
    /// session before awaiting an idle-summary provider call.
    pub(crate) fn stop_scheduling(&self) {
        {
            let mut state = self.state.lock().expect("cache controller state poisoned");
            state.shutting_down = true;
            state.snapshot.scheduled_for = None;
            state.snapshot.lifecycle.begin_shutdown();
        }
        self.cancel.cancel(CancelReason::Shutdown);
    }
}

impl Drop for CacheLifecycleController {
    fn drop(&mut self) {
        self.cancel.cancel(CancelReason::Shutdown);
        if let Some(task) = self
            .task
            .lock()
            .expect("cache controller task poisoned")
            .take()
        {
            task.abort();
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_controller(
    session: SessionHandle,
    config: CacheControllerConfig,
    clock: Arc<dyn Clock>,
    parking: Option<DelegationParkingMonitor>,
    capsule: Option<Arc<ResumeCapsuleSlot>>,
    artifacts: Option<Arc<dyn ArtifactStore>>,
    changes: Arc<ChangeRecorder>,
    state: Arc<Mutex<ControllerState>>,
    cancel: Cancellation,
) {
    let scheduler = match CacheScheduler::new(config.policy) {
        Ok(scheduler) => scheduler,
        Err(error) => {
            state
                .lock()
                .expect("cache controller state poisoned")
                .snapshot
                .last_error = Some(error.to_owned());
            return;
        }
    };
    let mut events = session.subscribe();
    loop {
        let wait_ms = {
            let state = state.lock().expect("cache controller state poisoned");
            state
                .snapshot
                .scheduled_for
                .map(|due| due.0.saturating_sub(clock.now().0))
        };
        let idle_wait_ms = {
            let state = state.lock().expect("cache controller state poisoned");
            idle_compaction_wait_ms(&config, &state, clock.now())
        };
        let timer = async {
            match wait_ms {
                Some(wait_ms) => tokio::time::sleep(Duration::from_millis(wait_ms)).await,
                None => std::future::pending::<()>().await,
            }
        };
        // Idle compaction has its own root-activity timer. It deliberately
        // does not derive its deadline from the parking monitor or child
        // lifecycle, so a child can keep running without extending or
        // interrupting the parent's independent idle boundary.
        let idle_timer = async {
            match idle_wait_ms {
                Some(wait_ms) => tokio::time::sleep(Duration::from_millis(wait_ms)).await,
                None => std::future::pending::<()>().await,
            }
        };
        let parking_change = async {
            match &parking {
                Some(parking) => parking.changed().await,
                None => std::future::pending::<()>().await,
            }
        };
        let mut persist_boundary = false;
        let mut idle_due = false;
        tokio::select! {
            _ = cancel.cancelled() => break,
            envelope = events.next() => match envelope {
                Some(envelope) => {
                    persist_boundary =
                        reduce_event(&state, capsule.as_deref(), &changes, &envelope);
                }
                None => break,
            },
            _ = parking_change => {
                persist_boundary = true;
            }
            _ = timer => {},
            _ = idle_timer => {
                idle_due = true;
            }
        }

        let parking_snapshot = parking.as_ref().map(DelegationParkingMonitor::snapshot);
        if reconcile_parking(&state, parking_snapshot.as_ref(), clock.now()) {
            persist_boundary = true;
        }
        if persist_boundary
            && capsule.is_some()
            && !cancel.is_cancelled()
            && session.persist().await.is_err()
        {
            state
                .lock()
                .expect("cache controller state poisoned")
                .snapshot
                .last_error = Some("capsule_persist_failed".to_owned());
        }
        if cancel.is_cancelled() {
            break;
        }
        if idle_due {
            run_idle_compaction(
                &session,
                &config,
                &clock,
                parking_snapshot.as_ref(),
                capsule.as_deref(),
                artifacts.as_deref(),
                &state,
                &cancel,
            )
            .await;
        }
        if cancel.is_cancelled() {
            break;
        }
        let mut decision = evaluate(
            &scheduler,
            &config,
            &state,
            parking_snapshot.as_ref(),
            clock.now(),
        );
        // Parking and user/goal admission can change independently of the
        // event worker. Re-read the identity-only parking projection at the
        // final synchronous dispatch boundary and repeat every policy gate.
        if decision.disposition == CacheSchedulerDisposition::Dispatch {
            let latest_parking = parking.as_ref().map(DelegationParkingMonitor::snapshot);
            decision = evaluate(
                &scheduler,
                &config,
                &state,
                latest_parking.as_ref(),
                clock.now(),
            );
        }
        let now = clock.now();
        let dispatch;
        {
            let mut state = state.lock().expect("cache controller state poisoned");
            dispatch = record_decision(&mut state, decision.clone(), now);
        }
        if !dispatch {
            continue;
        }
        let expected_identity = state
            .lock()
            .expect("cache controller state poisoned")
            .snapshot
            .lifecycle
            .current_identity
            .clone();
        let operation_started_at = clock.now();
        let result = dispatch_once(&session, &config, &clock, &state, &cancel, &decision).await;
        let operation_finished_at = clock.now();
        if let Ok(result) = &result {
            let snapshot = session.snapshot();
            let usage = result.attempt.as_ref().and_then(|attempt| {
                snapshot
                    .usage
                    .records()
                    .iter()
                    .rev()
                    .find(|record| {
                        record.provenance.attempt.as_ref() == Some(attempt)
                            && record.provenance.attempt_purpose == Some(result.purpose)
                    })
                    .map(|record| record.delta.clone())
            });
            record_cache_attempt_projection(
                &state,
                result,
                usage.as_ref(),
                operation_started_at,
                operation_finished_at,
            );
        }
        {
            let mut state = state.lock().expect("cache controller state poisoned");
            state.snapshot.operation_in_flight = false;
            state.snapshot.scheduled_for = None;
            if let Err(error) = &result {
                state.snapshot.last_error = Some(error.clone());
            }
        }
        // Runtime has already durably accounted an admitted cache operation.
        // During shutdown, skip optional Smith handoff projection work so no
        // capsule mutation can race the Runtime's final session save.
        if cancel.is_cancelled() {
            break;
        }
        if let (Ok(result), Some(expected_identity)) = (result, expected_identity)
            && let Err(error) = persist_handoff(
                &session,
                &config,
                capsule.as_deref(),
                artifacts.as_deref(),
                &result,
                &expected_identity,
                clock.now(),
            )
            .await
        {
            state
                .lock()
                .expect("cache controller state poisoned")
                .snapshot
                .last_error = Some(error);
        }
    }
    let mut state = state.lock().expect("cache controller state poisoned");
    state.shutting_down = true;
    state.snapshot.scheduled_for = None;
}

/// Returns the independent root-idle deadline. Parking, goal state, and child
/// activity are intentionally absent: they belong to the separate
/// cache-maintenance scheduler, not ordinary semantic compaction.
fn idle_compaction_wait_ms(
    config: &CacheControllerConfig,
    state: &ControllerState,
    now: Timestamp,
) -> Option<u64> {
    if !config.policy.idle_compaction
        || state.shutting_down
        || state.snapshot.lifecycle.shutdown
        || state.event_gap
        || state.parent_turn_active
        || state.snapshot.operation_in_flight
        || state.snapshot.idle_compaction.attempted
    {
        return None;
    }
    state.snapshot.idle_compaction.interval_id.as_ref()?;
    let activity = state.last_meaningful_activity_at?;
    Some(
        activity
            .plus_millis(config.policy.inactivity_limit_ms)
            .0
            .saturating_sub(now.0),
    )
}

/// Evaluates the durable idle gate once at the final host boundary before
/// marker persistence or the Runtime call. Admission retires the old cache
/// lease immediately, including when a later step fails or returns Busy.
fn admit_idle_compaction(
    config: &CacheControllerConfig,
    state: &Arc<Mutex<ControllerState>>,
    parking: Option<&ParkingSnapshot>,
    now: Timestamp,
    cancelled: bool,
) -> Option<String> {
    let mut state = state.lock().expect("cache controller state poisoned");
    let interval_id = state.snapshot.idle_compaction.interval_id.clone()?;
    let child_active = parking.is_some_and(|parking| !parking.pending_children.is_empty());
    let safe_boundary = !cancelled
        && !state.shutting_down
        && !state.snapshot.lifecycle.shutdown
        && !state.parent_turn_active
        && !state.snapshot.operation_in_flight
        && !state.event_gap;
    let last_meaningful_activity_at = state.last_meaningful_activity_at;
    let lifecycle_active = !state.shutting_down && !state.snapshot.lifecycle.shutdown;
    let shutdown = cancelled || state.shutting_down;
    let decision = state.snapshot.idle_compaction.evaluate(
        config.policy,
        &IdleCompactionInput {
            now,
            last_meaningful_activity_at,
            interval_id: interval_id.clone(),
            safe_boundary,
            lifecycle_active,
            shutdown,
            child_active,
        },
    );
    state.snapshot.idle_compaction_decision = Some(decision);
    if decision.disposition != IdleCompactionDisposition::Attempt {
        return None;
    }
    state.snapshot.scheduled_for = None;
    state.boundary_evaluated = true;
    state.snapshot.lifecycle.retire_after_compaction(now);
    Some(interval_id)
}

/// Runs one admitted ordinary idle-compaction attempt. The capsule marker is
/// persisted before entering Runtime, so a store failure cannot authorize a
/// provider retry.
#[allow(clippy::too_many_arguments)]
async fn run_idle_compaction(
    session: &SessionHandle,
    config: &CacheControllerConfig,
    clock: &Arc<dyn Clock>,
    parking: Option<&ParkingSnapshot>,
    capsule: Option<&ResumeCapsuleSlot>,
    artifacts: Option<&dyn ArtifactStore>,
    state: &Arc<Mutex<ControllerState>>,
    cancel: &Cancellation,
) {
    let started_at = clock.now();
    let Some(interval_id) =
        admit_idle_compaction(config, state, parking, started_at, cancel.is_cancelled())
    else {
        return;
    };

    if let Err(reason) = persist_idle_attempt_marker(session, capsule, &interval_id).await {
        record_idle_outcome(
            state,
            IdleOutcomeRecord {
                outcome: IdleCompactionOutcome::Failed,
                reason: Some(reason.to_owned()),
                started_at,
                finished_at: clock.now(),
                model: None,
                revision: None,
                usage: &UsageDelta::new(),
            },
        );
        return;
    }

    let admission = match session.try_idle_compaction().await {
        Ok(admission) => admission,
        Err(_) => {
            let reason = "runtime_error";
            let usage = UsageDelta::new();
            let revision = idle_failure_revision(reason);
            let Some(_projection_admission) = OptionalProjectionAdmission::try_begin(state) else {
                record_idle_outcome(
                    state,
                    IdleOutcomeRecord {
                        outcome: IdleCompactionOutcome::Shutdown,
                        reason: Some("shutdown".to_owned()),
                        started_at,
                        finished_at: clock.now(),
                        model: None,
                        revision: None,
                        usage: &usage,
                    },
                );
                return;
            };
            let _ = persist_idle_failure(
                session,
                capsule,
                config,
                IdleFailureProjection {
                    model: config
                        .semantic_summary_model
                        .clone()
                        .expect("admitted idle compaction has a summary model"),
                    revision: revision.clone(),
                    generated_at: clock.now(),
                    coverage: idle_summary_coverage(capsule),
                    usage: &usage,
                },
            )
            .await;
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::Failed,
                    reason: Some(reason.to_owned()),
                    started_at,
                    finished_at: clock.now(),
                    model: None,
                    revision: None,
                    usage: &usage,
                },
            );
            return;
        }
    };

    // Runtime has already committed any accepted compaction and its usage.
    // Admission and stop_scheduling serialize through the same state mutex:
    // shutdown prevents a new optional projection, while a projection that
    // won first is drained before Runtime's final session save.
    let _projection_admission = match &admission {
        IdleCompactionAdmission::Accepted { .. } => {
            match OptionalProjectionAdmission::try_begin(state) {
                Some(admission) => Some(admission),
                None => {
                    let empty = UsageDelta::new();
                    let (usage, model, revision) = match &admission {
                        IdleCompactionAdmission::Accepted {
                            summary: Some(summary),
                            ..
                        } => (
                            &summary.usage,
                            Some(summary.model_id.clone()),
                            Some(summary.summary_revision.clone()),
                        ),
                        IdleCompactionAdmission::Accepted {
                            summary: None,
                            usage,
                            ..
                        } => (usage, None, None),
                        IdleCompactionAdmission::Busy | IdleCompactionAdmission::Shutdown => {
                            (&empty, None, None)
                        }
                    };
                    record_idle_outcome(
                        state,
                        IdleOutcomeRecord {
                            outcome: IdleCompactionOutcome::Shutdown,
                            reason: Some("shutdown".to_owned()),
                            started_at,
                            finished_at: clock.now(),
                            model,
                            revision,
                            usage,
                        },
                    );
                    return;
                }
            }
        }
        IdleCompactionAdmission::Busy | IdleCompactionAdmission::Shutdown => None,
    };

    match admission {
        IdleCompactionAdmission::Accepted {
            summary: Some(summary),
            ..
        } => {
            finish_idle_summary(
                session,
                config,
                capsule,
                artifacts,
                state,
                clock,
                started_at,
                &interval_id,
                summary,
            )
            .await;
        }
        IdleCompactionAdmission::Accepted {
            summary: None,
            fallback_reason,
            usage,
        } => {
            let reason = bounded_idle_reason(fallback_reason.as_deref())
                .unwrap_or_else(|| "summary_unavailable".to_owned());
            let revision = idle_failure_revision(&reason);
            let _ = persist_idle_failure(
                session,
                capsule,
                config,
                IdleFailureProjection {
                    model: config
                        .semantic_summary_model
                        .clone()
                        .expect("admitted idle compaction has a summary model"),
                    revision: revision.clone(),
                    generated_at: clock.now(),
                    coverage: idle_summary_coverage(capsule),
                    usage: &usage,
                },
            )
            .await;
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::NoSummary,
                    reason: Some(reason),
                    started_at,
                    finished_at: clock.now(),
                    model: None,
                    revision: None,
                    usage: &usage,
                },
            );
        }
        IdleCompactionAdmission::Busy => {
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::Busy,
                    reason: Some("runtime_busy".to_owned()),
                    started_at,
                    finished_at: clock.now(),
                    model: None,
                    revision: None,
                    usage: &UsageDelta::new(),
                },
            );
        }
        IdleCompactionAdmission::Shutdown => {
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::Shutdown,
                    reason: Some("shutdown".to_owned()),
                    started_at,
                    finished_at: clock.now(),
                    model: None,
                    revision: None,
                    usage: &UsageDelta::new(),
                },
            );
        }
    }
}

async fn persist_idle_attempt_marker(
    session: &SessionHandle,
    capsule: Option<&ResumeCapsuleSlot>,
    interval_id: &str,
) -> Result<(), &'static str> {
    let Some(capsule) = capsule else {
        return Err("attempt_marker_unavailable");
    };
    let (previous, expected) = capsule
        .try_update_atomic(|capsule| {
            capsule.cache.idle_compaction_interval_id = Some(interval_id.to_owned());
            capsule.cache.idle_compaction_attempted = true;
            Ok(())
        })
        .map_err(|_| "attempt_marker_projection_failed")?;
    match session.persist().await {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = capsule.restore_if_current(&expected, previous);
            Err("attempt_marker_persist_failed")
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn finish_idle_summary(
    session: &SessionHandle,
    config: &CacheControllerConfig,
    capsule: Option<&ResumeCapsuleSlot>,
    artifacts: Option<&dyn ArtifactStore>,
    state: &Arc<Mutex<ControllerState>>,
    clock: &Arc<dyn Clock>,
    started_at: Timestamp,
    interval_id: &str,
    summary: ProtectedSemanticSummary,
) {
    // Runtime has already committed the accepted compaction (including its
    // extension state and usage) before returning this protected result. Any
    // failure below is therefore a bounded loss/warning for the optional
    // capsule projection, never a failed compaction and never a retry signal.
    let model = summary.model_id.clone();
    let revision = summary.summary_revision.clone();
    let usage = summary.usage.clone();
    let sensitivity = match artifact_sensitivity(summary.sensitivity) {
        Ok(sensitivity) => sensitivity,
        Err(reason) => {
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::Completed,
                    reason: Some(reason.to_owned()),
                    started_at,
                    finished_at: clock.now(),
                    model: Some(model),
                    revision: Some(revision),
                    usage: &usage,
                },
            );
            return;
        }
    };
    let body = summary.body.as_str();
    if body.is_empty() || body.len() > MAX_SUMMARY_BYTES {
        record_idle_outcome(
            state,
            IdleOutcomeRecord {
                outcome: IdleCompactionOutcome::Completed,
                reason: Some("summary_output_invalid".to_owned()),
                started_at,
                finished_at: clock.now(),
                model: Some(model),
                revision: Some(revision),
                usage: &usage,
            },
        );
        return;
    }
    let Some(artifacts) = artifacts else {
        record_idle_outcome(
            state,
            IdleOutcomeRecord {
                outcome: IdleCompactionOutcome::Completed,
                reason: Some("summary_artifact_store_unavailable".to_owned()),
                started_at,
                finished_at: clock.now(),
                model: Some(model),
                revision: Some(revision),
                usage: &usage,
            },
        );
        return;
    };
    let write = ArtifactWrite {
        bytes: body.as_bytes().to_vec(),
        media_type: RESUME_SUMMARY_MEDIA_TYPE.to_owned(),
        sensitivity,
        retention: ArtifactRetention::Session,
        provenance: ArtifactProvenance::new(
            session.id().clone(),
            RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
        ),
        idempotency_key: format!(
            "idle-summary:{}:{}",
            Fingerprint::of(interval_id.as_bytes()),
            revision.as_str()
        ),
    };
    let reference = match artifacts.put(write).await {
        Ok(reference) => reference,
        Err(_) => {
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::Completed,
                    reason: Some("summary_artifact_write_failed".to_owned()),
                    started_at,
                    finished_at: clock.now(),
                    model: Some(model),
                    revision: Some(revision),
                    usage: &usage,
                },
            );
            return;
        }
    };

    let coverage = idle_summary_coverage(capsule);
    let Some(capsule) = capsule else {
        record_idle_outcome(
            state,
            IdleOutcomeRecord {
                outcome: IdleCompactionOutcome::Completed,
                reason: Some("attempt_marker_unavailable".to_owned()),
                started_at,
                finished_at: clock.now(),
                model: Some(model),
                revision: Some(revision),
                usage: &usage,
            },
        );
        return;
    };
    let projection = capsule.try_update_atomic(|capsule| {
        capsule.record_ordinary_summary(
            config
                .semantic_summary_provider
                .clone()
                .expect("completed idle compaction has a summary provider"),
            model.clone(),
            revision.clone(),
            clock.now(),
            body,
            coverage.clone(),
        )?;
        if let Some(summary) = capsule.semantic_summary.as_mut() {
            summary.provenance.usage = summary_usage(&usage);
        }
        capsule.attach_summary_artifact(reference.clone())?;
        if capsule.exact_state.artifacts.len() < MAX_ARTIFACTS {
            capsule.exact_state.artifacts.push(ArtifactProjection {
                artifact: reference.id.to_string(),
                digest: Some(Fingerprint::of(reference.digest.hex.as_bytes())),
            });
        }
        Ok::<(), crate::resume_capsule::ResumeCapsuleError>(())
    });
    let (previous, expected) = match projection {
        Ok(projection) => projection,
        Err(_) => {
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::Completed,
                    reason: Some("summary_capsule_projection_failed".to_owned()),
                    started_at,
                    finished_at: clock.now(),
                    model: Some(model),
                    revision: Some(revision),
                    usage: &usage,
                },
            );
            return;
        }
    };
    if session.persist().await.is_err() {
        let _ = capsule.restore_if_current(&expected, previous);
        record_idle_outcome(
            state,
            IdleOutcomeRecord {
                outcome: IdleCompactionOutcome::Completed,
                reason: Some("capsule_persist_failed".to_owned()),
                started_at,
                finished_at: clock.now(),
                model: Some(model),
                revision: Some(revision),
                usage: &usage,
            },
        );
        return;
    }
    record_idle_outcome(
        state,
        IdleOutcomeRecord {
            outcome: IdleCompactionOutcome::Completed,
            reason: None,
            started_at,
            finished_at: clock.now(),
            model: Some(model),
            revision: Some(revision),
            usage: &usage,
        },
    );
}

fn idle_summary_coverage(capsule: Option<&ResumeCapsuleSlot>) -> Vec<SummaryCoverage> {
    let watermark = capsule
        .map(|capsule| capsule.snapshot().exact_state.watermark)
        .unwrap_or_default();
    vec![SummaryCoverage::new("canonical_events", 0, watermark)]
}

struct IdleFailureProjection<'a> {
    model: String,
    revision: RegistryRevision,
    generated_at: Timestamp,
    coverage: Vec<SummaryCoverage>,
    usage: &'a UsageDelta,
}

async fn persist_idle_failure(
    session: &SessionHandle,
    capsule: Option<&ResumeCapsuleSlot>,
    config: &CacheControllerConfig,
    failure: IdleFailureProjection<'_>,
) -> Result<(), &'static str> {
    let IdleFailureProjection {
        model,
        revision,
        generated_at,
        coverage,
        usage,
    } = failure;
    let Some(capsule) = capsule else {
        return Err("attempt_marker_unavailable");
    };
    // Establish the exact protected Runtime-summary state that this failed
    // attempt follows before publishing failure metadata. The subsequent save
    // can then prove that an unchanged Runtime state is stale and must not
    // overwrite the newer failure projection.
    session
        .persist()
        .await
        .map_err(|_| "capsule_baseline_persist_failed")?;
    let (previous, expected) = capsule
        .try_update_atomic(|capsule| {
            capsule.record_failed_ordinary_summary(
                config
                    .semantic_summary_provider
                    .clone()
                    .expect("admitted idle compaction has a summary provider"),
                model,
                revision,
                generated_at,
                coverage,
            )?;
            if let Some(summary) = capsule.semantic_summary.as_mut() {
                summary.provenance.usage = summary_usage(usage);
            }
            Ok::<(), crate::resume_capsule::ResumeCapsuleError>(())
        })
        .map_err(|_| "summary_failure_projection_failed")?;
    match session.persist().await {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = capsule.restore_if_current(&expected, previous);
            Err("capsule_persist_failed")
        }
    }
}

struct IdleOutcomeRecord<'a> {
    outcome: IdleCompactionOutcome,
    reason: Option<String>,
    started_at: Timestamp,
    finished_at: Timestamp,
    model: Option<String>,
    revision: Option<RegistryRevision>,
    usage: &'a UsageDelta,
}

fn record_idle_outcome(state: &Arc<Mutex<ControllerState>>, record: IdleOutcomeRecord<'_>) {
    let IdleOutcomeRecord {
        outcome,
        reason,
        started_at,
        finished_at,
        model,
        revision,
        usage,
    } = record;
    let mut state = state.lock().expect("cache controller state poisoned");
    let bounded_reason = reason.map(bounded_metadata_string);
    let bounded_model = model.map(bounded_metadata_string);
    let bounded_revision = revision.map(|revision| {
        RegistryRevision::new(bounded_metadata_string(revision.as_str().to_owned()))
    });
    state.snapshot.idle_compaction_outcome = Some(outcome);
    state.snapshot.idle_compaction_reason = bounded_reason.clone();
    state.snapshot.idle_compaction_latency_ms = Some(finished_at.0.saturating_sub(started_at.0));
    if let Some(model) = bounded_model {
        state.snapshot.idle_compaction_model = Some(model);
    }
    state.snapshot.idle_compaction_revision = bounded_revision;
    state.snapshot.idle_compaction_usage = summary_usage(usage);
    state.snapshot.scheduled_for = None;
    let status = bounded_reason.map_or_else(
        || format!("{outcome:?}").to_ascii_lowercase(),
        |reason| format!("{}:{reason}", format!("{outcome:?}").to_ascii_lowercase()),
    );
    let provider = state
        .snapshot
        .idle_compaction_provider
        .clone()
        .expect("idle outcome has a summary provider route");
    let model = state
        .snapshot
        .idle_compaction_model
        .clone()
        .expect("idle outcome has a summary model route");
    push_synthetic_attempt(
        &mut state.snapshot,
        SyntheticCacheAttemptProjection {
            operation: None,
            attempt: None,
            purpose: ProviderAttemptPurpose::IdleCompaction,
            provider,
            model,
            cache_identity: None,
            usage: summary_usage(usage),
            counter_provenance: counter_provenance(usage),
            cost_micro_usd: None,
            cost_provenance: SyntheticCostProvenance::Unknown,
            latency_ms: finished_at.0.saturating_sub(started_at.0),
            status: bounded_metadata_string(status),
        },
    );
}

fn record_cache_attempt_projection(
    state: &Arc<Mutex<ControllerState>>,
    result: &CacheOperationResult,
    usage: Option<&UsageDelta>,
    started_at: Timestamp,
    finished_at: Timestamp,
) {
    let usage = usage.cloned().unwrap_or_default();
    let mut state = state.lock().expect("cache controller state poisoned");
    let status = result
        .terminal_reason
        .or(result.rejection_reason)
        .map_or_else(
            || format!("{:?}", result.outcome).to_ascii_lowercase(),
            |reason| {
                format!(
                    "{}:{}",
                    format!("{:?}", result.outcome).to_ascii_lowercase(),
                    format!("{reason:?}").to_ascii_lowercase()
                )
            },
        );
    push_synthetic_attempt(
        &mut state.snapshot,
        SyntheticCacheAttemptProjection {
            operation: Some(result.operation.to_string()),
            attempt: result.attempt.as_ref().map(ToString::to_string),
            purpose: result.purpose,
            provider: bounded_metadata_string(result.identity.provider().to_owned()),
            model: bounded_metadata_string(result.identity.model().as_str().to_owned()),
            cache_identity: Some(result.identity.digest().to_string()),
            usage: summary_usage(&usage),
            counter_provenance: counter_provenance(&usage),
            cost_micro_usd: None,
            cost_provenance: SyntheticCostProvenance::Unknown,
            latency_ms: finished_at.0.saturating_sub(started_at.0),
            status: bounded_metadata_string(status),
        },
    );
}

fn push_synthetic_attempt(
    snapshot: &mut CacheControllerSnapshot,
    attempt: SyntheticCacheAttemptProjection,
) {
    if let Some(operation) = attempt.operation.as_deref()
        && let Some(existing) = snapshot
            .synthetic_attempts
            .iter_mut()
            .find(|existing| existing.operation.as_deref() == Some(operation))
    {
        *existing = attempt;
        return;
    }
    if snapshot.synthetic_attempts.len() >= MAX_SYNTHETIC_ATTEMPT_PROJECTIONS {
        snapshot.synthetic_attempts.remove(0);
    }
    snapshot.synthetic_attempts.push(attempt);
}

fn counter_provenance(usage: &UsageDelta) -> BTreeMap<CounterKind, SyntheticCounterProvenance> {
    usage
        .iter()
        .map(|(kind, _)| (kind, SyntheticCounterProvenance::ProviderReported))
        .collect()
}

fn summary_usage(usage: &UsageDelta) -> SummaryUsage {
    SummaryUsage {
        input_uncached: usage.get(CounterKind::InputUncached),
        input_cached: usage.get(CounterKind::InputCached),
        cache_write: usage.get(CounterKind::CacheWrite),
        output: usage.get(CounterKind::Output),
        reasoning: usage.get(CounterKind::Reasoning),
        cost_micro_usd: None,
        cost_is_estimate: false,
    }
}

fn artifact_sensitivity(sensitivity: Sensitivity) -> Result<ArtifactSensitivity, &'static str> {
    match sensitivity {
        Sensitivity::Public => Ok(ArtifactSensitivity::Public),
        Sensitivity::Internal | Sensitivity::Sensitive => Ok(ArtifactSensitivity::Sensitive),
        Sensitivity::Secret => Err("summary_sensitivity_invalid"),
    }
}

fn bounded_idle_reason(reason: Option<&str>) -> Option<String> {
    let reason = reason?;
    let category = match reason {
        "original_store_integrity_failed"
        | "summary_model_unavailable"
        | "summary_output_invalid"
        | "summary_usage_limit_exceeded" => reason,
        _ => "summary_fallback",
    };
    Some(category.to_owned())
}

fn idle_failure_revision(reason: &str) -> RegistryRevision {
    RegistryRevision::from_content(format!("smith-idle-compaction:{reason}"))
}

fn reconcile_parking(
    state: &Arc<Mutex<ControllerState>>,
    parking: Option<&ParkingSnapshot>,
    now: Timestamp,
) -> bool {
    let mut state = state.lock().expect("cache controller state poisoned");
    let child_parked =
        parking.is_some_and(|parking| parking.state == ParentParkingState::ParkedAwaitingChild);
    let goal_parked = state.goal_active && !state.parent_turn_active;
    if !child_parked && !goal_parked {
        state.parked_since = None;
        state.snapshot.scheduled_for = None;
        return false;
    }
    let interval = if child_parked {
        parking
            .expect("child parked state has a parking snapshot")
            .parked_interval
            .saturating_mul(2)
            .saturating_add(1)
    } else {
        state.parent_idle_interval.saturating_mul(2)
    };
    if state.snapshot.parked_interval == Some(interval) {
        return false;
    }
    state.snapshot.parked_interval = Some(interval);
    state.snapshot.interval_attempted = false;
    state.snapshot.interval_attempts = 0;
    state.boundary_evaluated = false;
    state.parked_since = Some(now);
    state
        .snapshot
        .lifecycle
        .begin_parked_interval(format!("parked-{interval}"));
    true
}

fn evaluate(
    scheduler: &CacheScheduler,
    config: &CacheControllerConfig,
    state: &Arc<Mutex<ControllerState>>,
    parking: Option<&ParkingSnapshot>,
    now: Timestamp,
) -> CacheSchedulerDecision {
    let mut state = state.lock().expect("cache controller state poisoned");
    if state.event_gap {
        return suppressed(config.policy, MaintenanceSuppressionReason::EventStreamGap);
    }
    let Some(lease) = state.snapshot.lifecycle.current().cloned() else {
        return suppressed(
            config.policy,
            MaintenanceSuppressionReason::ProviderEvidenceUnavailable,
        );
    };
    let child_parked =
        parking.is_some_and(|parking| parking.state == ParentParkingState::ParkedAwaitingChild);
    let parked = child_parked || (state.goal_active && !state.parent_turn_active);
    let child_active = parking.is_some_and(|parking| !parking.pending_children.is_empty());
    let continuation_source = child_active || state.goal_active;
    if parked
        && state.snapshot.scheduled_for.is_none()
        && !state.boundary_evaluated
        && state.snapshot.interval_attempts < config.policy.max_maintenance_calls
    {
        state.snapshot.scheduled_for = Some(schedule_boundary(
            &lease,
            config.policy,
            state.parked_since.unwrap_or(now),
            now,
        ));
    }
    if state.snapshot.interval_attempts >= config.policy.max_maintenance_calls {
        return suppressed(
            config.policy,
            MaintenanceSuppressionReason::CallBudgetExhausted,
        );
    }
    let same_provider_and_model = identity_matches_config(&lease.identity, config);
    let handoff_selected = config.policy.handoff_checkpoint
        && same_provider_and_model
        && config
            .contract
            .supports_synthetic(ProviderAttemptPurpose::CacheHandoffCheckpoint);
    let mut input = CacheSchedulerInput::new(lease.identity.clone(), now);
    let handoff_suffix_tokens = if handoff_selected {
        HANDOFF_SUFFIX.len().min(u32::MAX as usize) as u32
    } else {
        0
    };
    input.planned_input_tokens = state
        .planned_input_tokens
        .saturating_add(handoff_suffix_tokens);
    input.model_input_limit = config.model_input_limit;
    input.scheduled_for = state.snapshot.scheduled_for;
    input.parent_parked = parked;
    input.parked_since = state.parked_since;
    input.continuation_source = continuation_source;
    input.child_active = child_active;
    input.process_active = !state.shutting_down;
    input.session_active = !state.snapshot.lifecycle.shutdown;
    input.lifecycle_lease_active = !state.shutting_down;
    input.shutdown = state.shutting_down;
    input.host_synthetic_spend_allowed =
        config.synthetic_spend == SyntheticCacheSpendAuthority::Allow;
    input.continuation_expected_by =
        hard_continuation_boundary(&lease, config.policy, state.parked_since, child_active);
    input.real_parent_activity_at = state.last_meaningful_activity_at;
    input.cold_resume = lease.cold_resume;
    input.same_provider_and_model = same_provider_and_model;
    input.operation_in_flight = state.snapshot.operation_in_flight;
    input.contract = config.contract.clone();
    input.limits = scheduler_limits(&state);
    scheduler.evaluate(&lease, &input)
}

fn identity_matches_config(identity: &CacheIdentity, config: &CacheControllerConfig) -> bool {
    identity.provider() == config.provider
        && identity.model().as_str() == config.model
        && config.endpoint_identity.as_ref() == Some(identity.endpoint())
        && identity.profile() == &config.profile_identity
}

fn scheduler_limits(state: &ControllerState) -> SchedulerLimits {
    let mut limits = SchedulerLimits {
        session_total_tokens: state.goal_remaining_tokens,
        ..SchedulerLimits::default()
    };
    let Some(snapshot) = state.provider_rate_limits.as_ref() else {
        return limits;
    };
    for window in &snapshot.windows {
        let id = window
            .id
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let remaining = window.remaining;
        if id.contains("request") || id.contains("call") {
            limits.provider_attempts = min_u64_as_u32(limits.provider_attempts, remaining);
        } else if id.contains("input") && id.contains("token") {
            limits.provider_input_tokens = min_option(limits.provider_input_tokens, remaining);
        } else if id.contains("output") && id.contains("token") {
            limits.provider_output_tokens = min_option(limits.provider_output_tokens, remaining);
        } else if id.contains("token") {
            limits.provider_total_tokens = min_option(limits.provider_total_tokens, remaining);
        } else if window.is_exhausted() {
            // An unnamed exhausted provider window is enough to fail closed,
            // but a non-exhausted unnamed window cannot be assigned to a
            // fabricated request or token budget.
            limits.provider_attempts = Some(0);
        }
    }
    limits
}

fn min_option(current: Option<u64>, candidate: Option<u64>) -> Option<u64> {
    match (current, candidate) {
        (Some(current), Some(candidate)) => Some(current.min(candidate)),
        (None, candidate) => candidate,
        (current, None) => current,
    }
}

fn min_u64_as_u32(current: Option<u32>, candidate: Option<u64>) -> Option<u32> {
    min_option(current.map(u64::from), candidate).map(|value| value.min(u64::from(u32::MAX)) as u32)
}

fn record_decision(
    state: &mut ControllerState,
    decision: CacheSchedulerDecision,
    now: Timestamp,
) -> bool {
    let dispatch = decision.disposition == CacheSchedulerDisposition::Dispatch;
    let due = state
        .snapshot
        .scheduled_for
        .is_some_and(|scheduled| now >= scheduled);
    state.snapshot.decision = Some(decision.clone());
    if dispatch {
        state.snapshot.operation_in_flight = true;
        state.snapshot.interval_attempted = true;
        state.snapshot.interval_attempts = state.snapshot.interval_attempts.saturating_add(1);
        state.snapshot.scheduled_for = None;
    } else if due && decision.reason != Some(MaintenanceSuppressionReason::NotDue) {
        // A due fail-closed/observe decision is terminal for this boundary.
        // Clearing the zero-duration timer prevents a tight loop; a new real
        // interval or an admitted prior touch is required before rescheduling.
        state.boundary_evaluated = true;
        state.snapshot.interval_attempted = true;
        state.snapshot.scheduled_for = None;
    }
    dispatch
}

fn suppressed(
    policy: CacheMaintenancePolicy,
    reason: MaintenanceSuppressionReason,
) -> CacheSchedulerDecision {
    CacheSchedulerDecision {
        disposition: CacheSchedulerDisposition::Suppressed,
        action: None,
        purpose: None,
        reason: Some(reason),
        planned_input_tokens: 0,
        max_output_tokens: policy.max_maintenance_output_tokens,
        deadline_ms: policy.maintenance_deadline_ms,
    }
}

fn schedule_boundary(
    lease: &crate::cache_lifecycle::CacheLease,
    policy: CacheMaintenancePolicy,
    parked_since: Timestamp,
    now: Timestamp,
) -> Timestamp {
    let inactivity = lease
        .last_meaningful_activity_at
        .unwrap_or(parked_since)
        .plus_millis(policy.inactivity_limit_ms);
    let child_hold = (policy.max_hold_while_child_ms > 0)
        .then(|| parked_since.plus_millis(policy.max_hold_while_child_ms));
    let hard_boundary = child_hold.map_or(inactivity, |hold| hold.min(inactivity));
    let provider_boundary = lease
        .effective_guaranteed_until(now)
        .map_or(hard_boundary, |guarantee| guarantee.min(hard_boundary));
    let seed = lease
        .identity
        .digest()
        .as_str()
        .bytes()
        .fold(0u64, |seed, byte| {
            seed.wrapping_mul(131).wrapping_add(u64::from(byte))
        });
    CacheScheduler::jittered_due_time(
        provider_boundary,
        policy.keepalive_margin_ms,
        policy.keepalive_jitter_percent,
        seed,
    )
    .max(now)
}

/// Computes the hard continuation boundary used to decide whether a provider
/// guarantee already covers the whole pending parent continuation. It is the
/// minimum of meaningful inactivity and the enabled child-hold deadline; it
/// never manufactures cache evidence or extends either policy boundary.
fn hard_continuation_boundary(
    lease: &crate::cache_lifecycle::CacheLease,
    policy: CacheMaintenancePolicy,
    parked_since: Option<Timestamp>,
    child_active: bool,
) -> Option<Timestamp> {
    let inactivity = lease
        .last_meaningful_activity_at
        .or(parked_since)
        .map(|activity| activity.plus_millis(policy.inactivity_limit_ms))?;
    let child_hold = if child_active && policy.max_hold_while_child_ms > 0 {
        parked_since.map(|parked| parked.plus_millis(policy.max_hold_while_child_ms))
    } else {
        None
    };
    Some(child_hold.map_or(inactivity, |hold| hold.min(inactivity)))
}

async fn dispatch_once(
    session: &SessionHandle,
    config: &CacheControllerConfig,
    clock: &Arc<dyn Clock>,
    state: &Arc<Mutex<ControllerState>>,
    cancel: &Cancellation,
    decision: &CacheSchedulerDecision,
) -> Result<CacheOperationResult, String> {
    let identity = state
        .lock()
        .expect("cache controller state poisoned")
        .snapshot
        .lifecycle
        .current_identity
        .clone()
        .ok_or_else(|| "cache_identity_unavailable".to_owned())?;
    let interval = state
        .lock()
        .expect("cache controller state poisoned")
        .snapshot
        .parked_interval
        .unwrap_or_default();
    let action = decision
        .action
        .ok_or_else(|| "cache_action_unavailable".to_owned())?;
    let purpose = action.purpose();
    let operation = CacheOperationId::new(format!(
        "smith-cache-{interval}-{}-{}",
        purpose.as_str(),
        &identity.digest().as_str()[..identity.digest().as_str().len().min(16)]
    ));
    let authority = CacheAuthority::new(format!("smith-cache-authority:{}", session.id()));
    let budget = CacheOperationBudget {
        max_input_tokens: if config.policy.max_maintenance_input_tokens == 0 {
            config.model_input_limit
        } else {
            config
                .policy
                .max_maintenance_input_tokens
                .min(config.model_input_limit)
        },
        max_output_bytes: 16 * 1024,
        max_output_tokens: config.policy.max_maintenance_output_tokens,
    };
    let operation_cancel = cancel.child();
    let deadline = Deadline::after(clock.as_ref(), config.policy.maintenance_deadline_ms);
    let request = match action {
        CacheMaintenanceAction::HandoffCheckpoint => session.cache_handoff_from_last_plan(
            operation,
            CacheHandoffSuffix::new(HANDOFF_SUFFIX).map_err(|_| "handoff_suffix_invalid")?,
            authority,
            budget,
            operation_cancel,
            deadline,
        ),
        CacheMaintenanceAction::Keepalive => session.cache_operation_from_last_plan(
            operation,
            ProviderAttemptPurpose::CacheKeepalive,
            authority,
            budget,
            operation_cancel,
            deadline,
        ),
    }
    .map_err(|error| format!("cache_preflight:{:?}", error.kind))?;
    session
        .dispatch_cache_operation(request)
        .await
        .map_err(|error| format!("cache_dispatch:{:?}", error.kind))
}

async fn persist_handoff(
    session: &SessionHandle,
    config: &CacheControllerConfig,
    capsule: Option<&ResumeCapsuleSlot>,
    artifacts: Option<&dyn ArtifactStore>,
    result: &CacheOperationResult,
    expected_identity: &CacheIdentity,
    now: Timestamp,
) -> Result<(), String> {
    if &result.identity != expected_identity || !identity_matches_config(&result.identity, config) {
        return Err("handoff_identity_mismatch".to_owned());
    }
    let Some(capsule) = capsule else {
        return Ok(());
    };
    let Some(_identity_lease) = session.lock_current_cache_identity(&result.identity).await else {
        return Err("handoff_identity_retired".to_owned());
    };
    if result.purpose != ProviderAttemptPurpose::CacheHandoffCheckpoint
        || result.outcome != CacheOperationOutcome::Completed
    {
        let (previous, expected) = capsule
            .try_update_atomic(|candidate| {
                project_handoff_cache_state(candidate, result);
                Ok(())
            })
            .map_err(|error| error.to_string())?;
        return persist_capsule_after_update(session, capsule, &expected, previous)
            .await
            .map_err(|_| "capsule_persist_failed".to_owned());
    }
    // Persist the Runtime state that this handoff supersedes before changing
    // summary purpose. RedactingSessionStore records its protected artifact
    // reference, allowing the post-handoff save to distinguish the same stale
    // ordinary state from a genuinely newer Runtime summary.
    session
        .persist()
        .await
        .map_err(|_| "capsule_baseline_persist_failed".to_owned())?;
    let revision = RegistryRevision::from_content(format!(
        "{}\n{}",
        result.operation,
        result
            .captured_output
            .as_ref()
            .map_or("", |output| output.as_str().trim())
    ));
    let Some(output) = result.captured_output.as_ref() else {
        return persist_failed_handoff(
            session,
            capsule,
            config,
            result,
            revision,
            now,
            "handoff_output_missing",
        )
        .await;
    };
    let body = output.as_str().trim();
    if body.is_empty() || body.len() > MAX_SUMMARY_BYTES {
        return persist_failed_handoff(
            session,
            capsule,
            config,
            result,
            revision,
            now,
            "handoff_output_invalid",
        )
        .await;
    }
    let Some(artifacts) = artifacts else {
        return persist_failed_handoff(
            session,
            capsule,
            config,
            result,
            revision,
            now,
            "handoff_artifact_store_unavailable",
        )
        .await;
    };
    let write = ArtifactWrite {
        bytes: body.as_bytes().to_vec(),
        media_type: RESUME_SUMMARY_MEDIA_TYPE.to_owned(),
        sensitivity: ArtifactSensitivity::Sensitive,
        retention: ArtifactRetention::Session,
        provenance: ArtifactProvenance::new(session.id().clone(), RESUME_SUMMARY_ARTIFACT_PURPOSE),
        idempotency_key: result.operation.to_string(),
    };
    let reference = match artifacts.put(write).await {
        Ok(reference) => reference,
        Err(_) => {
            return persist_failed_handoff(
                session,
                capsule,
                config,
                result,
                revision,
                now,
                "handoff_artifact_write_failed",
            )
            .await;
        }
    };
    let (previous, expected) = capsule
        .try_update_atomic(|candidate| {
            project_handoff_cache_state(candidate, result);
            let coverage = vec![SummaryCoverage::new(
                "canonical_events",
                0,
                candidate.exact_state.watermark,
            )];
            candidate.record_handoff_summary(
                config.provider.clone(),
                config.model.clone(),
                revision,
                result.identity.clone(),
                now,
                body,
                coverage,
            )?;
            candidate.attach_summary_artifact(reference.clone())?;
            let artifact_id = reference.id.to_string();
            if candidate.exact_state.artifacts.len() < MAX_ARTIFACTS
                && !candidate
                    .exact_state
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.artifact == artifact_id)
            {
                let projection = ArtifactProjection {
                    artifact: artifact_id,
                    digest: Some(Fingerprint::of(reference.digest.hex.as_bytes())),
                };
                candidate.exact_state.artifacts.push(projection);
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    persist_capsule_after_update(session, capsule, &expected, previous)
        .await
        .map_err(|_| "capsule_persist_failed".to_owned())
}

fn project_handoff_cache_state(capsule: &mut ResumeCapsule, result: &CacheOperationResult) {
    capsule.cache.prior_identity = Some(result.identity.clone());
    capsule.cache.provider_warmth = match result.state {
        agent_runtime_core::event::CacheState::WarmObserved => ResumeCacheWarmth::WarmObserved,
        agent_runtime_core::event::CacheState::MissObserved => ResumeCacheWarmth::MissObserved,
        agent_runtime_core::event::CacheState::Expired => ResumeCacheWarmth::ExpiredObserved,
        _ => ResumeCacheWarmth::Unknown,
    };
}

#[allow(clippy::too_many_arguments)]
async fn persist_failed_handoff(
    session: &SessionHandle,
    capsule: &ResumeCapsuleSlot,
    config: &CacheControllerConfig,
    result: &CacheOperationResult,
    revision: RegistryRevision,
    now: Timestamp,
    reason: &'static str,
) -> Result<(), String> {
    let (previous, expected) = capsule
        .try_update_atomic(|candidate| {
            project_handoff_cache_state(candidate, result);
            let coverage = vec![SummaryCoverage::new(
                "canonical_events",
                0,
                candidate.exact_state.watermark,
            )];
            candidate.record_failed_handoff_summary(
                config.provider.clone(),
                config.model.clone(),
                revision,
                result.identity.clone(),
                now,
                coverage,
            )
        })
        .map_err(|error| error.to_string())?;
    persist_capsule_after_update(session, capsule, &expected, previous)
        .await
        .map_err(|_| "capsule_persist_failed".to_owned())?;
    Err(reason.to_owned())
}

/// Persists one already-mutated capsule and restores its previous projection
/// when the store rejects the write.  The conditional restore prevents a
/// concurrent newer event from being erased by an older failed save.
async fn persist_capsule_after_update(
    session: &SessionHandle,
    capsule: &ResumeCapsuleSlot,
    expected: &ResumeCapsule,
    previous: ResumeCapsule,
) -> Result<(), ()> {
    match session.persist().await {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = capsule.restore_if_current(expected, previous);
            Err(())
        }
    }
}

fn reduce_event(
    state: &Arc<Mutex<ControllerState>>,
    capsule: Option<&ResumeCapsuleSlot>,
    changes: &ChangeRecorder,
    envelope: &EventEnvelope,
) -> bool {
    let mut state = state.lock().expect("cache controller state poisoned");
    if state
        .snapshot
        .lifecycle
        .last_event_sequence
        .is_some_and(|last| envelope.seq <= last)
    {
        let _ = state.snapshot.lifecycle.apply(envelope);
        return false;
    }
    if state
        .snapshot
        .lifecycle
        .last_event_sequence
        .is_some_and(|last| envelope.seq > last.saturating_add(1))
    {
        state.event_gap = true;
        state.boundary_evaluated = true;
        state.snapshot.scheduled_for = None;
        state.snapshot.last_error = Some("runtime_event_gap".to_owned());
    }
    let synthetic_state_attempt = match &envelope.payload {
        RuntimeEvent::CacheStateChanged { attempt, .. } => {
            state.synthetic_attempts.contains(attempt)
        }
        _ => false,
    };
    state.snapshot.lifecycle.apply(envelope);
    let mut persist = false;
    match &envelope.payload {
        RuntimeEvent::TurnStarted | RuntimeEvent::InternalTurnStarted { .. } => {
            state.parent_turn_active = true;
            state.last_meaningful_activity_at = Some(envelope.timestamp);
            state
                .snapshot
                .lifecycle
                .record_parent_activity(envelope.timestamp);
        }
        RuntimeEvent::TurnSteerCommitted { .. }
        | RuntimeEvent::ToolCallRequested { .. }
        | RuntimeEvent::ToolCallCompleted { .. } => {
            state.last_meaningful_activity_at = Some(envelope.timestamp);
            state
                .snapshot
                .lifecycle
                .record_parent_activity(envelope.timestamp);
        }
        RuntimeEvent::ContextPlanned { input_tokens, .. } => {
            state.planned_input_tokens = *input_tokens;
        }
        RuntimeEvent::CachePlanChanged {
            preserved_prefix_tokens,
            ..
        } => {
            state.plan_has_comparable_predecessor = *preserved_prefix_tokens > 0;
        }
        RuntimeEvent::CacheStateChanged {
            cache_identity: Some(identity),
            ..
        } if !synthetic_state_attempt => {
            state
                .snapshot
                .lifecycle
                .record_parent_request(identity, envelope.timestamp);
            if let Some(activity) = state.last_meaningful_activity_at
                && let Some(lease) = state.snapshot.lifecycle.current_mut()
            {
                lease.record_meaningful_activity(activity);
            }
            persist = true;
        }
        RuntimeEvent::CacheOperationStarted {
            attempt: Some(attempt),
            ..
        } => {
            state.synthetic_attempts.insert(attempt.clone());
        }
        RuntimeEvent::CacheOperationCompleted {
            attempt: Some(attempt),
            ..
        } => {
            state.synthetic_attempts.remove(attempt);
            persist = true;
        }
        RuntimeEvent::GoalUpdated { goal, .. } => {
            state.goal_active = goal.as_ref().is_some_and(|goal| goal.status.is_active());
            state.goal_remaining_tokens = goal.as_ref().and_then(|goal| {
                goal.token_budget
                    .zip(goal.usage.charged_tokens)
                    .map(|(budget, used)| budget.saturating_sub(used))
            });
            persist = true;
        }
        RuntimeEvent::RateLimitObservation { snapshot, .. } => {
            state.provider_rate_limits = Some(snapshot.clone());
        }
        RuntimeEvent::PlanUpdated { .. }
        | RuntimeEvent::ChildSpawned { .. }
        | RuntimeEvent::ChildNeedsInput { .. }
        | RuntimeEvent::ChildCompleted { .. }
        | RuntimeEvent::ChildStopped { .. }
        | RuntimeEvent::ChildFailed { .. }
        | RuntimeEvent::InteractionRequested { .. }
        | RuntimeEvent::InteractionResolved { .. }
        | RuntimeEvent::ContextCompacted { .. } => persist = true,
        RuntimeEvent::TurnCompleted { .. } => {
            state.parent_turn_active = false;
            state.parent_idle_interval = state.parent_idle_interval.saturating_add(1);
            state.last_meaningful_activity_at = Some(envelope.timestamp);
            state
                .snapshot
                .lifecycle
                .record_parent_activity(envelope.timestamp);
            let interval_id = envelope
                .turn
                .as_ref()
                .map(|turn| format!("root-turn:{}", turn.as_str()))
                .unwrap_or_else(|| format!("root-event:{}", envelope.seq));
            state.snapshot.idle_compaction.reset(interval_id);
            persist = true;
        }
        RuntimeEvent::SessionShutdown => state.shutting_down = true,
        _ => {}
    }
    drop(state);
    if let Some(capsule) = capsule {
        update_capsule(capsule, changes, envelope, !synthetic_state_attempt);
    }
    persist
}

fn update_capsule(
    slot: &ResumeCapsuleSlot,
    changes: &ChangeRecorder,
    envelope: &EventEnvelope,
    real_parent_cache_state: bool,
) {
    slot.update(|capsule| {
        capsule.exact_state.watermark = capsule.exact_state.watermark.max(envelope.seq);
        if let Some(turn) = &envelope.turn {
            capsule.parent_turn_id = Some(turn.clone());
            capsule.exact_state.parent_turn_id = Some(turn.clone());
        }
        if matches!(
            &envelope.payload,
            RuntimeEvent::TurnStarted
                | RuntimeEvent::InternalTurnStarted { .. }
                | RuntimeEvent::TurnSteerCommitted { .. }
                | RuntimeEvent::ToolCallRequested { .. }
                | RuntimeEvent::ToolCallCompleted { .. }
        ) {
            capsule.cache.last_meaningful_activity_at = Some(envelope.timestamp);
        }
        match &envelope.payload {
            RuntimeEvent::GoalUpdated { goal, .. } => {
                capsule.exact_state.goal = goal.as_ref().map(|goal| ExactGoalProjection {
                    goal_id: Some(goal.id.to_string()),
                    generation: goal.generation,
                    state: Some(goal.status.as_str().to_owned()),
                });
            }
            RuntimeEvent::PlanUpdated {
                revision, counts, ..
            } => {
                capsule.exact_state.plan = Some(ExactPlanProjection {
                    revision: *revision,
                    pending: count(counts, "pending"),
                    completed: count(counts, "completed"),
                    failed: count(counts, "failed").saturating_add(count(counts, "cancelled")),
                });
            }
            RuntimeEvent::ChildSpawned { child, .. } => {
                if metadata_fits(child.as_str())
                    && (capsule.exact_state.children.contains_key(child)
                        || capsule.exact_state.children.len() < MAX_CHILDREN)
                {
                    capsule.exact_state.children.insert(
                        child.clone(),
                        ChildResumeProjection {
                            child: child.clone(),
                            task_digest: None,
                            state: ChildLifecycleState::Running,
                            terminal_outcome: None,
                            watermark: envelope.seq,
                        },
                    );
                }
            }
            RuntimeEvent::ChildNeedsInput { child, request, .. } => {
                update_child(
                    &mut capsule.exact_state.children,
                    child,
                    ChildLifecycleState::NeedsInput,
                    Some(Fingerprint::of(request.as_str())),
                    envelope.seq,
                );
            }
            RuntimeEvent::ChildCompleted { child, result } => {
                update_child(
                    &mut capsule.exact_state.children,
                    child,
                    ChildLifecycleState::Completed,
                    Some(Fingerprint::of(result.as_bytes())),
                    envelope.seq,
                );
            }
            RuntimeEvent::ChildStopped { child, .. } => update_child(
                &mut capsule.exact_state.children,
                child,
                ChildLifecycleState::Stopped,
                None,
                envelope.seq,
            ),
            RuntimeEvent::ChildFailed { child, .. } => update_child(
                &mut capsule.exact_state.children,
                child,
                ChildLifecycleState::Failed,
                None,
                envelope.seq,
            ),
            RuntimeEvent::InteractionRequested { .. } => {
                capsule.exact_state.unresolved_decisions =
                    capsule.exact_state.unresolved_decisions.saturating_add(1);
            }
            RuntimeEvent::InteractionResolved { .. } => {
                capsule.exact_state.unresolved_decisions =
                    capsule.exact_state.unresolved_decisions.saturating_sub(1);
            }
            RuntimeEvent::ToolCallCompleted {
                call,
                name,
                is_error,
            } if matches!(name.as_str(), "shell" | "task_output") => {
                let key = call.to_string();
                if metadata_fits(&key)
                    && (capsule.exact_state.validations.contains_key(&key)
                        || capsule.exact_state.validations.len() < MAX_VALIDATIONS)
                {
                    capsule.exact_state.validations.insert(
                        key.clone(),
                        ValidationProjection {
                            validation: key,
                            exit_status: Some(i32::from(*is_error)),
                            watermark: envelope.seq,
                        },
                    );
                }
            }
            RuntimeEvent::CacheStateChanged {
                cache_identity,
                state,
                ..
            } => {
                if real_parent_cache_state {
                    capsule.retire_handoff_if_identity_changed(cache_identity.as_ref());
                    capsule.cache.prior_identity = cache_identity.clone();
                }
                capsule.cache.provider_warmth = match state {
                    agent_runtime_core::event::CacheState::WarmObserved => {
                        ResumeCacheWarmth::WarmObserved
                    }
                    agent_runtime_core::event::CacheState::MissObserved => {
                        ResumeCacheWarmth::MissObserved
                    }
                    agent_runtime_core::event::CacheState::Expired => {
                        ResumeCacheWarmth::ExpiredObserved
                    }
                    _ => ResumeCacheWarmth::Unknown,
                };
                if real_parent_cache_state {
                    capsule.cache.cold_resume = false;
                }
            }
            RuntimeEvent::CacheAvailabilityEvidenceRecorded { evidence } => {
                if let Some(guaranteed_until) = evidence.guaranteed_until {
                    capsule.cache.guaranteed_until = Some(guaranteed_until);
                }
            }
            RuntimeEvent::TurnCompleted { .. } => {
                capsule.cache.last_meaningful_activity_at = Some(envelope.timestamp);
                capsule.cache.idle_compaction_interval_id = Some(
                    envelope
                        .turn
                        .as_ref()
                        .map(|turn| format!("root-turn:{}", turn.as_str()))
                        .unwrap_or_else(|| format!("root-event:{}", envelope.seq)),
                );
                capsule.cache.idle_compaction_attempted = false;
                if let Some(turn) = &envelope.turn {
                    let _ = capsule.push_recent_turn(RecentTurnProjection {
                        turn: turn.clone(),
                        role: RecentTurnRole::Assistant,
                        content_digest: None,
                    });
                }
                if let Some(change_set) = changes.latest() {
                    for mutation in change_set.mutations {
                        if let ToolMutation::Exact(edit) = mutation {
                            let path = edit.path.to_string_lossy().into_owned();
                            if metadata_fits(&path)
                                && capsule.exact_state.changed_files.len() < MAX_CHANGED_FILES
                                && !capsule
                                    .exact_state
                                    .changed_files
                                    .iter()
                                    .any(|changed| changed.path == path)
                            {
                                capsule
                                    .exact_state
                                    .changed_files
                                    .push(ChangedFileProjection {
                                        path,
                                        additions: 0,
                                        deletions: 0,
                                        digest: Some(Fingerprint::of(edit.after_hash)),
                                    });
                            }
                        }
                    }
                }
            }
            RuntimeEvent::ContextCompacted { .. } => {
                capsule.retire_handoff_if_identity_changed(None);
                capsule.cache.prior_identity = None;
                capsule.cache.provider_warmth = ResumeCacheWarmth::Unknown;
                capsule.cache.guaranteed_until = None;
                capsule.cache.cold_resume = true;
            }
            _ => {}
        }
    });
}

fn update_child(
    children: &mut BTreeMap<ChildId, ChildResumeProjection>,
    child: &ChildId,
    state: ChildLifecycleState,
    digest: Option<Fingerprint>,
    watermark: u64,
) {
    if !metadata_fits(child.as_str())
        || (!children.contains_key(child) && children.len() >= MAX_CHILDREN)
    {
        return;
    }
    let projection = children
        .entry(child.clone())
        .or_insert_with(|| ChildResumeProjection {
            child: child.clone(),
            task_digest: None,
            state,
            terminal_outcome: None,
            watermark,
        });
    projection.state = state;
    projection.watermark = watermark;
    projection.terminal_outcome = Some(ChildTerminalOutcome {
        state,
        result_digest: digest,
        watermark,
    });
}

fn metadata_fits(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_METADATA_BYTES
}

fn bounded_metadata_string(value: impl Into<String>) -> String {
    let value = value.into();
    if value.len() <= MAX_METADATA_BYTES {
        return value;
    }
    let mut end = MAX_METADATA_BYTES;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_owned()
}

fn count(counts: &BTreeMap<String, u32>, key: &str) -> u32 {
    counts.get(key).copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
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
        initial.snapshot.lifecycle.install_plan(
            parent_identity.clone(),
            true,
            false,
            0,
            Timestamp(0),
        );
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

        record_cache_attempt_projection(
            &state,
            &result,
            Some(&usage),
            Timestamp(10),
            Timestamp(42),
        );

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
}
