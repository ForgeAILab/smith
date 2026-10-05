use super::*;

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
