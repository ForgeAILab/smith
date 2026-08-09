//! Versioned, redaction-safe cold-continuation projection.
//!
//! A resume capsule is a projection over Smith's existing canonical snapshot
//! and protected checkpoint.  It is not a sidecar database and it is not a
//! second source of truth.  Exact structured state is selected by committed
//! watermarks; semantic summary text is optional, bounded, and never emitted
//! into the redaction-safe projection.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;

use agent_runtime::registry::{Fingerprint, RegistryRevision};
use agent_runtime_core::artifact::{
    ArtifactRead, ArtifactRef, ArtifactSensitivity, ArtifactStore, MAX_ARTIFACT_READ_BYTES,
};
use agent_runtime_core::cache::CacheIdentity;
use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::ids::{ChildId, SessionId, TurnId};
use agent_runtime_core::store::{SessionStateSensitivity, VersionedSessionState};
use serde::{Deserialize, Serialize};

/// Current resume-capsule schema revision.
pub const RESUME_CAPSULE_SCHEMA_VERSION: u32 = 1;
/// Existing session extension-state namespace used for capsule persistence.
pub const RESUME_CAPSULE_STATE_NAMESPACE: &str = "smith.resume-capsule";
/// Version of the extension-state envelope.
pub const RESUME_CAPSULE_STATE_REVISION: &str = "resume-capsule-1";
/// Maximum UTF-8 bytes retained in a live-only semantic summary.
pub const MAX_SUMMARY_BYTES: usize = 16 * 1024;
/// Maximum number of recent canonical turn metadata entries.
pub const MAX_RECENT_TURNS: usize = 32;
/// Maximum number of children projected into a capsule.
pub const MAX_CHILDREN: usize = 128;
/// Maximum number of bounded validation records.
pub const MAX_VALIDATIONS: usize = 128;
/// Maximum number of changed-file records.
pub const MAX_CHANGED_FILES: usize = 256;
/// Maximum number of summary coverage records.
pub const MAX_SUMMARY_COVERAGE: usize = 64;
/// Maximum number of durable artifact projections.
pub const MAX_ARTIFACTS: usize = 256;
/// Maximum number of redaction-safe recovery diagnostics.
pub const MAX_DIAGNOSTICS: usize = 64;
/// Maximum UTF-8 bytes for any free-form metadata label or workspace path.
pub const MAX_METADATA_BYTES: usize = 4 * 1024;
/// Maximum serialized redaction-safe capsule size. Protected summary text is
/// stored as an artifact and is never counted here.
pub const MAX_SERIALIZED_CAPSULE_BYTES: usize = 512 * 1024;
/// Stable protected artifact purpose for a resume summary.
pub const RESUME_SUMMARY_ARTIFACT_PURPOSE: &str = "cache.handoff.resume-summary";
/// Stable protected artifact purpose for an independently attributed idle
/// semantic summary. It must never be mistaken for a same-cache handoff.
pub const RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE: &str = "cache.idle-compaction.resume-summary";
/// Stable protected artifact media type for a resume summary.
pub const RESUME_SUMMARY_MEDIA_TYPE: &str = "application/vnd.smith.resume-summary+text";
/// Stable protected artifact purpose for the latest Runtime semantic-summary
/// extension state.  The capsule stores only this owner-authorized reference;
/// the JSON state remains sensitive and is never copied into redacted output.
pub const RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE: &str =
    "cache.semantic-summary.runtime-state";
/// Stable protected artifact media type for a Runtime semantic-summary state.
pub const RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE: &str =
    "application/vnd.smith.semantic-summary-state+json";

/// Errors raised when a capsule boundary would exceed a redaction-safe bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeCapsuleError {
    /// The supplied summary is larger than the protected live-only bound.
    SummaryTooLarge,
    /// A collection would exceed its bounded projection limit.
    ProjectionLimit,
    /// A handoff summary did not carry the exact parent cache identity.
    HandoffIdentityRequired,
    /// A serialized capsule has an unsupported future revision.
    UnsupportedSchemaVersion,
    /// Serialized input was not a JSON object.
    InvalidSerializedForm,
    /// A redaction-safe metadata field exceeded its allocation bound.
    MetadataTooLarge,
    /// A same-model handoff was not bound to the current exact cache identity.
    HandoffIdentityMismatch,
}

impl fmt::Display for ResumeCapsuleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SummaryTooLarge => "resume summary exceeds bounded size",
            Self::ProjectionLimit => "resume projection exceeds bounded size",
            Self::HandoffIdentityRequired => "handoff summary requires exact cache identity",
            Self::UnsupportedSchemaVersion => "resume capsule schema version is unsupported",
            Self::InvalidSerializedForm => "resume capsule serialized form must be an object",
            Self::MetadataTooLarge => "resume capsule metadata exceeds bounded size",
            Self::HandoffIdentityMismatch => {
                "handoff summary cache identity does not match the current parent identity"
            }
        })
    }
}

impl std::error::Error for ResumeCapsuleError {}

fn bounded_metadata(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_METADATA_BYTES
}

fn validate_coverage(coverage: &[SummaryCoverage]) -> Result<(), ResumeCapsuleError> {
    if coverage.len() > MAX_SUMMARY_COVERAGE
        || coverage.iter().any(|entry| {
            !bounded_metadata(&entry.source) || entry.from_watermark > entry.to_watermark
        })
    {
        return Err(ResumeCapsuleError::ProjectionLimit);
    }
    Ok(())
}

/// Bounded live-only text.  Its `Debug` representation and serialized parent
/// projection never expose its contents.
#[derive(Clone, PartialEq, Eq)]
pub struct ProtectedSummaryText(String);

impl ProtectedSummaryText {
    /// Validates and stores bounded text.
    pub fn new(text: impl Into<String>) -> Result<Self, ResumeCapsuleError> {
        let text = text.into();
        if text.len() > MAX_SUMMARY_BYTES {
            return Err(ResumeCapsuleError::SummaryTooLarge);
        }
        Ok(Self(text))
    }

    /// Reads the text for the live caller that owns protected persistence.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ProtectedSummaryText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProtectedSummaryText([redacted])")
    }
}

fn empty_protected_text() -> Option<ProtectedSummaryText> {
    None
}

/// Whether a semantic summary came from same-model handoff or ordinary
/// independent summarization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeSummaryPurpose {
    /// Same-provider/same-model cache-assisted handoff checkpoint.
    HandoffCheckpoint,
    /// Ordinary semantic summary/idle compaction route.
    OrdinarySummary,
}

const fn summary_artifact_purpose(purpose: ResumeSummaryPurpose) -> &'static str {
    match purpose {
        ResumeSummaryPurpose::HandoffCheckpoint => RESUME_SUMMARY_ARTIFACT_PURPOSE,
        ResumeSummaryPurpose::OrdinarySummary => RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
    }
}

/// Bounded outcome of an optional semantic summary operation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeSummaryOutcome {
    /// No summary body was committed.
    #[default]
    Missing,
    /// A bounded body was committed.
    Completed,
    /// Provider/persistence failed; exact state remains authoritative.
    Failed,
    /// The operation was cancelled or rejected.
    Cancelled,
}

/// Bounded source coverage for a semantic summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryCoverage {
    /// Redaction-safe source category.
    pub source: String,
    /// First logical watermark covered.
    pub from_watermark: u64,
    /// Last logical watermark covered.
    pub to_watermark: u64,
}

impl SummaryCoverage {
    /// Creates one bounded coverage record.
    pub fn new(source: impl Into<String>, from_watermark: u64, to_watermark: u64) -> Self {
        Self {
            source: source.into(),
            from_watermark,
            to_watermark,
        }
    }
}

/// Disjoint usage/cost provenance for a summary route.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryUsage {
    /// Uncached input tokens.
    pub input_uncached: u64,
    /// Cached input tokens.
    pub input_cached: u64,
    /// Cache-write input tokens.
    pub cache_write: u64,
    /// Output tokens.
    pub output: u64,
    /// Reasoning tokens.
    pub reasoning: u64,
    /// Provider-reported or calculated cost, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_micro_usd: Option<u128>,
    /// Cost is presentation/estimate provenance, never dispatch authority.
    #[serde(default)]
    pub cost_is_estimate: bool,
}

/// Redaction-safe provenance attached to a summary body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeSummaryProvenance {
    /// Summary route purpose.
    pub purpose: ResumeSummaryPurpose,
    /// Provider attribution.
    pub provider: String,
    /// Model attribution.
    pub model: String,
    /// Summary route revision.
    pub revision: RegistryRevision,
    /// Exact parent cache identity for a handoff route only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_identity: Option<CacheIdentity>,
    /// Bounded source coverage.
    #[serde(default)]
    pub source_coverage: Vec<SummaryCoverage>,
    /// Summary operation timestamp.
    pub generated_at: Timestamp,
    /// Bounded operation outcome.
    pub outcome: ResumeSummaryOutcome,
    /// Separately attributed usage and presentation cost.
    #[serde(default)]
    pub usage: SummaryUsage,
    /// Protected session artifact containing the bounded summary body. The
    /// reference is safe to persist; it never grants cross-session access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_artifact: Option<ArtifactRef>,
}

impl ResumeSummaryProvenance {
    /// Builds handoff provenance, requiring the exact Runtime identity.
    pub fn handoff(
        provider: impl Into<String>,
        model: impl Into<String>,
        revision: RegistryRevision,
        cache_identity: CacheIdentity,
        generated_at: Timestamp,
    ) -> Self {
        Self {
            purpose: ResumeSummaryPurpose::HandoffCheckpoint,
            provider: provider.into(),
            model: model.into(),
            revision,
            cache_identity: Some(cache_identity),
            source_coverage: Vec::new(),
            generated_at,
            outcome: ResumeSummaryOutcome::Missing,
            usage: SummaryUsage::default(),
            summary_artifact: None,
        }
    }

