//! Pure Smith policy for the provider-cache lifecycle.
//!
//! Agent Runtime owns the provider mechanism and its event vocabulary.  This
//! module is deliberately a consumer projection: it consumes the opaque
//! [`agent_runtime_core::cache::CacheIdentity`] and canonical cache events,
//! keeps the parent activity and provider-touch clocks separate, and returns
//! bounded scheduler decisions.  It never builds a provider request and it
//! never emits a `RuntimeEvent` of its own.

use std::collections::BTreeMap;

use agent_runtime_core::cache::{
    CacheAvailabilityEvidence, CacheEvidenceKind, CacheEvidenceSource, CacheIdentity,
    ProviderAttemptPurpose, ProviderCacheBehavior, ProviderCacheContract,
};
use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::event::{
    CacheOperationOutcome, CacheOperationReason, CacheState, EventEnvelope, RuntimeEvent,
};
use agent_runtime_core::usage::{CounterKind, UsageRecord};
use serde::{Deserialize, Serialize};

/// The bounded default meaningful-inactivity window from the Smith policy.
pub const DEFAULT_INACTIVITY_LIMIT_MS: u64 = 60 * 60 * 1_000;
/// The bounded default child hold window from the Smith policy.
pub const DEFAULT_MAX_HOLD_WHILE_CHILD_MS: u64 = 60 * 60 * 1_000;
/// The default number of synthetic calls allowed per parked interval.
pub const DEFAULT_MAX_MAINTENANCE_CALLS: u32 = 1;
/// The default synthetic output limit.
pub const DEFAULT_MAX_MAINTENANCE_OUTPUT_TOKENS: u32 = 256;
/// The default synthetic deadline.
pub const DEFAULT_MAINTENANCE_DEADLINE_MS: u64 = 30_000;
/// Maximum current-plus-retired exact identities retained in status/session
/// projections. Runtime events remain the canonical historical record.
pub const MAX_CACHE_LEASES: usize = 32;

/// Provider evidence state projected by a Smith cache lease.
///
/// `Suspended` is a Smith policy state.  The last provider-observed state is
/// retained in [`CacheLease::observed_status`] so a miss/expiry is still
/// explainable after synthetic work is stopped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheLeaseStatus {
    /// The provider/model cannot reuse a stable prefix.
    Unsupported,
    /// No current provider evidence is available.
    #[default]
    Unknown,
    /// A first request or request without a comparable predecessor is
    /// structurally eligible, but has no warmth evidence.
    Eligible,
    /// Provider evidence observed a reusable cache read or resource.
    WarmObserved,
    /// Provider evidence observed a miss against Runtime's comparable
    /// expectation.
    MissObserved,
    /// Provider evidence explicitly reported expiry or absence.
    ExpiredObserved,
    /// Smith has suspended further synthetic maintenance for this identity.
    Suspended,
}

impl From<CacheState> for CacheLeaseStatus {
    fn from(value: CacheState) -> Self {
        match value {
            CacheState::Unsupported => Self::Unsupported,
            CacheState::Unknown => Self::Unknown,
            CacheState::Eligible => Self::Eligible,
            CacheState::WarmObserved => Self::WarmObserved,
            CacheState::MissObserved => Self::MissObserved,
            CacheState::Expired => Self::ExpiredObserved,
            CacheState::Suspended => Self::Suspended,
        }
    }
}

/// Bounded, redaction-safe reason a lease stopped synthetic maintenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseSuspensionReason {
    /// Runtime reported a comparable cache miss.
    CacheMiss,
    /// Runtime received typed provider expiry/absence evidence.
    CacheExpired,
    /// The exact identity changed and the prior state cannot transfer.
    IdentityChanged,
    /// Idle compaction retired the old plan.
    Compacted,
    /// Provider warmth is intentionally unknown after process resume.
    ColdResume,
    /// The session lifecycle was released.
    Shutdown,
    /// The adapter/provider violated a synthetic-operation contract.
    ProtocolViolation,
    /// Runtime changed or rejected the cache capability.
    CapabilityChanged,
    /// An explicit local policy boundary stopped maintenance.
    PolicyBoundary,
}

impl From<CacheOperationReason> for LeaseSuspensionReason {
    fn from(value: CacheOperationReason) -> Self {
        match value {
            CacheOperationReason::CacheMiss => Self::CacheMiss,
            CacheOperationReason::CacheExpired => Self::CacheExpired,
            CacheOperationReason::ProtocolViolation => Self::ProtocolViolation,
            CacheOperationReason::CapabilityChanged => Self::CapabilityChanged,
            CacheOperationReason::IdentityChanged => Self::IdentityChanged,
            CacheOperationReason::Shutdown => Self::Shutdown,
            _ => Self::PolicyBoundary,
        }
    }
}

/// Structural cache planning facts.  These are intentionally separate from
/// provider evidence and are safe to render when the provider reports no
/// cache counters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralCacheProjection {
    /// Runtime's opaque cache-plan fingerprint, when one was observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_plan: Option<String>,
    /// Tokens structurally preserved by context planning.
    pub preserved_prefix_tokens: u32,
    /// Tokens structurally invalidated by context planning.
    pub invalidated_prefix_tokens: u32,
    /// Whether the resolved provider can reuse a stable prefix.
    pub provider_cache_supported: bool,
    /// Whether Runtime had a comparable predecessor for this plan.
    pub has_comparable_predecessor: bool,
}

/// Canonical cache-operation stage retained by Smith's consumer projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheOperationStage {
    /// Runtime prepared the immutable operation envelope.
    Prepared,
    /// Runtime rejected the operation before provider I/O.
    Rejected,
    /// Runtime crossed the provider admission boundary.
    Started,
    /// Runtime reached a terminal operation result.
    Completed,
    /// Runtime suspended further maintenance for the identity.
    Suspended,
}

/// Evidence-bearing state for one exact opaque provider cache identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheLease {
    /// The exact Runtime-owned cache identity.  Smith never recomputes it.
    pub identity: CacheIdentity,
    /// Current Smith policy state.
    pub status: CacheLeaseStatus,
    /// Last provider state before local suspension, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_status: Option<CacheLeaseStatus>,
    /// Provider-declared minimum-retention boundary.  This is evidence, not
    /// a TTL guessed from elapsed time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guaranteed_until: Option<Timestamp>,
    /// The last accepted provider request/resource operation for this exact
    /// identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_cache_touch_at: Option<Timestamp>,
    /// Last provider evidence of a cache read/hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_hit_at: Option<Timestamp>,
    /// Last provider evidence of a cache write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_write_at: Option<Timestamp>,
    /// Last canonical miss evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_miss_at: Option<Timestamp>,
    /// Parent-only meaningful activity clock.  Child and synthetic activity
    /// never updates this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_meaningful_activity_at: Option<Timestamp>,
    /// Current parked interval, if the parent is parked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parked_interval_id: Option<String>,
    /// Accepted synthetic maintenance calls in the current parked interval.
    pub maintenance_calls: u32,
    /// Input tokens attributed to synthetic maintenance in the interval.
    pub maintenance_input_tokens: u64,
    /// Output tokens attributed to synthetic maintenance in the interval.
    pub maintenance_output_tokens: u64,
    /// Provider-reported/calculated cost for presentation only.  The
    /// scheduler never reads this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance_cost: Option<u128>,
    /// Why synthetic maintenance is currently suspended, when suspended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspension_reason: Option<LeaseSuspensionReason>,
    /// Runtime's comparable read expectation for the latest attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_read_tokens: Option<u64>,
    /// Provider read field for the latest attributed attempt. `Some(0)` is
    /// distinct from omitted evidence (`None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_read_tokens: Option<u64>,
    /// Provider write field for the latest attributed attempt. `Some(0)` is
    /// distinct from omitted evidence (`None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_write_tokens: Option<u64>,
    /// Runtime-derived shortfall, when canonical evidence supplied it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missed_tokens: Option<u64>,
    /// Structural prefix count associated with the current plan.
    pub structurally_preserved_prefix_tokens: u32,
    /// Historical identities remain inspectable but cannot authorize work.
    #[serde(default)]
    pub retired: bool,
    /// A cold resume deliberately begins with unknown provider warmth and no
    /// prewarm permission.  It is cleared only by a real matching request.
    #[serde(default)]
    pub cold_resume: bool,
    /// Whether a natural matching parent request established this identity.
    /// New identities and cold-resume baselines cannot be synthetically
    /// prewarmed before this becomes true.
    #[serde(default)]
    pub real_request_observed: bool,
    /// Latest canonical operation stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_operation_stage: Option<CacheOperationStage>,
    /// Latest canonical operation identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_operation_id: Option<String>,
    /// Latest canonical operation purpose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_operation_purpose: Option<ProviderAttemptPurpose>,
    /// Latest canonical terminal outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_operation_outcome: Option<CacheOperationOutcome>,
    /// Latest canonical rejection or terminal reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_operation_reason: Option<CacheOperationReason>,
    /// Latest bounded canonical operation metrics.
    #[serde(default)]
    pub last_operation_metrics: BTreeMap<String, u64>,
}

