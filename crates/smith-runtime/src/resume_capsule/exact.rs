use super::*;

/// Redaction-safe lifecycle state for one direct child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildLifecycleState {
    /// Child was created but has not committed execution.
    Pending,
    /// Child was live before process exit.
    Running,
    /// Child committed success.
    Completed,
    /// Child committed an interaction request awaiting parent follow-up.
    NeedsInput,
    /// Child committed failure.
    Failed,
    /// Child was explicitly stopped.
    Stopped,
    /// Live/uncommitted child reconciled after process exit.
    InterruptedByProcessExit,
}

/// Bounded terminal child outcome metadata.  Result content is represented by
/// a digest, never copied into ordinary status or capsule JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildTerminalOutcome {
    /// Terminal state.
    pub state: ChildLifecycleState,
    /// Optional result digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_digest: Option<Fingerprint>,
    /// Protected outcome watermark.
    pub watermark: u64,
}

/// Redaction-safe exact child projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildResumeProjection {
    /// Stable child identity.
    pub child: ChildId,
    /// Digest of the task text; raw task text stays protected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_digest: Option<Fingerprint>,
    /// Current exact state.
    pub state: ChildLifecycleState,
    /// Committed terminal outcome, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_outcome: Option<ChildTerminalOutcome>,
    /// Exact child record watermark.
    pub watermark: u64,
}

impl ChildResumeProjection {
    /// Reconciles live/uncommitted state after process exit without restarting.
    pub fn reconcile_after_process_exit(&mut self) -> bool {
        if self.terminal_outcome.is_some() {
            return false;
        }
        if matches!(
            self.state,
            ChildLifecycleState::Pending | ChildLifecycleState::Running
        ) {
            self.state = ChildLifecycleState::InterruptedByProcessExit;
            return true;
        }
        false
    }
}

/// Bounded goal/plan state that remains authoritative over summary prose.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactGoalProjection {
    /// Stable goal id, if active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    /// Monotonic goal generation.
    pub generation: u64,
    /// Bounded lifecycle label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// Bounded plan state projection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactPlanProjection {
    /// Plan revision.
    pub revision: u64,
    /// Number of pending items.
    pub pending: u32,
    /// Number of completed items.
    pub completed: u32,
    /// Number of failed/cancelled items.
    pub failed: u32,
}

/// Exact validation evidence retained as structured metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationProjection {
    /// Bounded validation key, normally a digest of the command.
    pub validation: String,
    /// Observed process exit status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_status: Option<i32>,
    /// Validation record watermark.
    pub watermark: u64,
}

/// Changed-file metadata without file bodies or private prompt content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFileProjection {
    /// Workspace-relative path.
    pub path: String,
    /// Bounded diff metadata.
    pub additions: u32,
    /// Bounded diff metadata.
    pub deletions: u32,
    /// Optional content digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<Fingerprint>,
}

/// Durable artifact reference metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactProjection {
    /// Redaction-safe artifact id.
    pub artifact: String,
    /// Optional digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<Fingerprint>,
}

/// A recent canonical turn represented only by metadata/digests. Synthetic
/// maintenance turns cannot enter this collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecentTurnRole {
    /// User-authored canonical turn.
    User,
    /// Assistant-authored canonical turn.
    Assistant,
    /// Tool-result canonical turn.
    Tool,
    /// Runtime internal canonical turn.
    Internal,
}

/// Redaction-safe recent canonical-turn metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentTurnProjection {
    /// Stable turn identity.
    pub turn: TurnId,
    /// Canonical role.
    pub role: RecentTurnRole,
    /// Content digest, not content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<Fingerprint>,
}