    /// Builds ordinary independent-summary provenance.  It cannot refresh a
    /// parent cache identity.
    pub fn ordinary(
        provider: impl Into<String>,
        model: impl Into<String>,
        revision: RegistryRevision,
        generated_at: Timestamp,
    ) -> Self {
        Self {
            purpose: ResumeSummaryPurpose::OrdinarySummary,
            provider: provider.into(),
            model: model.into(),
            revision,
            cache_identity: None,
            source_coverage: Vec::new(),
            generated_at,
            outcome: ResumeSummaryOutcome::Missing,
            usage: SummaryUsage::default(),
            summary_artifact: None,
        }
    }

    fn redacted(&self) -> RedactedSummaryProvenance {
        RedactedSummaryProvenance {
            purpose: self.purpose,
            provider: self.provider.clone(),
            model: self.model.clone(),
            revision: self.revision.clone(),
            cache_identity: self.cache_identity.clone(),
            source_coverage: self.source_coverage.clone(),
            generated_at: self.generated_at,
            outcome: self.outcome,
            usage: self.usage.clone(),
            summary_artifact: self.summary_artifact.clone(),
        }
    }
}

/// A live semantic summary retained in protected state.  The body is skipped
/// by serde; only its bounded provenance is visible in redaction-safe output.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSummary {
    /// Summary route provenance.
    pub provenance: ResumeSummaryProvenance,
    /// Structured, redaction-safe claims used only for bounded diagnostics.
    #[serde(default)]
    pub claims: Vec<SummaryClaim>,
    /// Optional protected summary body.
    #[serde(skip, default = "empty_protected_text")]
    body: Option<ProtectedSummaryText>,
}

impl fmt::Debug for SemanticSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SemanticSummary")
            .field("provenance", &self.provenance)
            .field("claims", &self.claims)
            .field("body", &"[redacted]")
            .finish()
    }
}

impl SemanticSummary {
    /// Creates a bounded summary with optional live-only body.
    pub fn new(
        provenance: ResumeSummaryProvenance,
        body: Option<impl Into<String>>,
    ) -> Result<Self, ResumeCapsuleError> {
        validate_coverage(&provenance.source_coverage)?;
        if !bounded_metadata(&provenance.provider) || !bounded_metadata(&provenance.model) {
            return Err(ResumeCapsuleError::MetadataTooLarge);
        }
        let body = body.map(ProtectedSummaryText::new).transpose()?;
        let mut provenance = provenance;
        provenance.outcome = if body.is_some() {
            ResumeSummaryOutcome::Completed
        } else {
            ResumeSummaryOutcome::Missing
        };
        Ok(Self {
            provenance,
            claims: Vec::new(),
            body,
        })
    }

    /// Reads the protected body for a live caller.
    pub fn body(&self) -> Option<&str> {
        self.body.as_ref().map(ProtectedSummaryText::as_str)
    }

    /// Adds a bounded structured claim without exposing prose.
    pub fn push_claim(&mut self, claim: SummaryClaim) -> Result<(), ResumeCapsuleError> {
        if self.claims.len() >= MAX_SUMMARY_COVERAGE {
            return Err(ResumeCapsuleError::ProjectionLimit);
        }
        match &claim {
            SummaryClaim::ValidationExit { validation, .. } if !bounded_metadata(validation) => {
                return Err(ResumeCapsuleError::MetadataTooLarge);
            }
            SummaryClaim::ChildState { child, .. } if !bounded_metadata(child.as_str()) => {
                return Err(ResumeCapsuleError::MetadataTooLarge);
            }
            _ => {}
        }
        self.claims.push(claim);
        Ok(())
    }

    /// Returns the redaction-safe summary projection.
    pub fn redacted_projection(&self) -> RedactedSummary {
        RedactedSummary {
            provenance: self.provenance.redacted(),
            claim_count: self.claims.len() as u32,
        }
    }
}

/// A structured claim that can be compared with exact state without parsing
/// or logging private summary prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SummaryClaim {
    /// Summary claims a validation exit status.
    ValidationExit {
        /// Bounded validation key.
        validation: String,
        /// Claimed process exit status.
        exit_status: i32,
    },
    /// Summary claims a child lifecycle state.
    ChildState {
        /// Stable child identity.
        child: ChildId,
        /// Claimed child state.
        state: ChildLifecycleState,
    },
    /// Summary claims a goal generation.
    GoalGeneration {
        /// Claimed monotonic goal generation.
        generation: u64,
    },
}

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
    fn validate_bounds(&self) -> Result<(), ResumeCapsuleError> {
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

/// Redaction-safe summary metadata.  It intentionally omits summary body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedSummary {
    /// Bounded route provenance.
    pub provenance: RedactedSummaryProvenance,
    /// Number of structured claims retained for diagnostics.
    pub claim_count: u32,
}

/// Redaction-safe summary provenance projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedSummaryProvenance {
    /// Summary purpose.
    pub purpose: ResumeSummaryPurpose,
    /// Provider label.
    pub provider: String,
    /// Model label.
    pub model: String,
    /// Summary route revision.
    pub revision: RegistryRevision,
    /// Opaque cache identity, if handoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_identity: Option<CacheIdentity>,
    /// Bounded source coverage.
    pub source_coverage: Vec<SummaryCoverage>,
    /// Generation timestamp.
    pub generated_at: Timestamp,
    /// Operation outcome.
    pub outcome: ResumeSummaryOutcome,
    /// Disjoint usage and cost provenance.
    pub usage: SummaryUsage,
    /// Protected session artifact containing the summary body, when one was
    /// durably committed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_artifact: Option<ArtifactRef>,
}

/// A bounded diagnostic retained when exact state disagrees with summary
/// claims.  It contains no conflicting prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeDiagnostic {
    /// Exact field/category that won.
    pub field: ResumeDiagnosticField,
    /// Authoritative source selected by watermark.
    pub authoritative_source: RecoverySource,
}

/// Field categories safe to expose in a conflict diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeDiagnosticField {
    /// Validation evidence category.
    Validation,
    /// Child evidence category.
    Child,
    /// Goal evidence category.
    Goal,
}

/// Source of the authoritative exact state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoverySource {
    /// Authenticated protected checkpoint.
    ProtectedCheckpoint,
    /// Canonical persisted snapshot.
    CanonicalSnapshot,
}

/// One exact source candidate with its commit watermark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactStateRecord {
    /// Session that owns this exact projection.
    session_id: SessionId,
    /// Commit watermark.
    watermark: u64,
    /// Structured exact state.
    state: ExactResumeState,
}

impl ExactStateRecord {
    /// Creates a source record and aligns its state watermark.
    pub fn new(session_id: SessionId, watermark: u64, mut state: ExactResumeState) -> Self {
        state.watermark = watermark;
        Self {
            session_id,
            watermark,
            state,
        }
    }

    fn validate_for(&self, session_id: &SessionId) -> Result<(), ResumeCapsuleError> {
        if &self.session_id != session_id || self.watermark != self.state.watermark {
            return Err(ResumeCapsuleError::InvalidSerializedForm);
        }
        self.state.validate_bounds()
    }
}

/// Result of selecting exact state for cold continuation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeRecoveryResult {
    /// Capsule with selected exact state.
    pub capsule: ResumeCapsule,
    /// Protected state wins on equal watermark.
    pub authoritative_source: RecoverySource,
    /// Bounded summary conflict diagnostics.
    pub diagnostics: Vec<ResumeDiagnostic>,
    /// Journal watermark is informational only.
    pub journal_watermark: Option<u64>,
}

/// Cold-resume result.  The next provider request is always the first natural
/// continuation; `prewarm_requested` is permanently false for this result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColdResumeResult {
    /// Reconciled capsule.
    pub capsule: ResumeCapsule,
    /// Provider warmth after reset.
    pub provider_warmth: ResumeCacheWarmth,
    /// Always false; retained to make the invariant inspectable in tests.
    pub prewarm_requested: bool,
    /// Children reconciled to interrupted state.
    pub interrupted_children: Vec<ChildId>,
}

/// The versioned Smith resume capsule projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeCapsule {
    /// Capsule schema revision.
    pub schema_version: u32,
    /// Root session id.
    pub session_id: SessionId,
    /// Parent boundary turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<TurnId>,
    /// Capsule creation/persistence timestamp.
    pub created_at: Timestamp,
    /// Resolved model profile identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_profile_identity: Option<Fingerprint>,
    /// Agent profile identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_profile_identity: Option<Fingerprint>,
    /// Project instruction revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_instruction_revision: Option<RegistryRevision>,
    /// Exact structured state selected at the latest Smith commit.
    pub exact_state: ExactResumeState,
    /// Optional semantic summary; body is protected/skipped by serde.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_summary: Option<SemanticSummary>,
    /// Protected reference to the latest Runtime semantic-summary extension
    /// state. This is the recovery source when ordinary session JSON omits
    /// Sensitive extension namespaces and no protected checkpoint exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_summary_state_artifact: Option<ArtifactRef>,
    /// Recent canonical turn metadata only.
    #[serde(default)]
    pub retained_recent_turns: Vec<RecentTurnProjection>,
    /// Provider-cache comparison baseline.
    pub cache: CacheResumeProjection,
    /// Last successful exact persistence watermark.
    #[serde(default)]
    pub last_persisted_watermark: u64,
    /// Last successful exact persistence boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_persisted_at: Option<Timestamp>,
    /// Bounded redaction-safe conflict diagnostics.
    #[serde(default)]
    pub diagnostics: Vec<ResumeDiagnostic>,
}

