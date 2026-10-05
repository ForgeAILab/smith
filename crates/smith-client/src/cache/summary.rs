use super::*;
/// Smith's presentation state. These are direct projections of canonical
/// runtime states. Smith also uses `Suspended` when an identity switch makes
/// the previous identity's evidence inapplicable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CacheVisibilityState {
    /// The provider cannot honor this plan.
    Unsupported,
    /// No cache evidence was supplied.
    #[default]
    Unknown,
    /// The plan is reusable but has no comparable positive result yet.
    Eligible,
    /// A provider read was observed.
    WarmObserved,
    /// The provider read was below the runtime expectation.
    MissObserved,
    /// The provider explicitly reported expiry for the exact identity.
    Expired,
    /// Runtime suspended maintenance, or an identity switch invalidated the
    /// previous identity's projection.
    Suspended,
}

impl From<CacheState> for CacheVisibilityState {
    fn from(value: CacheState) -> Self {
        match value {
            CacheState::Unsupported => Self::Unsupported,
            CacheState::Unknown => Self::Unknown,
            CacheState::Eligible => Self::Eligible,
            CacheState::WarmObserved => Self::WarmObserved,
            CacheState::MissObserved => Self::MissObserved,
            CacheState::Expired => Self::Expired,
            CacheState::Suspended => Self::Suspended,
        }
    }
}

impl CacheVisibilityState {
    /// Stable wire/display label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
            Self::Eligible => "eligible",
            Self::WarmObserved => "warm_observed",
            Self::MissObserved => "miss_observed",
            Self::Expired => "expired",
            Self::Suspended => "suspended",
        }
    }
}

/// A resolved per-model price used only for derived cache-miss cost.
///
/// Rates are micro-USD per million tokens.  `None` means the catalog did not
/// publish a compatible rate; callers must keep extra cost unknown rather
/// than substituting another model's price.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CachePrice {
    /// Uncached input rate.
    pub input: Option<u64>,
    /// Cache-read rate.
    pub cache_read: Option<u64>,
    /// Cache-write rate.
    pub cache_write: Option<u64>,
}

impl From<&smith_config::catalog::CatalogModelCost> for CachePrice {
    fn from(cost: &smith_config::catalog::CatalogModelCost) -> Self {
        Self {
            input: cost.input,
            cache_read: cost.cache_read,
            cache_write: cost.cache_write,
        }
    }
}

/// A completed root-turn cache projection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CacheTurnSummary {
    /// The completed root turn identity.
    pub turn: String,
    /// Aggregate canonical state for the turn.
    pub state: CacheVisibilityState,
    /// Exact redaction-safe Runtime cache-identity digest, when all evidence
    /// for the turn was correlated to the same identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_identity: Option<String>,
    /// Expected reusable read tokens, when every cache-evidence-bearing
    /// attempt supplied it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_read_tokens: Option<u64>,
    /// Observed cache-read tokens, preserving explicit zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_read_tokens: Option<u64>,
    /// Observed cache-write tokens, preserving explicit zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_write_tokens: Option<u64>,
    /// Runtime-derived missed tokens, when canonical evidence supplied them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missed_tokens: Option<u64>,
    /// Planner confidence, omitted when attempts disagree or no evidence was
    /// supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<EstimationConfidence>,
    /// Provider-reported cache-read share of prompt input, rounded to a whole
    /// percent.  `Some(0)` is an explicit zero; `None` means evidence is
    /// absent or prompt input could not be attributed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_percent: Option<u8>,
    /// Number of billed attempts with a positive canonical shortfall.
    pub miss_count: u32,
    /// Missed tokens as a separate derived diagnostic, never a usage counter.
    pub rebilled_tokens: u64,
    /// Factual elapsed idle context before the first miss-bearing logical
    /// request, in whole minutes.  This never implies expiry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_minutes: Option<u64>,
    /// Derived extra cost when a compatible price was supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_cost_micro_usd: Option<u128>,
}

/// The latest canonical phase observed for one Runtime cache operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheOperationDisposition {
    /// Runtime preflight accepted the operation.
    Prepared,
    /// Runtime rejected the operation before provider I/O.
    Rejected,
    /// The operation crossed provider admission.
    Started,
    /// The operation reached a terminal result.
    Completed,
    /// Runtime suspended maintenance for the exact identity.
    Suspended,
}

