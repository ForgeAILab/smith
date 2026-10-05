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

mod exact;
mod persistence;
mod summary;

use summary::{summary_artifact_purpose, validate_coverage};

pub use exact::{
    ArtifactProjection, CacheResumeProjection, ChangedFileProjection, ChildLifecycleState,
    ChildResumeProjection, ChildTerminalOutcome, ExactGoalProjection, ExactPlanProjection,
    ExactResumeState, RecentTurnProjection, RecentTurnRole, ResumeCacheWarmth,
    ValidationProjection,
};
pub use persistence::{ResumeCapsuleSlot, restore_runtime_summary_state, restore_summary_artifact};
pub use summary::{
    ProtectedSummaryText, RedactedSummary, RedactedSummaryProvenance, ResumeSummaryOutcome,
    ResumeSummaryProvenance, ResumeSummaryPurpose, SemanticSummary, SummaryClaim, SummaryCoverage,
    SummaryUsage,
};

#[cfg(test)]
mod tests;

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