impl ResumeCapsule {
    /// Creates an empty versioned capsule.
    pub fn new(session_id: SessionId, created_at: Timestamp) -> Self {
        Self {
            schema_version: RESUME_CAPSULE_SCHEMA_VERSION,
            session_id,
            parent_turn_id: None,
            created_at,
            model_profile_identity: None,
            agent_profile_identity: None,
            project_instruction_revision: None,
            exact_state: ExactResumeState::default(),
            semantic_summary: None,
            latest_summary_state_artifact: None,
            retained_recent_turns: Vec::new(),
            cache: CacheResumeProjection::default(),
            last_persisted_watermark: 0,
            last_persisted_at: None,
            diagnostics: Vec::new(),
        }
    }

    /// Validates every persisted collection, metadata allocation, ownership,
    /// and watermark relationship before a capsule crosses a durability or
    /// recovery boundary.
    pub fn validate(&self) -> Result<(), ResumeCapsuleError> {
        if self.schema_version != RESUME_CAPSULE_SCHEMA_VERSION
            || !bounded_metadata(self.session_id.as_str())
            || self.retained_recent_turns.len() > MAX_RECENT_TURNS
            || self.diagnostics.len() > MAX_DIAGNOSTICS
            || self.last_persisted_watermark != self.exact_state.watermark
            || self.parent_turn_id != self.exact_state.parent_turn_id
        {
            return Err(ResumeCapsuleError::InvalidSerializedForm);
        }
        self.exact_state.validate_bounds()?;
        if self
            .cache
            .idle_compaction_interval_id
            .as_deref()
            .is_some_and(|interval| !bounded_metadata(interval))
        {
            return Err(ResumeCapsuleError::MetadataTooLarge);
        }
        if self
            .retained_recent_turns
            .iter()
            .any(|turn| !bounded_metadata(turn.turn.as_str()))
        {
            return Err(ResumeCapsuleError::MetadataTooLarge);
        }
        if let Some(summary) = &self.semantic_summary {
            validate_coverage(&summary.provenance.source_coverage)?;
            if summary.claims.len() > MAX_SUMMARY_COVERAGE
                || !bounded_metadata(&summary.provenance.provider)
                || !bounded_metadata(&summary.provenance.model)
                || summary.claims.iter().any(|claim| match claim {
                    SummaryClaim::ValidationExit { validation, .. } => {
                        !bounded_metadata(validation)
                    }
                    SummaryClaim::ChildState { child, .. } => !bounded_metadata(child.as_str()),
                    SummaryClaim::GoalGeneration { .. } => false,
                })
            {
                return Err(ResumeCapsuleError::MetadataTooLarge);
            }
            if summary.provenance.purpose == ResumeSummaryPurpose::HandoffCheckpoint {
                let identity = summary
                    .provenance
                    .cache_identity
                    .as_ref()
                    .ok_or(ResumeCapsuleError::HandoffIdentityRequired)?;
                if self.cache.prior_identity.as_ref() != Some(identity)
                    || summary.provenance.provider != identity.provider()
                    || summary.provenance.model != identity.model().as_str()
                {
                    return Err(ResumeCapsuleError::HandoffIdentityMismatch);
                }
            }
            if let Some(artifact) = &summary.provenance.summary_artifact {
                artifact
                    .validate()
                    .map_err(|_| ResumeCapsuleError::InvalidSerializedForm)?;
                if artifact.provenance.session != self.session_id
                    || artifact.provenance.purpose
                        != summary_artifact_purpose(summary.provenance.purpose)
                    || artifact.media_type != RESUME_SUMMARY_MEDIA_TYPE
                    || artifact.byte_length == 0
                    || artifact.byte_length > MAX_SUMMARY_BYTES as u64
                {
                    return Err(ResumeCapsuleError::InvalidSerializedForm);
                }
            } else if summary.provenance.outcome == ResumeSummaryOutcome::Completed {
                return Err(ResumeCapsuleError::InvalidSerializedForm);
            }
        }
        if let Some(reference) = &self.latest_summary_state_artifact {
            reference
                .validate()
                .map_err(|_| ResumeCapsuleError::InvalidSerializedForm)?;
            if reference.provenance.session != self.session_id
                || reference.provenance.purpose != RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE
                || reference.media_type != RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE
                || reference.sensitivity != ArtifactSensitivity::Sensitive
                || reference.byte_length == 0
                || reference.byte_length > MAX_SERIALIZED_CAPSULE_BYTES as u64
            {
                return Err(ResumeCapsuleError::InvalidSerializedForm);
            }
        }
        let encoded =
            serde_json::to_vec(self).map_err(|_| ResumeCapsuleError::InvalidSerializedForm)?;
        if encoded.len() > MAX_SERIALIZED_CAPSULE_BYTES {
            return Err(ResumeCapsuleError::ProjectionLimit);
        }
        Ok(())
    }

    /// Commits exact structured state at a meaningful canonical/protected
    /// boundary.  A stale watermark cannot overwrite newer state.
    pub fn commit_exact_state(&mut self, state: ExactResumeState, persisted_at: Timestamp) -> bool {
        if state.watermark < self.exact_state.watermark
            || (state.watermark == self.exact_state.watermark && state != self.exact_state)
        {
            return false;
        }
        self.replace_exact_state(state, persisted_at);
        true
    }

    fn replace_exact_state(&mut self, mut state: ExactResumeState, persisted_at: Timestamp) {
        self.parent_turn_id = state.parent_turn_id.clone();
        self.exact_state = {
            state.changed_files.truncate(MAX_CHANGED_FILES);
            state.artifacts.truncate(MAX_ARTIFACTS);
            state.children = state.children.into_iter().take(MAX_CHILDREN).collect();
            state.validations = state
                .validations
                .into_iter()
                .take(MAX_VALIDATIONS)
                .collect();
            state
        };
        self.last_persisted_watermark = self.exact_state.watermark;
        self.last_persisted_at = Some(persisted_at);
    }

    /// Adds canonical recent-turn metadata.  There is no text field, so raw
    /// prompt/response bodies cannot leak into the capsule.
    pub fn push_recent_turn(
        &mut self,
        turn: RecentTurnProjection,
    ) -> Result<(), ResumeCapsuleError> {
        if self.retained_recent_turns.len() >= MAX_RECENT_TURNS {
            self.retained_recent_turns.remove(0);
        }
        self.retained_recent_turns.push(turn);
        Ok(())
    }

    /// Stores a same-provider/model handoff summary using the exact parent
    /// identity.  Request/response content remains outside canonical history.
    #[allow(clippy::too_many_arguments)]
    pub fn record_handoff_summary(
        &mut self,
        provider: impl Into<String>,
        model: impl Into<String>,
        revision: RegistryRevision,
        identity: CacheIdentity,
        generated_at: Timestamp,
        body: impl Into<String>,
        source_coverage: Vec<SummaryCoverage>,
    ) -> Result<(), ResumeCapsuleError> {
        validate_coverage(&source_coverage)?;
        if self
            .cache
            .prior_identity
            .as_ref()
            .is_some_and(|current| current != &identity)
        {
            return Err(ResumeCapsuleError::HandoffIdentityMismatch);
        }
        let mut provenance =
            ResumeSummaryProvenance::handoff(provider, model, revision, identity, generated_at);
        provenance.source_coverage = source_coverage;
        self.semantic_summary = Some(SemanticSummary::new(provenance, Some(body))?);
        Ok(())
    }

    /// Records a bounded failed handoff without claiming that live text was
    /// durably committed. Exact state and any prior canonical history remain
    /// authoritative.
    #[allow(clippy::too_many_arguments)]
    pub fn record_failed_handoff_summary(
        &mut self,
        provider: impl Into<String>,
        model: impl Into<String>,
        revision: RegistryRevision,
        identity: CacheIdentity,
        generated_at: Timestamp,
        source_coverage: Vec<SummaryCoverage>,
    ) -> Result<(), ResumeCapsuleError> {
        validate_coverage(&source_coverage)?;
        if self
            .cache
            .prior_identity
            .as_ref()
            .is_some_and(|current| current != &identity)
        {
            return Err(ResumeCapsuleError::HandoffIdentityMismatch);
        }
        let mut provenance =
            ResumeSummaryProvenance::handoff(provider, model, revision, identity, generated_at);
        provenance.source_coverage = source_coverage;
        let mut summary = SemanticSummary::new(provenance, None::<String>)?;
        summary.provenance.outcome = ResumeSummaryOutcome::Failed;
        self.semantic_summary = Some(summary);
        Ok(())
    }

    /// Stores an independently attributed ordinary summary.  No parent cache
    /// projection is changed by this method.
    pub fn record_ordinary_summary(
        &mut self,
        provider: impl Into<String>,
        model: impl Into<String>,
        revision: RegistryRevision,
        generated_at: Timestamp,
        body: impl Into<String>,
        source_coverage: Vec<SummaryCoverage>,
    ) -> Result<(), ResumeCapsuleError> {
        validate_coverage(&source_coverage)?;
        let mut provenance =
            ResumeSummaryProvenance::ordinary(provider, model, revision, generated_at);
        provenance.source_coverage = source_coverage;
        self.semantic_summary = Some(SemanticSummary::new(provenance, Some(body))?);
        Ok(())
    }