impl CacheLease {
    /// Creates a lease with an explicitly supplied initial status.
    pub fn new(identity: CacheIdentity, status: CacheLeaseStatus) -> Self {
        Self {
            identity,
            status,
            observed_status: None,
            guaranteed_until: None,
            last_cache_touch_at: None,
            last_hit_at: None,
            last_write_at: None,
            last_miss_at: None,
            last_meaningful_activity_at: None,
            parked_interval_id: None,
            maintenance_calls: 0,
            maintenance_input_tokens: 0,
            maintenance_output_tokens: 0,
            maintenance_cost: None,
            suspension_reason: None,
            expected_read_tokens: None,
            observed_read_tokens: None,
            observed_write_tokens: None,
            missed_tokens: None,
            structurally_preserved_prefix_tokens: 0,
            retired: false,
            cold_resume: false,
            real_request_observed: false,
            last_operation_stage: None,
            last_operation_id: None,
            last_operation_purpose: None,
            last_operation_outcome: None,
            last_operation_reason: None,
            last_operation_metrics: BTreeMap::new(),
        }
    }

    /// Creates the conservative state for a newly resolved Runtime plan.
    pub fn from_plan(
        identity: CacheIdentity,
        provider_supported: bool,
        has_comparable_predecessor: bool,
    ) -> Self {
        let status = if !provider_supported {
            CacheLeaseStatus::Unsupported
        } else if has_comparable_predecessor {
            CacheLeaseStatus::Unknown
        } else {
            CacheLeaseStatus::Eligible
        };
        Self::new(identity, status)
    }

    /// Returns the opaque Runtime identity.
    pub fn identity(&self) -> &CacheIdentity {
        &self.identity
    }

    /// Returns the guarantee only while its provider-declared boundary is in
    /// the future.  Passing that boundary never creates miss/expiry evidence.
    pub fn effective_guaranteed_until(&self, now: Timestamp) -> Option<Timestamp> {
        self.guaranteed_until.filter(|boundary| now < *boundary)
    }

    /// Records real parent user/provider/tool activity without touching the
    /// provider cache clock.
    pub fn record_meaningful_activity(&mut self, at: Timestamp) {
        self.last_meaningful_activity_at = max_timestamp(self.last_meaningful_activity_at, at);
    }

    /// Records a provider request accepted under this exact identity.  This
    /// does not update the meaningful-activity clock.
    pub fn record_cache_touch(&mut self, at: Timestamp) {
        self.last_cache_touch_at = max_timestamp(self.last_cache_touch_at, at);
    }

    /// Records a real parent request, updating both clocks and clearing the
    /// no-prewarm marker left by a cold resume.
    pub fn record_real_parent_request(&mut self, at: Timestamp) {
        self.record_meaningful_activity(at);
        self.record_cache_touch(at);
        self.real_request_observed = true;
        if self.suspension_reason == Some(LeaseSuspensionReason::ColdResume) {
            self.suspension_reason = None;
            self.cold_resume = false;
        }
    }

    /// Records real parent tool work.  Tool work is meaningful activity but
    /// not a cache touch.
    pub fn record_parent_tool_activity(&mut self, at: Timestamp) {
        self.record_meaningful_activity(at);
    }

    /// Starts a new parked interval and resets its one-call maintenance
    /// allowance.  Replaying the same interval id is idempotent.
    pub fn begin_parked_interval(&mut self, interval_id: impl Into<String>) {
        let interval_id = interval_id.into();
        if self.parked_interval_id.as_deref() == Some(interval_id.as_str()) {
            return;
        }
        self.parked_interval_id = Some(interval_id);
        self.maintenance_calls = 0;
        self.maintenance_input_tokens = 0;
        self.maintenance_output_tokens = 0;
        self.maintenance_cost = None;
    }

    /// Whether an accepted synthetic call may be attributed to this lease.
    pub fn maintenance_allowed(&self) -> bool {
        !self.retired
            && !self.cold_resume
            && self.real_request_observed
            && self.suspension_reason.is_none()
    }

    /// Counts one provider-admitted maintenance attempt.  A pre-I/O Runtime
    /// rejection must not call this method.
    pub fn record_maintenance_call(&mut self, at: Timestamp) {
        self.maintenance_calls = self.maintenance_calls.saturating_add(1);
        self.record_cache_touch(at);
    }

    /// Adds actual attributed maintenance usage.  Estimates must not be
    /// passed here.
    pub fn record_maintenance_usage(&mut self, input_tokens: u64, output_tokens: u64) {
        self.maintenance_input_tokens = self.maintenance_input_tokens.saturating_add(input_tokens);
        self.maintenance_output_tokens =
            self.maintenance_output_tokens.saturating_add(output_tokens);
    }

    /// Adds a provider-reported/calculated cost for presentation only.
    pub fn record_maintenance_cost(&mut self, cost: u128) {
        self.maintenance_cost = Some(
            self.maintenance_cost
                .unwrap_or_default()
                .saturating_add(cost),
        );
    }

    /// Applies canonical Runtime cache-state evidence for one provider
    /// attempt.  Misses and expiries suspend synthetic work immediately.
    pub fn apply_cache_state(
        &mut self,
        state: CacheState,
        expected_read_tokens: Option<u64>,
        observed_read_tokens: Option<u64>,
        observed_write_tokens: Option<u64>,
        missed_tokens: Option<u64>,
        at: Timestamp,
    ) {
        self.expected_read_tokens = expected_read_tokens;
        self.observed_read_tokens = observed_read_tokens;
        self.observed_write_tokens = observed_write_tokens;
        self.missed_tokens = missed_tokens;
        self.record_cache_touch(at);
        if observed_write_tokens.is_some_and(|tokens| tokens > 0) {
            self.last_write_at = max_timestamp(self.last_write_at, at);
        }
        let observed = CacheLeaseStatus::from(state);
        self.observed_status = Some(observed);
        match observed {
            CacheLeaseStatus::WarmObserved => {
                self.status = CacheLeaseStatus::WarmObserved;
                self.last_hit_at = max_timestamp(self.last_hit_at, at);
            }
            CacheLeaseStatus::MissObserved => {
                self.last_miss_at = max_timestamp(self.last_miss_at, at);
                self.suspend(LeaseSuspensionReason::CacheMiss, Some(observed), at);
            }
            CacheLeaseStatus::ExpiredObserved => {
                self.suspend(LeaseSuspensionReason::CacheExpired, Some(observed), at);
            }
            CacheLeaseStatus::Suspended => {
                self.status = CacheLeaseStatus::Suspended;
            }
            CacheLeaseStatus::Unsupported => self.status = CacheLeaseStatus::Unsupported,
            CacheLeaseStatus::Unknown => self.status = CacheLeaseStatus::Unknown,
            CacheLeaseStatus::Eligible => self.status = CacheLeaseStatus::Eligible,
        }
    }

