use super::*;

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