    /// Records a failed independently attributed summary attempt without a
    /// body or artifact. Exact canonical state remains authoritative and the
    /// failure never authorizes retry or cache work.
    pub fn record_failed_ordinary_summary(
        &mut self,
        provider: impl Into<String>,
        model: impl Into<String>,
        revision: RegistryRevision,
        generated_at: Timestamp,
        source_coverage: Vec<SummaryCoverage>,
    ) -> Result<(), ResumeCapsuleError> {
        validate_coverage(&source_coverage)?;
        let mut provenance =
            ResumeSummaryProvenance::ordinary(provider, model, revision, generated_at);
        provenance.source_coverage = source_coverage;
        let mut summary = SemanticSummary::new(provenance, None::<String>)?;
        summary.provenance.outcome = ResumeSummaryOutcome::Failed;
        self.semantic_summary = Some(summary);
        Ok(())
    }

    /// Restores a summary object supplied by a live Runtime handoff.  A
    /// missing protected body is valid and never causes provider replay.
    pub fn set_summary(&mut self, summary: SemanticSummary) -> Result<(), ResumeCapsuleError> {
        validate_coverage(&summary.provenance.source_coverage)?;
        if summary.provenance.purpose == ResumeSummaryPurpose::HandoffCheckpoint
            && summary.provenance.cache_identity.is_none()
        {
            return Err(ResumeCapsuleError::HandoffIdentityRequired);
        }
        if summary.provenance.purpose == ResumeSummaryPurpose::HandoffCheckpoint
            && self
                .cache
                .prior_identity
                .as_ref()
                .is_some_and(|current| summary.provenance.cache_identity.as_ref() != Some(current))
        {
            return Err(ResumeCapsuleError::HandoffIdentityMismatch);
        }
        self.semantic_summary = Some(summary);
        Ok(())
    }

    /// Attaches protected artifact metadata to the current summary after the
    /// body has been durably written by the session-owned artifact store.
    pub fn attach_summary_artifact(
        &mut self,
        artifact: ArtifactRef,
    ) -> Result<(), ResumeCapsuleError> {
        artifact
            .validate()
            .map_err(|_| ResumeCapsuleError::InvalidSerializedForm)?;
        if artifact.provenance.session != self.session_id {
            return Err(ResumeCapsuleError::InvalidSerializedForm);
        }
        let summary = self
            .semantic_summary
            .as_mut()
            .ok_or(ResumeCapsuleError::InvalidSerializedForm)?;
        if artifact.provenance.purpose != summary_artifact_purpose(summary.provenance.purpose)
            || artifact.media_type != RESUME_SUMMARY_MEDIA_TYPE
            || artifact.byte_length == 0
            || artifact.byte_length > MAX_SUMMARY_BYTES as u64
        {
            return Err(ResumeCapsuleError::InvalidSerializedForm);
        }
        summary.provenance.summary_artifact = Some(artifact);
        Ok(())
    }

    /// Attaches the protected Runtime semantic-summary extension state used
    /// to reconstruct the latest summary when the ordinary session snapshot
    /// intentionally omits Sensitive namespaces.
    pub fn attach_runtime_summary_state_artifact(
        &mut self,
        artifact: ArtifactRef,
    ) -> Result<(), ResumeCapsuleError> {
        artifact
            .validate()
            .map_err(|_| ResumeCapsuleError::InvalidSerializedForm)?;
        if artifact.provenance.session != self.session_id
            || artifact.provenance.purpose != RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE
            || artifact.media_type != RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE
            || artifact.sensitivity != ArtifactSensitivity::Sensitive
            || artifact.byte_length == 0
            || artifact.byte_length > MAX_SERIALIZED_CAPSULE_BYTES as u64
        {
            return Err(ResumeCapsuleError::InvalidSerializedForm);
        }
        self.latest_summary_state_artifact = Some(artifact);
        Ok(())
    }

    /// Retires a handoff summary when Runtime establishes a different real
    /// parent cache identity (or no cache identity). Ordinary summaries are
    /// identity-independent and remain untouched.
    pub fn retire_handoff_if_identity_changed(&mut self, current: Option<&CacheIdentity>) -> bool {
        let stale = self.semantic_summary.as_ref().is_some_and(|summary| {
            summary.provenance.purpose == ResumeSummaryPurpose::HandoffCheckpoint
                && summary.provenance.cache_identity.as_ref() != current
        });
        if !stale {
            return false;
        }
        let artifact = self
            .semantic_summary
            .take()
            .and_then(|summary| summary.provenance.summary_artifact)
            .map(|artifact| artifact.id.to_string());
        if let Some(artifact) = artifact {
            self.exact_state
                .artifacts
                .retain(|projection| projection.artifact != artifact);
        }
        true
    }

    /// Restores a bounded summary body obtained through the protected
    /// session-owned artifact reference. This does not change exact state or
    /// authorize provider work.
    pub fn restore_summary_body(
        &mut self,
        body: impl Into<String>,
    ) -> Result<(), ResumeCapsuleError> {
        let summary = self
            .semantic_summary
            .as_mut()
            .ok_or(ResumeCapsuleError::InvalidSerializedForm)?;
        summary.body = Some(ProtectedSummaryText::new(body)?);
        summary.provenance.outcome = ResumeSummaryOutcome::Completed;
        Ok(())
    }

    /// Drops an unavailable protected body while retaining its redaction-safe
    /// route metadata.  Summary text and its artifact are optional; exact
    /// structured state remains fully recoverable when the artifact has been
    /// evicted or fails integrity verification.
    fn mark_summary_missing(&mut self) {
        if let Some(summary) = self.semantic_summary.as_mut() {
            summary.body = None;
            summary.provenance.summary_artifact = None;
            summary.provenance.outcome = ResumeSummaryOutcome::Missing;
        }
    }

    /// Returns the redaction-safe machine/status projection.  It omits all
    /// summary text and exact protected interaction/credential material.
    pub fn redacted_projection(&self) -> RedactedResumeCapsule {
        RedactedResumeCapsule {
            schema_version: self.schema_version,
            session_id: self.session_id.clone(),
            parent_turn_id: self.parent_turn_id.clone(),
            created_at: self.created_at,
            model_profile_identity: self.model_profile_identity.clone(),
            agent_profile_identity: self.agent_profile_identity.clone(),
            project_instruction_revision: self.project_instruction_revision.clone(),
            exact_state: self.exact_state.clone(),
            semantic_summary: self
                .semantic_summary
                .as_ref()
                .map(SemanticSummary::redacted_projection),
            latest_summary_state_artifact: self.latest_summary_state_artifact.clone(),
            retained_recent_turns: self.retained_recent_turns.clone(),
            cache: self.cache.clone(),
            last_persisted_watermark: self.last_persisted_watermark,
            last_persisted_at: self.last_persisted_at,
            diagnostics: self.diagnostics.clone(),
        }
    }

    /// Selects protected exact state over canonical state at equal watermark,
    /// and ignores journal replay as an authority.  Summary claims are only
    /// compared for bounded diagnostics.
    pub fn recover(
        &self,
        canonical: Option<ExactStateRecord>,
        protected: Option<ExactStateRecord>,
        journal_watermark: Option<u64>,
    ) -> Result<Option<ResumeRecoveryResult>, ResumeCapsuleError> {
        self.validate()?;
        if let Some(record) = canonical.as_ref() {
            record.validate_for(&self.session_id)?;
        }
        if let Some(record) = protected.as_ref() {
            record.validate_for(&self.session_id)?;
        }
        let (source, selected) = match (canonical, protected) {
            (None, None) => return Ok(None),
            (Some(canonical), None) => (RecoverySource::CanonicalSnapshot, canonical),
            (None, Some(protected)) => (RecoverySource::ProtectedCheckpoint, protected),
            (Some(canonical), Some(protected)) => {
                if protected.watermark >= canonical.watermark {
                    (RecoverySource::ProtectedCheckpoint, protected)
                } else {
                    (RecoverySource::CanonicalSnapshot, canonical)
                }
            }
        };
        let mut capsule = self.clone();
        capsule.replace_exact_state(selected.state, self.created_at);
        let diagnostics = capsule.summary_conflicts(source);
        capsule.diagnostics = diagnostics.clone();
        capsule.validate()?;
        Ok(Some(ResumeRecoveryResult {
            capsule,
            authoritative_source: source,
            diagnostics,
            journal_watermark,
        }))
    }

    /// Returns bounded diagnostics for structured summary claims that conflict
    /// with exact state.  Exact state always wins.
    pub fn summary_conflicts(&self, source: RecoverySource) -> Vec<ResumeDiagnostic> {
        let Some(summary) = self.semantic_summary.as_ref() else {
            return Vec::new();
        };
        let mut conflicts = Vec::new();
        for claim in &summary.claims {
            let conflict = match claim {
                SummaryClaim::ValidationExit {
                    validation,
                    exit_status,
                } => self
                    .exact_state
                    .validations
                    .get(validation)
                    .and_then(|validation| validation.exit_status)
                    .is_some_and(|exact| exact != *exit_status),
                SummaryClaim::ChildState { child, state } => self
                    .exact_state
                    .children
                    .get(child)
                    .is_some_and(|exact| exact.state != *state),
                SummaryClaim::GoalGeneration { generation } => self
                    .exact_state
                    .goal
                    .as_ref()
                    .is_some_and(|goal| goal.generation != *generation),
            };
            if conflict {
                let field = match claim {
                    SummaryClaim::ValidationExit { .. } => ResumeDiagnosticField::Validation,
                    SummaryClaim::ChildState { .. } => ResumeDiagnosticField::Child,
                    SummaryClaim::GoalGeneration { .. } => ResumeDiagnosticField::Goal,
                };
                conflicts.push(ResumeDiagnostic {
                    field,
                    authoritative_source: source,
                });
            }
        }
        conflicts
    }