    /// Applies one canonical presence-aware provider evidence record.  A
    /// zero/omitted token field is never turned into a miss here; Runtime's
    /// `CacheStateChanged` event is the authority for comparable misses.
    pub fn apply_evidence(&mut self, evidence: &CacheAvailabilityEvidence, at: Timestamp) {
        if let Some(boundary) = evidence.guaranteed_until {
            self.guaranteed_until = Some(
                self.guaranteed_until
                    .map_or(boundary, |existing| existing.max(boundary)),
            );
        }
        if evidence.request.is_some() || evidence.operation.is_some() {
            self.record_cache_touch(at);
        }
        if evidence.write_tokens.is_some_and(|tokens| tokens > 0)
            || evidence.kind == CacheEvidenceKind::Written
        {
            self.last_write_at = max_timestamp(self.last_write_at, at);
        }
        match evidence.kind {
            CacheEvidenceKind::Miss => {
                self.last_miss_at = max_timestamp(self.last_miss_at, at);
                self.suspend(
                    LeaseSuspensionReason::CacheMiss,
                    Some(CacheLeaseStatus::MissObserved),
                    at,
                );
            }
            CacheEvidenceKind::Expired | CacheEvidenceKind::Absent => {
                self.suspend(
                    LeaseSuspensionReason::CacheExpired,
                    Some(CacheLeaseStatus::ExpiredObserved),
                    at,
                );
            }
            CacheEvidenceKind::Hit => {
                self.status = CacheLeaseStatus::WarmObserved;
                self.observed_status = Some(CacheLeaseStatus::WarmObserved);
                self.last_hit_at = max_timestamp(self.last_hit_at, at);
            }
            CacheEvidenceKind::Written => {
                self.status = CacheLeaseStatus::WarmObserved;
                self.observed_status = Some(CacheLeaseStatus::WarmObserved);
            }
            CacheEvidenceKind::Observation => {
                if evidence.read_tokens.is_some_and(|tokens| tokens > 0) {
                    self.status = CacheLeaseStatus::WarmObserved;
                    self.observed_status = Some(CacheLeaseStatus::WarmObserved);
                    self.last_hit_at = max_timestamp(self.last_hit_at, at);
                }
            }
        }
    }

    /// Suspends further synthetic work while retaining the last observed
    /// provider state.
    pub fn suspend(
        &mut self,
        reason: LeaseSuspensionReason,
        observed_status: Option<CacheLeaseStatus>,
        at: Timestamp,
    ) {
        if let Some(observed_status) = observed_status {
            self.observed_status = Some(observed_status);
        }
        if reason == LeaseSuspensionReason::CacheMiss {
            self.last_miss_at = max_timestamp(self.last_miss_at, at);
        }
        self.status = CacheLeaseStatus::Suspended;
        self.suspension_reason = Some(reason);
    }

    /// Retires an identity because a new exact plan is current.  Its evidence
    /// remains historical and never transfers to another lease.
    pub fn retire_for_identity_change(&mut self, at: Timestamp) {
        self.retired = true;
        self.suspend(LeaseSuspensionReason::IdentityChanged, None, at);
    }

    /// Retires an identity after successful or failed idle compaction.
    pub fn retire_after_compaction(&mut self, at: Timestamp) {
        self.retired = true;
        self.suspend(LeaseSuspensionReason::Compacted, None, at);
    }

    /// Resets provider warmth to unknown after process resume.  No elapsed
    /// time or saved warm state is treated as current evidence.
    pub fn cold_resume(&mut self, at: Timestamp) {
        self.status = if self.status == CacheLeaseStatus::Unsupported {
            CacheLeaseStatus::Unsupported
        } else {
            CacheLeaseStatus::Unknown
        };
        self.observed_status = None;
        self.guaranteed_until = None;
        self.suspension_reason = Some(LeaseSuspensionReason::ColdResume);
        self.cold_resume = true;
        self.real_request_observed = false;
        self.last_cache_touch_at = None;
        self.last_hit_at = None;
        self.last_write_at = None;
        self.last_miss_at = None;
        self.maintenance_calls = 0;
        self.maintenance_input_tokens = 0;
        self.maintenance_output_tokens = 0;
        self.maintenance_cost = None;
        let _ = at;
    }
}

fn max_timestamp(current: Option<Timestamp>, candidate: Timestamp) -> Option<Timestamp> {
    Some(current.map_or(candidate, |existing| existing.max(candidate)))
}

/// A bounded local policy reason for suppressing a scheduler action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceSuppressionReason {
    /// Maintenance is disabled.
    Disabled,
    /// Policy records/evaluates but never dispatches.
    ObserveOnly,
    /// No child, goal, or other real continuation source exists.
    NoContinuationSource,
    /// The parent is not parked at a safe boundary.
    ParentNotParked,
    /// Process/session/lifecycle state is no longer active.
    LifecycleInactive,
    /// Shutdown has begun.
    Shutdown,
    /// The lease identity is not the exact current Runtime identity.
    IdentityChanged,
    /// The identity has been retired.
    Retired,
    /// Provider cache behavior does not support the action.
    ProviderUnsupported,
    /// Provider evidence capabilities are insufficient.
    ProviderEvidenceUnavailable,
    /// Adapter synthetic conformance is absent or incomplete.
    MissingConformance,
    /// No permitted synthetic purpose is available.
    ActionUnsupported,
    /// Host did not grant synthetic provider spend.
    MissingHostAuthority,
    /// The current identity is suspended by explicit evidence or policy.
    Suspended,
    /// A provider guarantee covers the known continuation window.
    GuaranteedRetention,
    /// Parent activity made the scheduled operation unnecessary.
    RecentActivity,
    /// Meaningful inactivity or bounded child hold elapsed.
    InactivityLimit,
    /// The bounded hold while a child runs elapsed.
    ChildHoldLimit,
    /// The current parked interval used its call allowance.
    CallBudgetExhausted,
    /// The exact resolved plan/model input budget cannot fit the operation.
    InputBudgetExceeded,
    /// The configured/provider output budget cannot fit the operation.
    OutputBudgetExceeded,
    /// The configured/provider deadline budget cannot fit the operation.
    DeadlineBudgetExceeded,
    /// Provider attempt allowance is exhausted.
    ProviderAttemptLimit,
    /// Session attempt allowance is exhausted.
    SessionAttemptLimit,
    /// A scheduled operation is not due yet.
    NotDue,
    /// A cold resume forbids a cache-only prewarm.
    ColdResumeNoPrewarm,
    /// A cache operation is already in flight.
    InFlight,
    /// The bounded Runtime event stream skipped a canonical sequence, so the
    /// consumer projection can no longer authorize synthetic work safely.
    EventStreamGap,
}

/// The two bounded synthetic actions Smith may choose.  Their actual request
/// construction and canonical events remain Agent Runtime-owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheMaintenanceAction {
    /// A useful same-model continuation handoff summary.
    HandoffCheckpoint,
    /// A minimal ephemeral prefix keepalive.
    Keepalive,
}

impl CacheMaintenanceAction {
    /// Canonical Runtime purpose for this action.
    pub const fn purpose(self) -> ProviderAttemptPurpose {
        match self {
            Self::HandoffCheckpoint => ProviderAttemptPurpose::CacheHandoffCheckpoint,
            Self::Keepalive => ProviderAttemptPurpose::CacheKeepalive,
        }
    }
}

/// Smith's requested cache-maintenance mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheMaintenanceMode {
    /// Do not even schedule synthetic maintenance.
    Off,
    /// Observe plans/evidence but never send synthetic requests.
    #[default]
    Observe,
    /// Allow bounded policy decisions when host authority and adapter gates
    /// also pass.
    Adaptive,
}