/// Provider-cache comparison baseline restored by a capsule.  Warmth is
/// intentionally reset on cold resume.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheResumeProjection {
    /// Prior exact identity used only for comparison with the next plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_identity: Option<CacheIdentity>,
    /// Structurally preserved tokens from planning.
    #[serde(default)]
    pub structurally_preserved_prefix_tokens: u32,
    /// Provider warmth status before a cold reset, if known.
    #[serde(default)]
    pub provider_warmth: ResumeCacheWarmth,
    /// Provider guarantee is evidence and is cleared on cold resume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guaranteed_until: Option<Timestamp>,
    /// True once the cold-resume no-prewarm invariant is active.
    #[serde(default)]
    pub cold_resume: bool,
    /// Last real parent activity used to continue the idle deadline across a
    /// process restart. Synthetic work and child activity never update it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_meaningful_activity_at: Option<Timestamp>,
    /// Stable root-turn interval whose once-only idle compaction state is
    /// persisted before provider dispatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_compaction_interval_id: Option<String>,
    /// Whether the interval's ordinary compaction attempt was durably
    /// consumed. A failure, cancellation, or restart must not replay it.
    #[serde(default)]
    pub idle_compaction_attempted: bool,
}

/// Provider warmth projection used by resume logic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeCacheWarmth {
    /// No current provider evidence after process resume.
    #[default]
    Unknown,
    /// Runtime previously observed a warm provider read.
    WarmObserved,
    /// Runtime previously observed an explicit miss.
    MissObserved,
    /// Runtime previously observed typed expiry.
    ExpiredObserved,
}

/// Exact structured state selected by a canonical/protected watermark.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactResumeState {
    /// Highest commit watermark represented by this state.
    #[serde(default)]
    pub watermark: u64,
    /// Parent turn boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<TurnId>,
    /// Exact goal state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<ExactGoalProjection>,
    /// Exact plan state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<ExactPlanProjection>,
    /// Direct child state and committed terminal outcomes.
    #[serde(default)]
    pub children: BTreeMap<ChildId, ChildResumeProjection>,
    /// Validation exit evidence.
    #[serde(default)]
    pub validations: BTreeMap<String, ValidationProjection>,
    /// Changed-file metadata.
    #[serde(default)]
    pub changed_files: Vec<ChangedFileProjection>,
    /// Durable artifact references.
    #[serde(default)]
    pub artifacts: Vec<ArtifactProjection>,
    /// Count only; exact protected interaction content remains protected.
    #[serde(default)]
    pub unresolved_approvals: u32,
    /// Count only; exact decisions remain protected.
    #[serde(default)]
    pub unresolved_decisions: u32,
    /// Count only; exact constraints remain protected.
    #[serde(default)]
    pub unresolved_constraints: u32,
}

impl ExactResumeState {
    pub(super) fn validate_bounds(&self) -> Result<(), ResumeCapsuleError> {
        if self.children.len() > MAX_CHILDREN
            || self.validations.len() > MAX_VALIDATIONS
            || self.changed_files.len() > MAX_CHANGED_FILES
            || self.artifacts.len() > MAX_ARTIFACTS
        {
            return Err(ResumeCapsuleError::ProjectionLimit);
        }
        if self.children.iter().any(|(child, projection)| {
            !bounded_metadata(child.as_str())
                || projection.child != *child
                || projection.watermark > self.watermark
                || projection
                    .terminal_outcome
                    .as_ref()
                    .is_some_and(|outcome| outcome.watermark > projection.watermark)
        }) || self.validations.iter().any(|(key, projection)| {
            !bounded_metadata(key)
                || !bounded_metadata(&projection.validation)
                || projection.validation != *key
                || projection.watermark > self.watermark
        }) || self
            .changed_files
            .iter()
            .any(|file| !bounded_metadata(&file.path))
            || self
                .artifacts
                .iter()
                .any(|artifact| !bounded_metadata(&artifact.artifact))
        {
            return Err(ResumeCapsuleError::MetadataTooLarge);
        }
        if let Some(goal) = &self.goal
            && (goal
                .goal_id
                .as_deref()
                .is_some_and(|value| !bounded_metadata(value))
                || goal
                    .state
                    .as_deref()
                    .is_some_and(|value| !bounded_metadata(value)))
        {
            return Err(ResumeCapsuleError::MetadataTooLarge);
        }
        Ok(())
    }

    /// Reconciles all live children without touching committed terminal ones.
    pub fn reconcile_children_after_process_exit(&mut self) -> Vec<ChildId> {
        let mut interrupted = Vec::new();
        for (child, projection) in &mut self.children {
            if projection.reconcile_after_process_exit() {
                interrupted.push(child.clone());
            }
        }
        interrupted
    }
}
