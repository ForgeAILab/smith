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

mod handoff;
mod idle_compaction;
mod resume_projection;
mod synthetic_projection;

use handoff::*;
use idle_compaction::*;
use resume_projection::*;
use synthetic_projection::*;

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

#[cfg(test)]
mod tests;