/// Bounded adaptive maintenance policy.  This is intentionally independent of
/// pricing: cost is a presentation field, never an authority or limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheMaintenancePolicy {
    /// Requested mode.
    pub maintenance: CacheMaintenanceMode,
    /// Meaningful parent inactivity limit.
    pub inactivity_limit_ms: u64,
    /// Maximum parent hold while a child is active; zero disables the hold.
    pub max_hold_while_child_ms: u64,
    /// Maximum accepted synthetic calls per parked interval.
    pub max_maintenance_calls: u32,
    /// Maximum exact plan input tokens; zero uses the exact model/plan limit.
    pub max_maintenance_input_tokens: u32,
    /// Maximum generated output tokens.
    pub max_maintenance_output_tokens: u32,
    /// Maximum synthetic deadline.
    pub maintenance_deadline_ms: u64,
    /// Early scheduling margin before a known guarantee boundary.
    pub keepalive_margin_ms: u64,
    /// Deterministic jitter percentage for a caller-provided seed.
    pub keepalive_jitter_percent: u8,
    /// Prefer a useful same-model handoff when supported.
    pub handoff_checkpoint: bool,
    /// Whether idle compaction may be attempted by its separate controller.
    pub idle_compaction: bool,
}

impl Default for CacheMaintenancePolicy {
    fn default() -> Self {
        Self {
            maintenance: CacheMaintenanceMode::Observe,
            inactivity_limit_ms: DEFAULT_INACTIVITY_LIMIT_MS,
            max_hold_while_child_ms: DEFAULT_MAX_HOLD_WHILE_CHILD_MS,
            max_maintenance_calls: DEFAULT_MAX_MAINTENANCE_CALLS,
            max_maintenance_input_tokens: 0,
            max_maintenance_output_tokens: DEFAULT_MAX_MAINTENANCE_OUTPUT_TOKENS,
            maintenance_deadline_ms: DEFAULT_MAINTENANCE_DEADLINE_MS,
            keepalive_margin_ms: 120_000,
            keepalive_jitter_percent: 10,
            handoff_checkpoint: true,
            idle_compaction: true,
        }
    }
}

impl CacheMaintenancePolicy {
    /// Validates the bounded ranges in the approved Smith policy.
    pub fn validate(self) -> Result<(), &'static str> {
        if !(1_000..=86_400_000).contains(&self.inactivity_limit_ms) {
            return Err("inactivity_limit_ms must be between 1_000 and 86_400_000");
        }
        if self.max_hold_while_child_ms > 86_400_000 {
            return Err("max_hold_while_child_ms must be at most 86_400_000");
        }
        if self.max_maintenance_calls > 8 {
            return Err("max_maintenance_calls must be at most 8");
        }
        if self.max_maintenance_output_tokens == 0 || self.max_maintenance_output_tokens > 4_096 {
            return Err("max_maintenance_output_tokens must be between 1 and 4_096");
        }
        if self.maintenance_deadline_ms == 0 || self.maintenance_deadline_ms > 120_000 {
            return Err("maintenance_deadline_ms must be between 1 and 120_000");
        }
        if self.keepalive_margin_ms > self.inactivity_limit_ms {
            return Err("keepalive_margin_ms must not exceed inactivity_limit_ms");
        }
        if self.keepalive_jitter_percent > 50 {
            return Err("keepalive_jitter_percent must be at most 50");
        }
        Ok(())
    }
}

/// Remaining ordinary/provider limits supplied by the host.  `None` means
/// that no narrower limit was declared at this policy boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SchedulerLimits {
    /// Remaining provider attempts.
    pub provider_attempts: Option<u32>,
    /// Remaining provider input tokens.
    pub provider_input_tokens: Option<u64>,
    /// Remaining provider output tokens.
    pub provider_output_tokens: Option<u64>,
    /// Remaining provider tokens when the provider reports a combined token
    /// window rather than disjoint input/output windows.
    pub provider_total_tokens: Option<u64>,
    /// Remaining session attempts.
    pub session_attempts: Option<u32>,
    /// Remaining session input tokens.
    pub session_input_tokens: Option<u64>,
    /// Remaining session output tokens.
    pub session_output_tokens: Option<u64>,
    /// Remaining session/goal tokens when policy supplies one combined
    /// charged-token budget.
    pub session_total_tokens: Option<u64>,
    /// Remaining deadline budget at the decision boundary.
    pub deadline_remaining_ms: Option<u64>,
}

/// Inputs to one pure scheduler evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheSchedulerInput {
    /// Current fake/system time.
    pub now: Timestamp,
    /// Exact Runtime identity selected by the immutable plan.
    pub identity: CacheIdentity,
    /// Exact input token count for the synthetic plan, including a bounded
    /// handoff suffix when applicable.
    pub planned_input_tokens: u32,
    /// Exact resolved model input limit.
    pub model_input_limit: u32,
    /// The scheduled due time, if a caller has set one.
    pub scheduled_for: Option<Timestamp>,
    /// Parent remains parked at a normal safe boundary.
    pub parent_parked: bool,
    /// Start of this parked interval.
    pub parked_since: Option<Timestamp>,
    /// A real continuation source (child, goal, or required durable work)
    /// exists.
    pub continuation_source: bool,
    /// A child is still active for the bounded hold gate.
    pub child_active: bool,
    /// Process/session/lifecycle lease state.
    pub process_active: bool,
    /// The Smith session remains active.
    pub session_active: bool,
    /// The Smith lifecycle lease remains held.
    pub lifecycle_lease_active: bool,
    /// Shutdown has begun.
    pub shutdown: bool,
    /// Explicit host authority for synthetic provider spend.
    pub host_synthetic_spend_allowed: bool,
    /// If known, the continuation's expected completion boundary.  This is
    /// used only to honor a provider guarantee; it never creates evidence.
    pub continuation_expected_by: Option<Timestamp>,
    /// Real parent activity observed after a scheduled boundary was created.
    pub real_parent_activity_at: Option<Timestamp>,
    /// Whether this decision follows cold resume and therefore cannot prewarm.
    pub cold_resume: bool,
    /// Whether this lease already owns a dispatched operation reservation.
    pub operation_in_flight: bool,
    /// Whether the proposed handoff is same-provider-and-model eligible.
    pub same_provider_and_model: bool,
    /// Runtime's validated model-scoped cache contract.
    pub contract: ProviderCacheContract,
    /// Ordinary provider/session remaining limits.
    pub limits: SchedulerLimits,
    /// Presentation-only estimate.  Deliberately ignored by evaluation.
    pub estimated_cost_micro_usd: Option<u128>,
}

impl CacheSchedulerInput {
    /// Starts a scheduler input with conservative lifecycle defaults.
    pub fn new(identity: CacheIdentity, now: Timestamp) -> Self {
        Self {
            now,
            identity,
            planned_input_tokens: 0,
            model_input_limit: u32::MAX,
            scheduled_for: None,
            parent_parked: true,
            parked_since: None,
            continuation_source: false,
            child_active: false,
            process_active: true,
            session_active: true,
            lifecycle_lease_active: true,
            shutdown: false,
            host_synthetic_spend_allowed: false,
            continuation_expected_by: None,
            real_parent_activity_at: None,
            cold_resume: false,
            operation_in_flight: false,
            same_provider_and_model: true,
            contract: ProviderCacheContract::default(),
            limits: SchedulerLimits::default(),
            estimated_cost_micro_usd: None,
        }
    }
}

/// The bounded result of scheduler evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheSchedulerDisposition {
    /// Dispatch one Runtime-owned cache operation.
    Dispatch,
    /// Record/evaluate only; no provider I/O is allowed.
    Observe,
    /// Suppress before provider dispatch.
    Suppressed,
}

/// A redaction-safe scheduler decision.  It is an intent projection, not a
/// provider request and carries no prompt body or credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheSchedulerDecision {
    /// Bounded disposition.
    pub disposition: CacheSchedulerDisposition,
    /// Selected action, when dispatch is permitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<CacheMaintenanceAction>,
    /// Canonical Runtime purpose for the selected action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<ProviderAttemptPurpose>,
    /// Local policy suppression/observation reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<MaintenanceSuppressionReason>,
    /// Exact input bound passed to Runtime request construction.
    pub planned_input_tokens: u32,
    /// Bounded output limit passed to Runtime request construction.
    pub max_output_tokens: u32,
    /// Bounded deadline requested from Runtime.
    pub deadline_ms: u64,
}

