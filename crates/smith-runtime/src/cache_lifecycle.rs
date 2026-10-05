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

mod compaction;
mod reducer;
mod scheduler;

pub use compaction::{
    CompactionSuppressionReason, IdleCompactionController, IdleCompactionDecision,
    IdleCompactionDisposition, IdleCompactionInput,
};
pub use reducer::{CacheLifecycleEffect, CacheLifecycleReducer};
pub use scheduler::{
    CacheMaintenanceAction, CacheMaintenanceMode, CacheMaintenancePolicy, CacheScheduler,
    CacheSchedulerDecision, CacheSchedulerDisposition, CacheSchedulerInput,
    MaintenanceSuppressionReason, SchedulerLimits,
};

#[cfg(test)]
mod tests;

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
