use super::*;

pub(super) fn validate_coverage(coverage: &[SummaryCoverage]) -> Result<(), ResumeCapsuleError> {
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

pub(super) const fn summary_artifact_purpose(purpose: ResumeSummaryPurpose) -> &'static str {
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
    pub(super) body: Option<ProtectedSummaryText>,
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