    /// Performs cold resume reconciliation.  Prior identity remains only a
    /// comparison baseline; provider warmth and guarantee become unknown;
    /// no prewarm request is authorized.
    pub fn cold_resume(&self) -> ColdResumeResult {
        let mut capsule = self.clone();
        let interrupted_children = capsule.exact_state.reconcile_children_after_process_exit();
        capsule.cache.provider_warmth = ResumeCacheWarmth::Unknown;
        capsule.cache.guaranteed_until = None;
        capsule.cache.cold_resume = true;
        ColdResumeResult {
            capsule,
            provider_warmth: ResumeCacheWarmth::Unknown,
            prewarm_requested: false,
            interrupted_children,
        }
    }

    /// Migrates a JSON capsule from the pre-versioned v0 shape to the current
    /// schema.  Unknown future revisions fail closed.
    pub fn from_json_value(mut value: serde_json::Value) -> Result<Self, ResumeCapsuleError> {
        let object = value
            .as_object_mut()
            .ok_or(ResumeCapsuleError::InvalidSerializedForm)?;
        let version = match object.get("schema_version") {
            None => 0,
            Some(value) => {
                let value = value
                    .as_u64()
                    .ok_or(ResumeCapsuleError::InvalidSerializedForm)?;
                u32::try_from(value).map_err(|_| ResumeCapsuleError::UnsupportedSchemaVersion)?
            }
        };
        if version > RESUME_CAPSULE_SCHEMA_VERSION {
            return Err(ResumeCapsuleError::UnsupportedSchemaVersion);
        }
        if version == 0 {
            object.insert(
                "schema_version".to_owned(),
                serde_json::Value::from(RESUME_CAPSULE_SCHEMA_VERSION),
            );
            object
                .entry("exact_state".to_owned())
                .or_insert_with(|| serde_json::json!({}));
            object
                .entry("cache".to_owned())
                .or_insert_with(|| serde_json::json!({}));
            object
                .entry("retained_recent_turns".to_owned())
                .or_insert_with(|| serde_json::json!([]));
            object
                .entry("diagnostics".to_owned())
                .or_insert_with(|| serde_json::json!([]));
        }
        let capsule: Self =
            serde_json::from_value(value).map_err(|_| ResumeCapsuleError::InvalidSerializedForm)?;
        capsule.validate()?;
        Ok(capsule)
    }

    /// Serializes only the redaction-safe projection.
    pub fn to_redacted_json(&self) -> serde_json::Result<serde_json::Value> {
        serde_json::to_value(self.redacted_projection())
    }
}

/// Redaction-safe capsule shape for status/final JSON/streaming output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedResumeCapsule {
    /// Schema revision.
    pub schema_version: u32,
    /// Session id.
    pub session_id: SessionId,
    /// Parent turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<TurnId>,
    /// Creation timestamp.
    pub created_at: Timestamp,
    /// Model profile identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_profile_identity: Option<Fingerprint>,
    /// Agent profile identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_profile_identity: Option<Fingerprint>,
    /// Project instruction revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_instruction_revision: Option<RegistryRevision>,
    /// Exact structured metadata.
    pub exact_state: ExactResumeState,
    /// Summary provenance without body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_summary: Option<RedactedSummary>,
    /// Protected reference to the latest Runtime semantic-summary state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_summary_state_artifact: Option<ArtifactRef>,
    /// Recent canonical turn metadata.
    pub retained_recent_turns: Vec<RecentTurnProjection>,
    /// Cache baseline and explicit unknown warmth.
    pub cache: CacheResumeProjection,
    /// Last persistence watermark.
    pub last_persisted_watermark: u64,
    /// Last persistence boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_persisted_at: Option<Timestamp>,
    /// Bounded diagnostics.
    pub diagnostics: Vec<ResumeDiagnostic>,
}

#[derive(Debug)]
struct ResumeCapsuleSlotState {
    capsule: ResumeCapsule,
    restored_source: Option<RecoverySource>,
}

/// Thread-safe live capsule projection shared by Runtime observers and the
/// existing session/checkpoint persistence adapters.
///
/// The slot applies the approved recovery precedence: the larger exact
/// watermark wins and protected state wins a tie. Semantic prose never
/// participates in that selection or authorizes work.
#[derive(Debug)]
pub struct ResumeCapsuleSlot {
    state: Mutex<ResumeCapsuleSlotState>,
}

impl ResumeCapsuleSlot {
    /// Creates an empty live slot for a root session.
    pub fn new(session: SessionId, created_at: Timestamp) -> Self {
        Self {
            state: Mutex::new(ResumeCapsuleSlotState {
                capsule: ResumeCapsule::new(session, created_at),
                restored_source: None,
            }),
        }
    }

    /// Returns the current exact live capsule without exposing protected
    /// summary text through debug or machine projections.
    pub fn snapshot(&self) -> ResumeCapsule {
        self.state
            .lock()
            .expect("resume capsule slot poisoned")
            .capsule
            .clone()
    }

    /// Mutates the live projection at one host-owned commit boundary.
    pub fn update<R>(&self, update: impl FnOnce(&mut ResumeCapsule) -> R) -> R {
        let mut state = self.state.lock().expect("resume capsule slot poisoned");
        update(&mut state.capsule)
    }

    /// Applies a fallible host projection as one atomic slot mutation.
    ///
    /// The callback edits a private candidate. If any validation step fails,
    /// the live capsule is left untouched. On success, the returned before and
    /// after images can be used with [`Self::restore_if_current`] to roll back
    /// a later persistence failure without erasing a concurrent newer event.
    pub(crate) fn try_update_atomic(
        &self,
        update: impl FnOnce(&mut ResumeCapsule) -> Result<(), ResumeCapsuleError>,
    ) -> Result<(ResumeCapsule, ResumeCapsule), ResumeCapsuleError> {
        let mut state = self.state.lock().expect("resume capsule slot poisoned");
        let previous = state.capsule.clone();
        let mut expected = previous.clone();
        update(&mut expected)?;
        state.capsule = expected.clone();
        Ok((previous, expected))
    }

    /// Encodes the redaction-safe capsule into the existing session extension
    /// state. Protected handoff text is stored separately as a sensitive
    /// session artifact and is skipped by this serialization.
    pub fn versioned_state(&self) -> Result<VersionedSessionState, ResumeCapsuleError> {
        Self::encode(&self.snapshot())
    }

    /// Prepares the current projection for one persistence attempt.
    ///
    /// The live slot can contain canonical state that has not reached a
    /// durable store yet.  Its `last_persisted_*` fields therefore cannot be
    /// stamped in `update_capsule`: doing so makes an unsuccessful save look
    /// successful to observers and recovery.  This method stamps a clone for
    /// the bytes being attempted; [`Self::commit_persisted`] publishes those
    /// markers only after the store confirms success.
    pub(crate) fn prepare_versioned_state(
        &self,
        persisted_at: Timestamp,
    ) -> Result<(ResumeCapsule, VersionedSessionState), ResumeCapsuleError> {
        let mut capsule = self.snapshot();
        capsule.last_persisted_watermark = capsule.exact_state.watermark;
        capsule.last_persisted_at = Some(persisted_at);
        let state = Self::encode(&capsule)?;
        Ok((capsule, state))
    }

    /// Publishes persistence markers after a store has accepted the prepared
    /// projection.  A concurrent event/summary update must not be marked as
    /// durable by an older save, so the commit is conditional on the live
    /// capsule matching the prepared bytes apart from its persistence fields.
    pub(crate) fn commit_persisted(&self, prepared: &ResumeCapsule) -> bool {
        let mut state = self.state.lock().expect("resume capsule slot poisoned");
        let mut comparable = state.capsule.clone();
        comparable.last_persisted_watermark = prepared.last_persisted_watermark;
        comparable.last_persisted_at = prepared.last_persisted_at;
        if comparable != *prepared {
            return false;
        }
        state.capsule.last_persisted_watermark = prepared.last_persisted_watermark;
        state.capsule.last_persisted_at = prepared.last_persisted_at;
        true
    }

    /// Rolls back one failed host-owned projection only when no newer update
    /// has entered the slot since the failed attempt began.  This protects a
    /// later canonical event from being erased by an older asynchronous save
    /// failure.
    pub(crate) fn restore_if_current(
        &self,
        expected: &ResumeCapsule,
        previous: ResumeCapsule,
    ) -> bool {
        let mut state = self.state.lock().expect("resume capsule slot poisoned");
        if state.capsule != *expected {
            return false;
        }
        state.capsule = previous;
        true
    }

    fn encode(capsule: &ResumeCapsule) -> Result<VersionedSessionState, ResumeCapsuleError> {
        capsule.validate()?;
        let value =
            serde_json::to_value(capsule).map_err(|_| ResumeCapsuleError::InvalidSerializedForm)?;
        Ok(VersionedSessionState {
            revision: RegistryRevision::new(RESUME_CAPSULE_STATE_REVISION),
            sensitivity: SessionStateSensitivity::RedactionSafe,
            value,
        })
    }