impl CacheSchedulerDecision {
    fn suppressed(reason: MaintenanceSuppressionReason, policy: CacheMaintenancePolicy) -> Self {
        Self {
            disposition: CacheSchedulerDisposition::Suppressed,
            action: None,
            purpose: None,
            reason: Some(reason),
            planned_input_tokens: 0,
            max_output_tokens: policy.max_maintenance_output_tokens,
            deadline_ms: policy.maintenance_deadline_ms,
        }
    }

    fn observed(reason: MaintenanceSuppressionReason, policy: CacheMaintenancePolicy) -> Self {
        Self {
            disposition: CacheSchedulerDisposition::Observe,
            action: None,
            purpose: None,
            reason: Some(reason),
            planned_input_tokens: 0,
            max_output_tokens: policy.max_maintenance_output_tokens,
            deadline_ms: policy.maintenance_deadline_ms,
        }
    }

    /// Whether provider dispatch is authorized by this decision.
    pub fn is_dispatch(&self) -> bool {
        self.disposition == CacheSchedulerDisposition::Dispatch
    }
}

/// Pure bounded scheduler.  Constructing it does not start a timer or spawn a
/// task; callers can evaluate it against a fake clock whenever a boundary is
/// reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheScheduler {
    /// Policy used by evaluations.
    pub policy: CacheMaintenancePolicy,
}

impl CacheScheduler {
    /// Creates a scheduler after validating policy bounds.
    pub fn new(policy: CacheMaintenancePolicy) -> Result<Self, &'static str> {
        policy.validate()?;
        Ok(Self { policy })
    }

    /// Evaluates all lifecycle, identity, authority, conformance, guarantee,
    /// and ordinary-limit gates in deterministic order.
    pub fn evaluate(
        &self,
        lease: &CacheLease,
        input: &CacheSchedulerInput,
    ) -> CacheSchedulerDecision {
        let policy = self.policy;
        if input.shutdown {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::Shutdown,
                policy,
            );
        }
        if !input.process_active || !input.session_active || !input.lifecycle_lease_active {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::LifecycleInactive,
                policy,
            );
        }
        if policy.maintenance == CacheMaintenanceMode::Off {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::Disabled,
                policy,
            );
        }
        if policy.maintenance == CacheMaintenanceMode::Observe {
            return CacheSchedulerDecision::observed(
                MaintenanceSuppressionReason::ObserveOnly,
                policy,
            );
        }
        if !input.continuation_source {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::NoContinuationSource,
                policy,
            );
        }
        if !input.parent_parked {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::ParentNotParked,
                policy,
            );
        }
        if input.shutdown {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::Shutdown,
                policy,
            );
        }
        if lease.identity != input.identity {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::IdentityChanged,
                policy,
            );
        }
        if lease.retired {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::Retired,
                policy,
            );
        }
        if input.operation_in_flight {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::InFlight,
                policy,
            );
        }
        if input.cold_resume || lease.cold_resume || !lease.real_request_observed {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::ColdResumeNoPrewarm,
                policy,
            );
        }
        if lease.status == CacheLeaseStatus::Unsupported {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::ProviderUnsupported,
                policy,
            );
        }
        if lease.suspension_reason.is_some()
            || matches!(
                lease.status,
                CacheLeaseStatus::Suspended
                    | CacheLeaseStatus::MissObserved
                    | CacheLeaseStatus::ExpiredObserved
            )
        {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::Suspended,
                policy,
            );
        }
        if let Some(due) = input.scheduled_for
            && input.now < due
        {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::NotDue,
                policy,
            );
        }
        if input
            .real_parent_activity_at
            .is_some_and(|activity| input.scheduled_for.is_some_and(|due| activity >= due))
        {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::RecentActivity,
                policy,
            );
        }
        let planned_total = u64::from(input.planned_input_tokens)
            .saturating_add(u64::from(policy.max_maintenance_output_tokens));
        if input
            .limits
            .provider_total_tokens
            .is_some_and(|remaining| remaining < planned_total)
            || input
                .limits
                .session_total_tokens
                .is_some_and(|remaining| remaining < planned_total)
        {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::InputBudgetExceeded,
                policy,
            );
        }
        if input.parked_since.is_some_and(|parked| {
            lease
                .last_meaningful_activity_at
                .is_some_and(|activity| activity > parked)
        }) {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::RecentActivity,
                policy,
            );
        }
        if lease
            .effective_guaranteed_until(input.now)
            .zip(input.continuation_expected_by)
            .is_some_and(|(guarantee, expected)| expected <= guarantee)
        {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::GuaranteedRetention,
                policy,
            );
        }
        if lease.maintenance_calls >= policy.max_maintenance_calls {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::CallBudgetExhausted,
                policy,
            );
        }
        if input.child_active
            && (policy.max_hold_while_child_ms == 0
                || input.parked_since.is_some_and(|parked| {
                    input.now >= parked.plus_millis(policy.max_hold_while_child_ms)
                }))
        {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::ChildHoldLimit,
                policy,
            );
        }
        if lease
            .last_meaningful_activity_at
            .is_some_and(|activity| input.now >= activity.plus_millis(policy.inactivity_limit_ms))
        {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::InactivityLimit,
                policy,
            );
        }
        if !input.host_synthetic_spend_allowed {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::MissingHostAuthority,
                policy,
            );
        }
        if input.contract.behavior == ProviderCacheBehavior::Unsupported {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::ProviderUnsupported,
                policy,
            );
        }
        let action = select_action(policy, input);
        let Some(action) = action else {
            return CacheSchedulerDecision::suppressed(
                if !input.contract.evidence.stream {
                    MaintenanceSuppressionReason::ProviderEvidenceUnavailable
                } else if input.contract.conformance.is_none()
                    || !input
                        .contract
                        .conformance
                        .is_some_and(|conformance| conformance.passes())
                {
                    MaintenanceSuppressionReason::MissingConformance
                } else {
                    MaintenanceSuppressionReason::ActionUnsupported
                },
                policy,
            );
        };
        let input_limit = if policy.max_maintenance_input_tokens == 0 {
            input.model_input_limit
        } else {
            policy
                .max_maintenance_input_tokens
                .min(input.model_input_limit)
        };
        if input.planned_input_tokens > input_limit
            || input
                .limits
                .provider_input_tokens
                .is_some_and(|remaining| remaining < u64::from(input.planned_input_tokens))
            || input
                .limits
                .session_input_tokens
                .is_some_and(|remaining| remaining < u64::from(input.planned_input_tokens))
        {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::InputBudgetExceeded,
                policy,
            );
        }
        if input
            .limits
            .provider_output_tokens
            .is_some_and(|remaining| remaining < u64::from(policy.max_maintenance_output_tokens))
            || input.limits.session_output_tokens.is_some_and(|remaining| {
                remaining < u64::from(policy.max_maintenance_output_tokens)
            })
        {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::OutputBudgetExceeded,
                policy,
            );
        }
        if input
            .limits
            .deadline_remaining_ms
            .is_some_and(|remaining| remaining < policy.maintenance_deadline_ms)
        {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::DeadlineBudgetExceeded,
                policy,
            );
        }
        if input
            .limits
            .provider_attempts
            .is_some_and(|remaining| remaining == 0)
        {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::ProviderAttemptLimit,
                policy,
            );
        }
        if input
            .limits
            .session_attempts
            .is_some_and(|remaining| remaining == 0)
        {
            return CacheSchedulerDecision::suppressed(
                MaintenanceSuppressionReason::SessionAttemptLimit,
                policy,
            );
        }
        CacheSchedulerDecision {
            disposition: CacheSchedulerDisposition::Dispatch,
            action: Some(action),
            purpose: Some(action.purpose()),
            reason: None,
            planned_input_tokens: input.planned_input_tokens,
            max_output_tokens: policy.max_maintenance_output_tokens,
            deadline_ms: policy.maintenance_deadline_ms,
        }
    }

    /// Computes a deterministic due time before a known provider boundary.
    /// The seed is supplied by the host so tests do not depend on randomness.
    pub fn jittered_due_time(
        boundary: Timestamp,
        margin_ms: u64,
        jitter_percent: u8,
        seed: u64,
    ) -> Timestamp {
        let base = boundary.0.saturating_sub(margin_ms);
        if jitter_percent == 0 || margin_ms == 0 {
            return Timestamp(base);
        }
        let span = margin_ms
            .saturating_mul(u64::from(jitter_percent))
            .saturating_div(100);
        if span == 0 {
            return Timestamp(base);
        }
        let width = span.saturating_mul(2).saturating_add(1);
        let offset = seed % width;
        if offset <= span {
            Timestamp(base.saturating_sub(span - offset))
        } else {
            Timestamp(base.saturating_add(offset - span).min(boundary.0))
        }
    }
}