/// Bounded projection of one canonical Runtime cache-operation lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheOperationSummary {
    /// Stable upstream operation identity.
    pub operation: String,
    /// Exact redaction-safe Runtime cache-identity digest.
    pub cache_identity: String,
    /// Typed provider-attempt purpose.
    pub purpose: ProviderAttemptPurpose,
    /// Latest canonical lifecycle phase.
    pub disposition: CacheOperationDisposition,
    /// Logical request, when Runtime allocated one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    /// Provider attempt, when the operation crossed provider admission.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<String>,
    /// Terminal result, when completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CacheOperationOutcome>,
    /// Structured rejection, failure, or suspension reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CacheOperationReason>,
    /// Bounded Runtime metrics; never provider bodies.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metrics: BTreeMap<String, u64>,
}

/// Canonical provider evidence and operation facts accumulated for a session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CacheLifecycleSummary {
    /// Latest exact redaction-safe Runtime cache-identity digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_identity: Option<String>,
    /// Latest typed provider evidence kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<CacheEvidenceKind>,
    /// Provider-declared guarantee boundary, in Runtime clock milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guaranteed_until_ms: Option<u64>,
    /// Explicit resource existence, preserving omitted versus false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_exists: Option<bool>,
    /// Number of canonical operations that crossed provider admission.
    pub maintenance_calls_used: u32,
    /// Latest canonical operation lifecycle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_operation: Option<CacheOperationSummary>,
    /// Latest canonical maintenance-suspension reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suspension_reason: Option<CacheOperationReason>,
}

/// Provider-reported cache reads, without implying a lease or cache guarantee.
/// Missing counters are not zero. The lifecycle may be unsupported even when
/// the provider reports cached input, so do not use it to gate this display.
pub fn render_cache_read_usage(percent: Option<u8>, tokens: Option<u64>) -> Option<String> {
    render_cache_read_usage_value(percent, tokens).map(|value| format!("prompt cache: {value}"))
}

fn render_cache_read_usage_value(percent: Option<u8>, tokens: Option<u64>) -> Option<String> {
    match (percent, tokens) {
        (Some(percent), Some(tokens)) => Some(format!(
            "{percent}% of input read from cache · {} cached tokens",
            compact_tokens(tokens),
        )),
        (Some(percent), None) => Some(format!("{percent}% of input read from cache")),
        (None, Some(tokens)) => Some(format!("{} cached input tokens", compact_tokens(tokens))),
        (None, None) => None,
    }
}

impl CacheTurnSummary {
    /// Usage from the last completed root turn; not a prediction for the next.
    pub fn render_usage(&self) -> Option<String> {
        self.render_usage_value()
            .map(|value| format!("prompt cache: {value}"))
    }

    /// The usage value without a field label, for typed local reports.
    pub fn render_usage_value(&self) -> Option<String> {
        render_cache_read_usage_value(self.cache_read_percent, self.observed_read_tokens)
            .map(|value| format!("{value} (last turn)"))
    }
    /// The footer's compact cache-hit metric.
    pub fn render_ch(&self) -> String {
        self.cache_read_percent
            .map_or_else(|| "?".to_owned(), |percent| format!("{percent}%"))
    }

    /// Whether this summary crosses Smith's fixed notice threshold.
    pub fn significant(&self) -> bool {
        self.rebilled_tokens >= MISS_NOTICE_TOKENS
            || self
                .extra_cost_micro_usd
                .is_some_and(|cost| cost >= MISS_NOTICE_COST_MICRO_USD)
    }

    /// A bounded factual local notice.  It intentionally says idle, never
    /// expired or likely expired.
    pub fn render_notice(&self) -> String {
        let mut text = String::from("Cache miss");
        if let Some(minutes) = self.idle_minutes.filter(|minutes| *minutes > 0) {
            text.push_str(&format!(" after {minutes}m idle"));
        }
        text.push_str(&format!(
            " · re-billed {}",
            compact_tokens(self.rebilled_tokens)
        ));
        if let (Some(expected), Some(observed)) =
            (self.expected_read_tokens, self.observed_read_tokens)
        {
            text.push_str(&format!(
                " · expected {} · observed {}",
                compact_tokens(expected),
                compact_tokens(observed)
            ));
        }
        if let Some(cost) = self.extra_cost_micro_usd {
            text.push_str(&format!(" · +{} derived", format_usd(cost)));
        }
        text
    }
}