    /// Restores one canonical or protected candidate while enforcing exact
    /// watermark precedence. Returns whether the candidate became current.
    pub fn restore_versioned_state(
        &self,
        persisted: &VersionedSessionState,
        source: RecoverySource,
    ) -> Result<bool, ResumeCapsuleError> {
        if persisted.revision != RegistryRevision::new(RESUME_CAPSULE_STATE_REVISION) {
            return Err(ResumeCapsuleError::UnsupportedSchemaVersion);
        }
        let candidate = ResumeCapsule::from_json_value(persisted.value.clone())?;
        let mut state = self.state.lock().expect("resume capsule slot poisoned");
        if candidate.session_id != state.capsule.session_id {
            return Err(ResumeCapsuleError::InvalidSerializedForm);
        }
        let candidate_watermark = candidate.exact_state.watermark;
        let current_watermark = state.capsule.exact_state.watermark;
        let wins = state.restored_source.is_none()
            || candidate_watermark > current_watermark
            || (candidate_watermark == current_watermark
                && source == RecoverySource::ProtectedCheckpoint
                && state.restored_source != Some(RecoverySource::ProtectedCheckpoint));
        if !wins {
            return Ok(false);
        }
        let mut candidate = candidate;
        candidate.diagnostics = candidate.summary_conflicts(source);
        state.capsule = candidate;
        state.restored_source = Some(source);
        Ok(true)
    }

    /// Applies cold-process reconciliation once after persisted candidates
    /// have been selected.
    pub fn cold_resume(&self) -> ColdResumeResult {
        let mut state = self.state.lock().expect("resume capsule slot poisoned");
        let result = state.capsule.cold_resume();
        state.capsule = result.capsule.clone();
        result
    }
}

/// Restores a completed summary body from its protected, session-owned
/// artifact. The reference and returned page are verified before UTF-8 text
/// enters live memory; failures never fall back to untrusted prose.
pub async fn restore_summary_artifact(
    slot: &ResumeCapsuleSlot,
    store: &dyn ArtifactStore,
) -> Result<bool, ResumeCapsuleError> {
    let capsule = slot.snapshot();
    let Some(summary) = capsule.semantic_summary.as_ref() else {
        return Ok(false);
    };
    if summary.provenance.outcome != ResumeSummaryOutcome::Completed {
        return Ok(false);
    }
    let Some(reference) = summary.provenance.summary_artifact.clone() else {
        slot.update(ResumeCapsule::mark_summary_missing);
        return Ok(false);
    };
    if reference.provenance.session != capsule.session_id
        || reference.provenance.purpose != summary_artifact_purpose(summary.provenance.purpose)
        || reference.media_type != RESUME_SUMMARY_MEDIA_TYPE
        || reference.byte_length == 0
        || reference.byte_length > MAX_SUMMARY_BYTES as u64
    {
        slot.update(ResumeCapsule::mark_summary_missing);
        return Ok(false);
    }
    let request = ArtifactRead {
        session: capsule.session_id,
        id: reference.id.clone(),
        offset: 0,
        limit: u32::try_from(MAX_SUMMARY_BYTES)
            .expect("summary bound fits u32")
            .min(MAX_ARTIFACT_READ_BYTES),
    };
    let chunk = match store.read(request.clone()).await {
        Ok(chunk) => chunk,
        Err(_) => {
            slot.update(ResumeCapsule::mark_summary_missing);
            return Ok(false);
        }
    };
    if chunk.validate_for(&request).is_err()
        || chunk.reference != reference
        || chunk.next_offset.is_some()
    {
        slot.update(ResumeCapsule::mark_summary_missing);
        return Ok(false);
    }
    let body = match String::from_utf8(chunk.bytes) {
        Ok(body) => body,
        Err(_) => {
            slot.update(ResumeCapsule::mark_summary_missing);
            return Ok(false);
        }
    };
    if slot
        .update(|capsule| capsule.restore_summary_body(body))
        .is_err()
    {
        slot.update(ResumeCapsule::mark_summary_missing);
        return Ok(false);
    }
    Ok(true)
}