fn select_action(
    policy: CacheMaintenancePolicy,
    input: &CacheSchedulerInput,
) -> Option<CacheMaintenanceAction> {
    if policy.handoff_checkpoint
        && input.same_provider_and_model
        && input
            .contract
            .supports_synthetic(ProviderAttemptPurpose::CacheHandoffCheckpoint)
    {
        return Some(CacheMaintenanceAction::HandoffCheckpoint);
    }
    input
        .contract
        .supports_synthetic(ProviderAttemptPurpose::CacheKeepalive)
        .then_some(CacheMaintenanceAction::Keepalive)
}

/// Why an idle-compaction attempt was not admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionSuppressionReason {
    /// Compaction is disabled.
    Disabled,
    /// Lifecycle has ended.
    LifecycleInactive,
    /// Shutdown has begun.
    Shutdown,
    /// Meaningful inactivity has not elapsed.
    NotDue,
    /// The provider/tool loop has not reached a safe boundary.
    UnsafeBoundary,
    /// This idle interval already consumed its one ordinary attempt.
    AlreadyAttempted,
}

/// Inputs for the once-per-idle-interval compaction gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleCompactionInput {
    /// Current fake/system time.
    pub now: Timestamp,
    /// Last meaningful parent activity.
    pub last_meaningful_activity_at: Option<Timestamp>,
    /// Current interval identity.
    pub interval_id: String,
    /// Parent is at a safe persistence/provider boundary.
    pub safe_boundary: bool,
    /// Process/session/lifecycle lease state.
    pub lifecycle_active: bool,
    /// Shutdown has begun.
    pub shutdown: bool,
    /// A child remains active; it must not be interrupted by compaction.
    pub child_active: bool,
}

/// Pure result of idle-compaction evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdleCompactionDisposition {
    /// One ordinary `cache_idle_compaction` attempt may be made.
    Attempt,
    /// No attempt is admitted.
    Suppressed,
}

/// Bounded idle-compaction decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdleCompactionDecision {
    /// Attempt or suppression.
    pub disposition: IdleCompactionDisposition,
    /// Bounded reason when suppressed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<CompactionSuppressionReason>,
    /// Canonical Runtime purpose when an attempt is admitted.
    pub purpose: ProviderAttemptPurpose,
}

/// Once-per-idle-interval tracker.  It records an attempt before provider
/// dispatch so a failure/cancellation cannot trigger an automatic retry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdleCompactionController {
    /// Interval whose attempt state is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_id: Option<String>,
    /// Whether the one ordinary attempt has been consumed.
    pub attempted: bool,
}

impl IdleCompactionController {
    /// Evaluates the idle boundary and consumes the attempt on admission.
    pub fn evaluate(
        &mut self,
        policy: CacheMaintenancePolicy,
        input: &IdleCompactionInput,
    ) -> IdleCompactionDecision {
        if self.interval_id.as_deref() != Some(input.interval_id.as_str()) {
            self.interval_id = Some(input.interval_id.clone());
            self.attempted = false;
        }
        let suppressed = |reason| IdleCompactionDecision {
            disposition: IdleCompactionDisposition::Suppressed,
            reason: Some(reason),
            purpose: ProviderAttemptPurpose::IdleCompaction,
        };
        if !policy.idle_compaction {
            return suppressed(CompactionSuppressionReason::Disabled);
        }
        if input.shutdown {
            return suppressed(CompactionSuppressionReason::Shutdown);
        }
        if !input.lifecycle_active {
            return suppressed(CompactionSuppressionReason::LifecycleInactive);
        }
        if self.attempted {
            return suppressed(CompactionSuppressionReason::AlreadyAttempted);
        }
        if input
            .last_meaningful_activity_at
            .is_none_or(|activity| input.now < activity.plus_millis(policy.inactivity_limit_ms))
        {
            return suppressed(CompactionSuppressionReason::NotDue);
        }
        if !input.safe_boundary {
            return suppressed(CompactionSuppressionReason::UnsafeBoundary);
        }
        // Child activity does not extend the deadline, but it also never
        // blocks or interrupts the child's own lifecycle.
        let _child_active = input.child_active;
        self.attempted = true;
        IdleCompactionDecision {
            disposition: IdleCompactionDisposition::Attempt,
            reason: None,
            purpose: ProviderAttemptPurpose::IdleCompaction,
        }
    }

    /// Clears the tracker for a newly committed idle interval.
    pub fn reset(&mut self, interval_id: impl Into<String>) {
        self.interval_id = Some(interval_id.into());
        self.attempted = false;
    }
}

/// The reducer's bounded effect, useful to status projections without adding
/// a second canonical event vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheLifecycleEffect {
    /// Event was already reduced or did not carry an exact identity.
    Ignored,
    /// Structural planning facts changed.
    StructuralChanged,
    /// A lease was created for a new exact identity.
    LeaseCreated,
    /// An existing exact lease changed.
    LeaseUpdated,
    /// A prior identity was retired and a new one became current.
    IdentityRetired,
    /// Synthetic work for an identity was suspended.
    Suspended,
    /// Actual synthetic usage was attributed.
    UsageAttributed,
    /// Lifecycle shutdown suspended all leases.
    Shutdown,
}

/// Live/replay-equivalent Smith cache lifecycle projection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheLifecycleReducer {
    /// All current and historical exact leases.
    pub leases: Vec<CacheLease>,
    /// Exact current identity, when a plan has been installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_identity: Option<CacheIdentity>,
    /// Structural planning facts.
    pub structural: StructuralCacheProjection,
    /// Lifecycle has begun shutdown.
    #[serde(default)]
    pub shutdown: bool,
    /// Highest canonical event sequence already reduced. Persisting this
    /// watermark keeps replay idempotent across process restarts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_sequence: Option<u64>,
}

impl CacheLifecycleReducer {
    /// Restores an exact persisted identity only as a cold comparison
    /// baseline. Provider warmth, guarantees, and synthetic authority never
    /// survive a process boundary.
    pub fn restore_cold_identity(
        &mut self,
        identity: CacheIdentity,
        event_sequence: u64,
        at: Timestamp,
    ) {
        self.current_identity = Some(identity.clone());
        let mut lease = CacheLease::from_plan(identity, true, true);
        lease.cold_resume(at);
        self.push_lease(lease);
        self.last_event_sequence = Some(event_sequence);
    }

