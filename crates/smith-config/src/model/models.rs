//! Per-model metadata and cache policy overrides.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{ContextWindowSection, ModelReasoningSection};

/// Limits and capability metadata for one `"<provider>/<model>"` pair.
///
/// Each field is independently optional because a catalog source may supply
/// the rest. Smith supplies none of them by default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSection {
    /// Cache lifecycle defaults for this provider/model route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<ModelCacheSection>,
    /// Accepted input modalities, overriding catalog metadata for this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_modalities: Option<Vec<crate::catalog::CatalogModality>>,
    /// Total context window, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u32>,
    /// The largest input Smith may plan for, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    /// The largest output the model can produce, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// Named context limits: `[models."p/m".context_windows.<name>]`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context_windows: BTreeMap<String, ContextWindowSection>,
    /// Name selected when no profile, flag, or session override wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_context_window: Option<String>,
    /// Exact, owner-controlled reasoning capability metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ModelReasoningSection>,
}

/// Model-specific host inactivity cutoff, independent of server cache TTL.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCacheSection {
    /// Meaningful inactivity before idle compaction, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inactivity_limit_ms: Option<u64>,
}