/// Reads and validates the latest protected Runtime semantic-summary state.
/// The returned value is still Sensitive extension state; callers must pass
/// it only to Agent Runtime's protected restore seam and never to redacted
/// status or ordinary JSON output.
pub async fn restore_runtime_summary_state(
    slot: &ResumeCapsuleSlot,
    store: &dyn ArtifactStore,
) -> Result<Option<VersionedSessionState>, ResumeCapsuleError> {
    let capsule = slot.snapshot();
    let Some(reference) = capsule.latest_summary_state_artifact.clone() else {
        return Ok(None);
    };
    if reference.provenance.session != capsule.session_id
        || reference.provenance.purpose != RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE
        || reference.media_type != RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE
        || reference.sensitivity != ArtifactSensitivity::Sensitive
        || reference.byte_length == 0
        || reference.byte_length > MAX_SERIALIZED_CAPSULE_BYTES as u64
    {
        return Ok(None);
    }
    let request = ArtifactRead {
        session: capsule.session_id,
        id: reference.id.clone(),
        offset: 0,
        limit: u32::try_from(MAX_SERIALIZED_CAPSULE_BYTES)
            .expect("capsule bound fits u32")
            .min(MAX_ARTIFACT_READ_BYTES),
    };
    let chunk = match store.read(request.clone()).await {
        Ok(chunk) => chunk,
        Err(_) => return Ok(None),
    };
    if chunk.validate_for(&request).is_err()
        || chunk.reference != reference
        || chunk.next_offset.is_some()
    {
        return Ok(None);
    }
    let state: VersionedSessionState = match serde_json::from_slice(&chunk.bytes) {
        Ok(state) => state,
        Err(_) => return Ok(None),
    };
    if state.sensitivity != SessionStateSensitivity::Sensitive {
        return Ok(None);
    }
    Ok(Some(state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::registry::Fingerprint;
    use agent_runtime_core::artifact::{
        ArtifactProvenance, ArtifactRetention, ArtifactSensitivity, ArtifactWrite,
    };

    use crate::artifact::SmithArtifactStore;
    use crate::session::{ProjectId, SessionPaths};

    fn capsule() -> ResumeCapsule {
        ResumeCapsule::new(SessionId::new("session"), Timestamp(1))
    }

    fn identity(label: &str) -> CacheIdentity {
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
        .provider_key(Fingerprint::of(label))
        .stable_prefix(vec![agent_runtime_core::cache::CacheIdentityFragment::new(
            "system",
            Fingerprint::of("system"),
        )])
        .build()
    }

    fn artifact_store(root: &std::path::Path) -> SmithArtifactStore {
        SmithArtifactStore::new(SessionPaths::new(
            root,
            &ProjectId::new("resume-artifact-project").expect("project id"),
        ))
    }

    #[test]
    fn capsule_is_versioned_and_redaction_safe() {
        let mut capsule = capsule();
        capsule
            .record_handoff_summary(
                "provider",
                "model",
                RegistryRevision::new("summary-r1"),
                identity("a"),
                Timestamp(2),
                "PRIVATE_PROMPT_BODY should never be serialized",
                vec![SummaryCoverage::new("turns", 1, 4)],
            )
            .unwrap();
        let json = serde_json::to_string(&capsule.redacted_projection()).unwrap();
        assert_eq!(capsule.schema_version, RESUME_CAPSULE_SCHEMA_VERSION);
        assert!(!json.contains("PRIVATE_PROMPT_BODY"));
        assert!(!json.contains("summary body"));
        assert!(json.contains("handoff_checkpoint"));
    }

    #[test]
    fn exact_validation_state_wins_over_conflicting_summary_claim() {
        let mut capsule = capsule();
        let mut state = ExactResumeState {
            watermark: 5,
            ..ExactResumeState::default()
        };
        state.validations.insert(
            "tests".to_owned(),
            ValidationProjection {
                validation: "tests".to_owned(),
                exit_status: Some(1),
                watermark: 5,
            },
        );
        capsule.commit_exact_state(state.clone(), Timestamp(5));
        let mut summary = SemanticSummary::new(
            ResumeSummaryProvenance::ordinary(
                "provider",
                "small-model",
                RegistryRevision::new("r1"),
                Timestamp(6),
            ),
            Some("tests passed"),
        )
        .unwrap();
        summary
            .push_claim(SummaryClaim::ValidationExit {
                validation: "tests".to_owned(),
                exit_status: 0,
            })
            .unwrap();
        capsule.set_summary(summary).unwrap();
        let diagnostics = capsule.summary_conflicts(RecoverySource::CanonicalSnapshot);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            capsule.exact_state.validations["tests"].exit_status,
            Some(1)
        );
    }

    #[test]
    fn protected_watermark_wins_equal_boundary_and_journal_is_not_authority() {
        let capsule = capsule();
        let canonical = ExactStateRecord::new(
            capsule.session_id.clone(),
            7,
            ExactResumeState {
                unresolved_decisions: 2,
                ..Default::default()
            },
        );
        let protected = ExactStateRecord::new(
            capsule.session_id.clone(),
            7,
            ExactResumeState {
                unresolved_decisions: 1,
                ..Default::default()
            },
        );
        let recovered = capsule
            .recover(Some(canonical), Some(protected), Some(99))
            .unwrap()
            .unwrap();
        assert_eq!(
            recovered.authoritative_source,
            RecoverySource::ProtectedCheckpoint
        );
        assert_eq!(recovered.capsule.exact_state.unresolved_decisions, 1);
        assert_eq!(recovered.journal_watermark, Some(99));
    }

    #[test]
    fn recovery_rejects_foreign_sessions_and_misaligned_records() {
        let capsule = capsule();
        let foreign = ExactStateRecord::new(
            SessionId::new("other-session"),
            2,
            ExactResumeState::default(),
        );
        assert_eq!(
            capsule.recover(Some(foreign), None, None).unwrap_err(),
            ResumeCapsuleError::InvalidSerializedForm
        );

        let mut misaligned =
            ExactStateRecord::new(capsule.session_id.clone(), 3, ExactResumeState::default());
        misaligned.state.watermark = 4;
        assert_eq!(
            capsule.recover(None, Some(misaligned), None).unwrap_err(),
            ResumeCapsuleError::InvalidSerializedForm
        );
    }

    #[test]
    fn total_serialized_capsule_size_is_bounded() {
        let mut capsule = capsule();
        capsule.exact_state.changed_files = (0..MAX_CHANGED_FILES)
            .map(|index| ChangedFileProjection {
                path: format!("{index:04}-{}", "x".repeat(MAX_METADATA_BYTES - 6)),
                additions: 0,
                deletions: 0,
                digest: Some(Fingerprint::of(index.to_string())),
            })
            .collect();
        assert_eq!(
            capsule.validate().unwrap_err(),
            ResumeCapsuleError::ProjectionLimit
        );
    }

    #[test]
    fn persisted_handoff_identity_must_match_the_cache_baseline() {
        let mut capsule = capsule();
        capsule.cache.prior_identity = Some(identity("baseline"));
        capsule.semantic_summary = Some(
            SemanticSummary::new(
                ResumeSummaryProvenance::handoff(
                    "provider",
                    "model",
                    RegistryRevision::new("summary-r1"),
                    identity("tampered"),
                    Timestamp(2),
                ),
                Some("bounded"),
            )
            .unwrap(),
        );
        assert_eq!(
            capsule.validate().unwrap_err(),
            ResumeCapsuleError::HandoffIdentityMismatch
        );
    }

    #[test]
    fn cold_resume_makes_warmth_unknown_and_interrupts_only_live_children() {
        let mut capsule = capsule();
        let child_running = ChildId::new("running");
        let child_done = ChildId::new("done");
        capsule.exact_state.children.insert(
            child_running.clone(),
            ChildResumeProjection {
                child: child_running.clone(),
                task_digest: Some(Fingerprint::of("private task")),
                state: ChildLifecycleState::Running,
                terminal_outcome: None,
                watermark: 4,
            },
        );
        capsule.exact_state.children.insert(
            child_done.clone(),
            ChildResumeProjection {
                child: child_done.clone(),
                task_digest: None,
                state: ChildLifecycleState::Completed,
                terminal_outcome: Some(ChildTerminalOutcome {
                    state: ChildLifecycleState::Completed,
                    result_digest: None,
                    watermark: 5,
                }),
                watermark: 5,
            },
        );
        capsule.cache.prior_identity = Some(identity("warm-before-restart"));
        capsule.cache.provider_warmth = ResumeCacheWarmth::WarmObserved;
        capsule.cache.guaranteed_until = Some(Timestamp(99));
        capsule.cache.last_meaningful_activity_at = Some(Timestamp(42));
        capsule.cache.idle_compaction_interval_id = Some("root-turn:7".to_owned());
        capsule.cache.idle_compaction_attempted = true;
        let cold = capsule.cold_resume();
        assert_eq!(cold.provider_warmth, ResumeCacheWarmth::Unknown);
        assert!(!cold.prewarm_requested);
        assert_eq!(cold.interrupted_children, vec![child_running]);
        assert_eq!(
            cold.capsule.cache.provider_warmth,
            ResumeCacheWarmth::Unknown
        );
        assert_eq!(cold.capsule.cache.guaranteed_until, None);
        assert_eq!(
            cold.capsule.cache.prior_identity,
            capsule.cache.prior_identity
        );
        assert_eq!(
            cold.capsule.cache.last_meaningful_activity_at,
            Some(Timestamp(42))
        );
        assert_eq!(
            cold.capsule.cache.idle_compaction_interval_id.as_deref(),
            Some("root-turn:7")
        );
        assert!(cold.capsule.cache.idle_compaction_attempted);
        assert_eq!(
            cold.capsule.exact_state.children[&child_done].state,
            ChildLifecycleState::Completed
        );
    }

    #[test]
    fn handoff_and_ordinary_summary_routes_remain_distinct() {
        let id = identity("parent");
        let mut capsule = capsule();
        capsule
            .record_handoff_summary(
                "provider",
                "model",
                RegistryRevision::new("handoff-r1"),
                id.clone(),
                Timestamp(2),
                "handoff",
                vec![],
            )
            .unwrap();
        assert_eq!(
            capsule
                .semantic_summary
                .as_ref()
                .unwrap()
                .provenance
                .purpose,
            ResumeSummaryPurpose::HandoffCheckpoint
        );
        assert_eq!(
            capsule
                .semantic_summary
                .as_ref()
                .unwrap()
                .provenance
                .cache_identity,
            Some(id)
        );
        capsule
            .record_ordinary_summary(
                "other-provider",
                "small-model",
                RegistryRevision::new("summary-r2"),
                Timestamp(3),
                "ordinary",
                vec![],
            )
            .unwrap();
        let summary = capsule.semantic_summary.as_ref().unwrap();
        assert_eq!(
            summary.provenance.purpose,
            ResumeSummaryPurpose::OrdinarySummary
        );
        assert_eq!(summary.provenance.cache_identity, None);
    }

    #[test]
    fn a_real_parent_identity_change_retires_only_identity_bound_handoffs() {
        let mut capsule = capsule();
        let first = identity("first");
        capsule.cache.prior_identity = Some(first.clone());
        capsule
            .record_handoff_summary(
                "provider",
                "model",
                RegistryRevision::new("handoff-r1"),
                first.clone(),
                Timestamp(2),
                "handoff",
                vec![],
            )
            .unwrap();

        assert!(!capsule.retire_handoff_if_identity_changed(Some(&first)));
        assert!(capsule.semantic_summary.is_some());
        assert!(capsule.retire_handoff_if_identity_changed(Some(&identity("second"))));
        assert!(capsule.semantic_summary.is_none());

        capsule
            .record_ordinary_summary(
                "summary-provider",
                "summary-model",
                RegistryRevision::new("ordinary-r2"),
                Timestamp(3),
                "ordinary",
                vec![],
            )
            .unwrap();
        assert!(!capsule.retire_handoff_if_identity_changed(None));
        assert_eq!(
            capsule
                .semantic_summary
                .as_ref()
                .map(|summary| summary.provenance.purpose),
            Some(ResumeSummaryPurpose::OrdinarySummary)
        );
    }

    #[test]
    fn synthetic_turns_have_no_canonical_recent_turn_slot() {
        let mut capsule = capsule();
        capsule
            .push_recent_turn(RecentTurnProjection {
                turn: TurnId::new("real"),
                role: RecentTurnRole::Assistant,
                content_digest: Some(Fingerprint::of("real")),
            })
            .unwrap();
        let json = serde_json::to_string(&capsule.redacted_projection()).unwrap();
        assert!(json.contains("real"));
        assert!(!json.contains("ping"));
        assert!(!json.contains("pong"));
    }

    #[test]
    fn v0_capsule_migrates_to_current_schema() {
        let value = serde_json::json!({
            "session_id": "session",
            "created_at": 1,
        });
        let migrated = ResumeCapsule::from_json_value(value).unwrap();
        assert_eq!(migrated.schema_version, RESUME_CAPSULE_SCHEMA_VERSION);
        assert_eq!(migrated.session_id, SessionId::new("session"));
    }

    #[test]
    fn summary_body_is_bounded() {
        let provenance = ResumeSummaryProvenance::ordinary(
            "provider",
            "model",
            RegistryRevision::new("r1"),
            Timestamp(1),
        );
        let body = "x".repeat(MAX_SUMMARY_BYTES + 1);
        assert_eq!(
            SemanticSummary::new(provenance, Some(body)).unwrap_err(),
            ResumeCapsuleError::SummaryTooLarge
        );
    }

    #[test]
    fn persistence_markers_commit_only_after_the_prepared_save_succeeds() {
        let slot = ResumeCapsuleSlot::new(SessionId::new("session"), Timestamp(1));
        slot.update(|capsule| capsule.exact_state.watermark = 7);

        let (prepared, persisted) = slot
            .prepare_versioned_state(Timestamp(9))
            .expect("prepare a bounded capsule write");
        assert_eq!(prepared.last_persisted_watermark, 7);
        assert_eq!(prepared.last_persisted_at, Some(Timestamp(9)));
        assert_eq!(persisted.value["last_persisted_watermark"], 7);
        assert_eq!(persisted.value["last_persisted_at"], 9);

        // A failed store must not make the live projection claim that this
        // watermark reached durable storage.
        let live = slot.snapshot();
        assert_eq!(live.last_persisted_watermark, 0);
        assert_eq!(live.last_persisted_at, None);

        assert!(slot.commit_persisted(&prepared));
        let committed = slot.snapshot();
        assert_eq!(committed.last_persisted_watermark, 7);
        assert_eq!(committed.last_persisted_at, Some(Timestamp(9)));
        assert!(slot.versioned_state().is_ok());
    }

    #[test]
    fn an_older_save_cannot_mark_a_newer_capsule_as_durable() {
        let slot = ResumeCapsuleSlot::new(SessionId::new("session"), Timestamp(1));
        slot.update(|capsule| capsule.exact_state.watermark = 7);
        let (prepared, _) = slot
            .prepare_versioned_state(Timestamp(9))
            .expect("prepare the older write");
        slot.update(|capsule| capsule.exact_state.watermark = 8);

        assert!(!slot.commit_persisted(&prepared));
        assert_eq!(slot.snapshot().last_persisted_watermark, 0);
    }

    #[test]
    fn fallible_slot_updates_publish_all_or_nothing() {
        let slot = ResumeCapsuleSlot::new(SessionId::new("session"), Timestamp(1));
        let before = slot.snapshot();

        let result = slot.try_update_atomic(|candidate| {
            candidate.cache.idle_compaction_attempted = true;
            Err(ResumeCapsuleError::InvalidSerializedForm)
        });

        assert_eq!(
            result.unwrap_err(),
            ResumeCapsuleError::InvalidSerializedForm
        );
        assert_eq!(slot.snapshot(), before);
    }

    #[tokio::test]
    async fn ordinary_summary_artifact_restores_without_serializing_the_body() {
        let root = tempfile::tempdir().expect("artifact root");
        let store = artifact_store(root.path());
        let session = SessionId::new("session");
        let body = "PRIVATE_ORDINARY_SUMMARY_BODY";
        let reference = store
            .put(ArtifactWrite {
                bytes: body.as_bytes().to_vec(),
                media_type: RESUME_SUMMARY_MEDIA_TYPE.to_owned(),
                sensitivity: ArtifactSensitivity::Sensitive,
                retention: ArtifactRetention::Session,
                provenance: ArtifactProvenance::new(
                    session.clone(),
                    RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
                ),
                idempotency_key: "ordinary-summary-r1".to_owned(),
            })
            .await
            .expect("write summary artifact");
        let slot = ResumeCapsuleSlot::new(session.clone(), Timestamp(1));
        slot.update(|capsule| {
            capsule.record_ordinary_summary(
                "provider",
                "summary-model",
                RegistryRevision::new("summary-r1"),
                Timestamp(2),
                body,
                vec![SummaryCoverage::new("canonical_events", 0, 4)],
            )?;
            capsule.attach_summary_artifact(reference)
        })
        .expect("attach summary artifact");

        let persisted = slot.versioned_state().expect("versioned capsule");
        let serialized = serde_json::to_string(&persisted.value).expect("capsule JSON");
        assert!(!serialized.contains(body));
        assert!(serialized.contains(RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE));

        let restored = ResumeCapsuleSlot::new(session, Timestamp(9));
        assert!(
            restored
                .restore_versioned_state(&persisted, RecoverySource::CanonicalSnapshot)
                .expect("restore capsule")
        );
        assert!(
            restore_summary_artifact(&restored, &store)
                .await
                .expect("restore summary body")
        );
        let snapshot = restored.snapshot();
        let summary = snapshot.semantic_summary.expect("summary metadata");
        assert_eq!(
            summary.provenance.purpose,
            ResumeSummaryPurpose::OrdinarySummary
        );
        assert_eq!(
            summary.body.as_ref().map(ProtectedSummaryText::as_str),
            Some(body)
        );
    }

    #[tokio::test]
    async fn runtime_summary_state_artifact_round_trips_as_sensitive_extension_state() {
        let root = tempfile::tempdir().expect("artifact root");
        let store = artifact_store(root.path());
        let session = SessionId::new("session");
        let state = VersionedSessionState::new(
            RegistryRevision::new("harness.semantic-summary:model-r1"),
            serde_json::json!({
                "purpose": "context.semantic_summary",
                "summary": "PRIVATE_RUNTIME_SUMMARY"
            }),
        );
        let bytes = serde_json::to_vec(&state).expect("serialize runtime state");
        let reference = store
            .put(ArtifactWrite {
                bytes,
                media_type: RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE.to_owned(),
                sensitivity: ArtifactSensitivity::Sensitive,
                retention: ArtifactRetention::Session,
                provenance: ArtifactProvenance::new(
                    session.clone(),
                    RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE,
                ),
                idempotency_key: "runtime-summary-state-r1".to_owned(),
            })
            .await
            .expect("write runtime summary state");
        let mut public_reference = reference.clone();
        public_reference.sensitivity = ArtifactSensitivity::Public;
        let rejected = ResumeCapsuleSlot::new(session.clone(), Timestamp(1));
        assert_eq!(
            rejected
                .update(|capsule| {
                    capsule.attach_runtime_summary_state_artifact(public_reference)
                })
                .unwrap_err(),
            ResumeCapsuleError::InvalidSerializedForm,
            "a public reference must never authorize reading protected Runtime state"
        );
        assert!(rejected.snapshot().latest_summary_state_artifact.is_none());

        let slot = ResumeCapsuleSlot::new(session.clone(), Timestamp(1));
        slot.update(|capsule| capsule.attach_runtime_summary_state_artifact(reference))
            .expect("attach runtime state reference");
        let persisted = slot.versioned_state().expect("versioned capsule");
        let restored = ResumeCapsuleSlot::new(session, Timestamp(2));
        restored
            .restore_versioned_state(&persisted, RecoverySource::CanonicalSnapshot)
            .expect("restore capsule");
        let recovered = restore_runtime_summary_state(&restored, &store)
            .await
            .expect("read runtime summary state")
            .expect("runtime summary state exists");
        assert_eq!(recovered.sensitivity, SessionStateSensitivity::Sensitive);
        assert_eq!(recovered.value["purpose"], "context.semantic_summary");
        assert_eq!(recovered.value["summary"], "PRIVATE_RUNTIME_SUMMARY");
    }

    #[tokio::test]
    async fn malformed_summary_artifact_never_becomes_live_text() {
        let root = tempfile::tempdir().expect("artifact root");
        let store = artifact_store(root.path());
        let session = SessionId::new("session");
        let reference = store
            .put(ArtifactWrite {
                bytes: vec![0xff, 0xfe],
                media_type: RESUME_SUMMARY_MEDIA_TYPE.to_owned(),
                sensitivity: ArtifactSensitivity::Sensitive,
                retention: ArtifactRetention::Session,
                provenance: ArtifactProvenance::new(
                    session.clone(),
                    RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
                ),
                idempotency_key: "invalid-utf8-summary".to_owned(),
            })
            .await
            .expect("write invalid UTF-8 artifact");
        let slot = ResumeCapsuleSlot::new(session, Timestamp(1));
        slot.update(|capsule| {
            capsule.record_ordinary_summary(
                "provider",
                "summary-model",
                RegistryRevision::new("summary-r1"),
                Timestamp(2),
                "live body discarded before simulated restart",
                vec![],
            )?;
            capsule.attach_summary_artifact(reference)
        })
        .expect("attach artifact metadata");
        let persisted = slot.versioned_state().expect("versioned capsule");
        let restored = ResumeCapsuleSlot::new(SessionId::new("session"), Timestamp(3));
        restored
            .restore_versioned_state(&persisted, RecoverySource::ProtectedCheckpoint)
            .expect("restore metadata");

        assert!(
            !restore_summary_artifact(&restored, &store)
                .await
                .expect("invalid UTF-8 is an optional summary failure")
        );
        let snapshot = restored.snapshot();
        let summary = snapshot.semantic_summary.expect("summary metadata");
        assert_eq!(summary.provenance.outcome, ResumeSummaryOutcome::Missing);
        assert!(summary.provenance.summary_artifact.is_none());
        assert!(summary.body.is_none());
    }

    #[tokio::test]
    async fn missing_summary_artifact_degrades_without_losing_exact_state() {
        let source_root = tempfile::tempdir().expect("source artifact root");
        let missing_root = tempfile::tempdir().expect("missing artifact root");
        let source = artifact_store(source_root.path());
        let missing = artifact_store(missing_root.path());
        let session = SessionId::new("session");
        let reference = source
            .put(ArtifactWrite {
                bytes: b"summary body".to_vec(),
                media_type: RESUME_SUMMARY_MEDIA_TYPE.to_owned(),
                sensitivity: ArtifactSensitivity::Sensitive,
                retention: ArtifactRetention::Session,
                provenance: ArtifactProvenance::new(
                    session.clone(),
                    RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
                ),
                idempotency_key: "missing-summary-r1".to_owned(),
            })
            .await
            .expect("write summary artifact");
        let slot = ResumeCapsuleSlot::new(session.clone(), Timestamp(1));
        slot.update(|capsule| {
            capsule.record_ordinary_summary(
                "provider",
                "summary-model",
                RegistryRevision::new("summary-r1"),
                Timestamp(2),
                "summary body",
                vec![],
            )?;
            capsule.attach_summary_artifact(reference)?;
            let state = ExactResumeState {
                watermark: 7,
                unresolved_decisions: 2,
                ..ExactResumeState::default()
            };
            assert!(capsule.commit_exact_state(state, Timestamp(7)));
            Ok::<_, ResumeCapsuleError>(())
        })
        .expect("attach summary and exact state");

        let persisted = slot.versioned_state().expect("versioned capsule");
        let restored = ResumeCapsuleSlot::new(session, Timestamp(3));
        restored
            .restore_versioned_state(&persisted, RecoverySource::ProtectedCheckpoint)
            .expect("restore metadata");
        let exact_before = restored.snapshot().exact_state.clone();

        assert!(
            !restore_summary_artifact(&restored, &missing)
                .await
                .expect("missing artifact is an optional summary failure")
        );
        let snapshot = restored.snapshot();
        assert_eq!(snapshot.exact_state, exact_before);
        let summary = snapshot.semantic_summary.expect("summary metadata");
        assert_eq!(summary.provenance.outcome, ResumeSummaryOutcome::Missing);
        assert!(summary.provenance.summary_artifact.is_none());
        assert!(summary.body.is_none());
    }
}