    /// Installs one exact Runtime plan and retires any incompatible current
    /// lease without transferring evidence or maintenance budget.
    pub fn install_plan(
        &mut self,
        identity: CacheIdentity,
        provider_supported: bool,
        has_comparable_predecessor: bool,
        preserved_prefix_tokens: u32,
        at: Timestamp,
    ) -> CacheLifecycleEffect {
        self.structural.provider_cache_supported = provider_supported;
        self.structural.has_comparable_predecessor = has_comparable_predecessor;
        self.structural.preserved_prefix_tokens = preserved_prefix_tokens;
        let changed = self
            .current_identity
            .as_ref()
            .is_some_and(|current| current != &identity);
        if changed
            && let Some(current) = self.current_identity.clone()
            && let Some(previous) = self.find_active_mut(&current)
        {
            previous.retire_for_identity_change(at);
        }
        self.current_identity = Some(identity.clone());
        let status = if !provider_supported {
            CacheLeaseStatus::Unsupported
        } else if has_comparable_predecessor {
            CacheLeaseStatus::Unknown
        } else {
            CacheLeaseStatus::Eligible
        };
        let existed = self.find_active_index(&identity).is_some();
        let index = match self.find_active_index(&identity) {
            Some(index) => index,
            None => self.push_lease(CacheLease::from_plan(
                identity.clone(),
                provider_supported,
                has_comparable_predecessor,
            )),
        };
        let should_reset_status = changed
            || !provider_supported
            || self.leases[index].status == CacheLeaseStatus::Unsupported;
        if should_reset_status {
            self.leases[index].status = status;
        }
        self.leases[index].structurally_preserved_prefix_tokens = preserved_prefix_tokens;
        if changed {
            CacheLifecycleEffect::IdentityRetired
        } else if existed {
            CacheLifecycleEffect::LeaseUpdated
        } else {
            CacheLifecycleEffect::LeaseCreated
        }
    }

    /// Returns the current exact lease.
    pub fn current(&self) -> Option<&CacheLease> {
        self.current_identity
            .as_ref()
            .and_then(|identity| self.find_active(identity))
    }

    /// Returns the current lease mutably for host-side lifecycle boundaries.
    pub fn current_mut(&mut self) -> Option<&mut CacheLease> {
        let identity = self.current_identity.clone()?;
        self.find_active_mut(&identity)
    }

    /// Returns an exact lease by identity, including historical state.
    pub fn lease(&self, identity: &CacheIdentity) -> Option<&CacheLease> {
        self.leases
            .iter()
            .rev()
            .find(|lease| &lease.identity == identity)
    }

    /// Starts a parked interval on the current lease.
    pub fn begin_parked_interval(&mut self, interval_id: impl Into<String>) {
        if let Some(identity) = self.current_identity.clone()
            && let Some(lease) = self.find_active_mut(&identity)
        {
            lease.begin_parked_interval(interval_id);
        }
    }

    /// Records meaningful real parent activity without creating cache
    /// evidence.
    pub fn record_parent_activity(&mut self, at: Timestamp) {
        if let Some(identity) = self.current_identity.clone()
            && let Some(lease) = self.find_active_mut(&identity)
        {
            lease.record_meaningful_activity(at);
        }
    }

    /// Records a real matching parent request, touching both clocks.
    pub fn record_parent_request(&mut self, identity: &CacheIdentity, at: Timestamp) {
        if let Some(lease) = self.find_active_mut(identity) {
            lease.record_real_parent_request(at);
        }
    }

    /// Marks cold resume for the current identity.  The old exact identity is
    /// retained only as a comparison baseline and no prewarm is authorized.
    pub fn cold_resume(&mut self, at: Timestamp) {
        if let Some(identity) = self.current_identity.clone()
            && let Some(lease) = self.find_active_mut(&identity)
        {
            lease.cold_resume(at);
        }
    }

    /// Retires the current identity after compaction; the next real request
    /// must establish any new provider cache naturally.
    pub fn retire_after_compaction(&mut self, at: Timestamp) {
        if let Some(identity) = self.current_identity.clone()
            && let Some(lease) = self.find_active_mut(&identity)
        {
            lease.retire_after_compaction(at);
        }
    }

    /// Freezes the projection at the host shutdown boundary even when the
    /// controller stops consuming events before Runtime emits its terminal
    /// `SessionShutdown` event.
    pub fn begin_shutdown(&mut self) {
        self.shutdown = true;
        for lease in &mut self.leases {
            lease.suspend(LeaseSuspensionReason::Shutdown, None, Timestamp::ZERO);
        }
    }

    /// Reduces one canonical Runtime event.  The same method is used for live
    /// streams and journal replay.
    pub fn apply(&mut self, envelope: &EventEnvelope) -> CacheLifecycleEffect {
        if self
            .last_event_sequence
            .is_some_and(|sequence| envelope.seq <= sequence)
        {
            return CacheLifecycleEffect::Ignored;
        }
        self.last_event_sequence = Some(envelope.seq);
        let at = envelope.timestamp;
        match &envelope.payload {
            RuntimeEvent::CachePlanChanged {
                cache_plan,
                preserved_prefix_tokens,
                invalidated_prefix_tokens,
                provider_cache_supported,
            } => {
                self.structural.cache_plan = Some(cache_plan.to_string());
                self.structural.preserved_prefix_tokens = *preserved_prefix_tokens;
                self.structural.invalidated_prefix_tokens = *invalidated_prefix_tokens;
                self.structural.provider_cache_supported = *provider_cache_supported;
                CacheLifecycleEffect::StructuralChanged
            }
            RuntimeEvent::CacheObservation {
                request,
                attempt,
                cache_identity: Some(identity),
                read_tokens,
                write_tokens,
                ..
            } => {
                let Some(index) = self.ensure_event_identity(identity, at, true) else {
                    return CacheLifecycleEffect::Ignored;
                };
                let evidence = CacheAvailabilityEvidence {
                    source: CacheEvidenceSource::Stream,
                    kind: CacheEvidenceKind::Observation,
                    identity: identity.clone(),
                    request: request.clone(),
                    attempt: attempt.clone(),
                    operation: None,
                    ordering: 0,
                    read_tokens: *read_tokens,
                    write_tokens: *write_tokens,
                    refresh_cause: None,
                    guaranteed_until: None,
                    refreshed: None,
                    resource: None,
                    exists: None,
                };
                self.leases[index].apply_evidence(&evidence, at);
                CacheLifecycleEffect::LeaseUpdated
            }
            RuntimeEvent::CacheObservation { .. } => CacheLifecycleEffect::Ignored,
            RuntimeEvent::CacheStateChanged {
                cache_identity: Some(identity),
                state,
                expected_read_tokens,
                observed_read_tokens,
                observed_write_tokens,
                missed_tokens,
                ..
            } => {
                let Some(index) =
                    self.ensure_event_identity(identity, at, expected_read_tokens.is_some())
                else {
                    return CacheLifecycleEffect::Ignored;
                };
                self.leases[index].apply_cache_state(
                    *state,
                    *expected_read_tokens,
                    *observed_read_tokens,
                    *observed_write_tokens,
                    *missed_tokens,
                    at,
                );
                if matches!(
                    state,
                    CacheState::MissObserved | CacheState::Expired | CacheState::Suspended
                ) {
                    CacheLifecycleEffect::Suspended
                } else {
                    CacheLifecycleEffect::LeaseUpdated
                }
            }
            RuntimeEvent::CacheStateChanged { .. } => CacheLifecycleEffect::Ignored,
            RuntimeEvent::CacheAvailabilityEvidenceRecorded { evidence } => {
                let Some(index) = self.ensure_event_identity(&evidence.identity, at, false) else {
                    return CacheLifecycleEffect::Ignored;
                };
                self.leases[index].apply_evidence(evidence, at);
                if evidence.suspends_maintenance() {
                    CacheLifecycleEffect::Suspended
                } else {
                    CacheLifecycleEffect::LeaseUpdated
                }
            }
            RuntimeEvent::CacheOperationPrepared {
                operation,
                identity,
                purpose,
                ..
            } => {
                let Some(index) = self.ensure_event_identity(identity, at, true) else {
                    return CacheLifecycleEffect::Ignored;
                };
                let lease = &mut self.leases[index];
                lease.last_operation_stage = Some(CacheOperationStage::Prepared);
                lease.last_operation_id = Some(operation.to_string());
                lease.last_operation_purpose = Some(*purpose);
                lease.last_operation_outcome = None;
                lease.last_operation_reason = None;
                lease.last_operation_metrics.clear();
                CacheLifecycleEffect::LeaseUpdated
            }
            RuntimeEvent::CacheOperationRejected {
                operation,
                identity,
                purpose,
                reason,
                ..
            } => {
                let Some(index) = self.ensure_event_identity(identity, at, true) else {
                    return CacheLifecycleEffect::Ignored;
                };
                let lease = &mut self.leases[index];
                lease.last_operation_stage = Some(CacheOperationStage::Rejected);
                lease.last_operation_id = Some(operation.to_string());
                lease.last_operation_purpose = Some(*purpose);
                lease.last_operation_outcome = Some(CacheOperationOutcome::Rejected);
                lease.last_operation_reason = Some(*reason);
                lease.last_operation_metrics.clear();
                CacheLifecycleEffect::LeaseUpdated
            }
            RuntimeEvent::CacheOperationStarted {
                operation,
                identity,
                purpose,
                ..
            } => {
                let Some(index) = self.ensure_event_identity(identity, at, true) else {
                    return CacheLifecycleEffect::Ignored;
                };
                let lease = &mut self.leases[index];
                lease.last_operation_stage = Some(CacheOperationStage::Started);
                lease.last_operation_id = Some(operation.to_string());
                lease.last_operation_purpose = Some(*purpose);
                lease.last_operation_outcome = None;
                lease.last_operation_reason = None;
                lease.last_operation_metrics.clear();
                lease.record_cache_touch(at);
                if is_parked_maintenance(*purpose) {
                    lease.record_maintenance_call(at);
                }
                CacheLifecycleEffect::LeaseUpdated
            }
            RuntimeEvent::CacheOperationSuspended {
                identity,
                operation,
                reason,
                ..
            } => {
                let Some(index) = self.ensure_event_identity(identity, at, true) else {
                    return CacheLifecycleEffect::Ignored;
                };
                let lease = &mut self.leases[index];
                lease.last_operation_stage = Some(CacheOperationStage::Suspended);
                if let Some(operation) = operation {
                    lease.last_operation_id = Some(operation.to_string());
                }
                lease.last_operation_reason = Some(*reason);
                lease.suspend((*reason).into(), None, at);
                CacheLifecycleEffect::Suspended
            }
            RuntimeEvent::CacheOperationCompleted {
                operation,
                identity,
                purpose,
                outcome,
                reason,
                metrics,
                ..
            } => {
                let Some(index) = self.ensure_event_identity(identity, at, true) else {
                    return CacheLifecycleEffect::Ignored;
                };
                let lease = &mut self.leases[index];
                lease.last_operation_stage = Some(CacheOperationStage::Completed);
                lease.last_operation_id = Some(operation.to_string());
                lease.last_operation_purpose = Some(*purpose);
                lease.last_operation_outcome = Some(*outcome);
                lease.last_operation_reason = *reason;
                lease.last_operation_metrics = metrics.clone();
                if *outcome == CacheOperationOutcome::Suspended {
                    lease.suspend(
                        reason
                            .map(LeaseSuspensionReason::from)
                            .unwrap_or(LeaseSuspensionReason::PolicyBoundary),
                        None,
                        at,
                    );
                    CacheLifecycleEffect::Suspended
                } else {
                    CacheLifecycleEffect::LeaseUpdated
                }
            }
            RuntimeEvent::Usage { record } => self.apply_usage(record),
            RuntimeEvent::ContextCompacted { .. } => {
                self.retire_after_compaction(at);
                CacheLifecycleEffect::IdentityRetired
            }
            RuntimeEvent::SessionShutdown => {
                self.begin_shutdown();
                CacheLifecycleEffect::Shutdown
            }
            _ => CacheLifecycleEffect::Ignored,
        }
    }

    /// Replays canonical events idempotently.
    pub fn replay<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = EventEnvelope>,
    {
        for event in events {
            self.apply(&event);
        }
    }

    fn apply_usage(&mut self, record: &UsageRecord) -> CacheLifecycleEffect {
        let Some(identity) = record.provenance.cache_identity.as_ref() else {
            return CacheLifecycleEffect::Ignored;
        };
        let Some(purpose) = record.provenance.attempt_purpose else {
            return CacheLifecycleEffect::Ignored;
        };
        if !is_stream_maintenance(purpose) {
            return CacheLifecycleEffect::Ignored;
        }
        let Some(lease) = self.find_active_mut(identity) else {
            return CacheLifecycleEffect::Ignored;
        };
        lease.record_maintenance_usage(
            record.delta.input_tokens(),
            record.delta.get(CounterKind::Output),
        );
        CacheLifecycleEffect::UsageAttributed
    }

    fn ensure_current(
        &mut self,
        identity: &CacheIdentity,
        at: Timestamp,
        has_comparable_predecessor: bool,
    ) -> usize {
        let changed = self
            .current_identity
            .as_ref()
            .is_some_and(|current| current != identity);
        if changed
            && let Some(current) = self.current_identity.clone()
            && let Some(previous) = self.find_active_mut(&current)
        {
            previous.retire_for_identity_change(at);
        }
        self.current_identity = Some(identity.clone());
        if let Some(index) = self.find_active_index(identity) {
            return index;
        }
        self.push_lease(CacheLease::from_plan(
            identity.clone(),
            true,
            has_comparable_predecessor,
        ))
    }

    /// Late events for a retired exact identity must not switch the current
    /// lease back to historical state.  A never-seen identity may become
    /// current when Runtime emits its first event before a separate plan
    /// projection arrives.
    fn ensure_event_identity(
        &mut self,
        identity: &CacheIdentity,
        at: Timestamp,
        has_comparable_predecessor: bool,
    ) -> Option<usize> {
        if self
            .current_identity
            .as_ref()
            .is_some_and(|current| current != identity)
            && self.leases.iter().any(|lease| &lease.identity == identity)
        {
            return None;
        }
        Some(self.ensure_current(identity, at, has_comparable_predecessor))
    }

    fn find_active(&self, identity: &CacheIdentity) -> Option<&CacheLease> {
        self.leases
            .iter()
            .rev()
            .find(|lease| !lease.retired && &lease.identity == identity)
    }

    fn find_active_mut(&mut self, identity: &CacheIdentity) -> Option<&mut CacheLease> {
        self.leases
            .iter_mut()
            .rev()
            .find(|lease| !lease.retired && &lease.identity == identity)
    }

    fn find_active_index(&self, identity: &CacheIdentity) -> Option<usize> {
        self.leases
            .iter()
            .enumerate()
            .rev()
            .find(|(_, lease)| !lease.retired && &lease.identity == identity)
            .map(|(index, _)| index)
    }

    fn push_lease(&mut self, lease: CacheLease) -> usize {
        if self.leases.len() >= MAX_CACHE_LEASES {
            let evict = self
                .leases
                .iter()
                .position(|candidate| candidate.retired)
                .unwrap_or(0);
            self.leases.remove(evict);
        }
        self.leases.push(lease);
        self.leases.len() - 1
    }
}

fn is_stream_maintenance(purpose: ProviderAttemptPurpose) -> bool {
    matches!(
        purpose,
        ProviderAttemptPurpose::CacheKeepalive
            | ProviderAttemptPurpose::CacheHandoffCheckpoint
            | ProviderAttemptPurpose::IdleCompaction
    )
}

fn is_parked_maintenance(purpose: ProviderAttemptPurpose) -> bool {
    matches!(
        purpose,
        ProviderAttemptPurpose::CacheKeepalive | ProviderAttemptPurpose::CacheHandoffCheckpoint
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::registry::{Fingerprint, RegistryRevision};
    use agent_runtime_core::cache::{
        CacheEndpointIdentity, CacheRefreshCause, SyntheticConformance,
    };
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
}
