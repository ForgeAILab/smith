use serde::{Deserialize, Serialize};

use super::ReasoningSection;

/// Ordered model alternatives within one agent profile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFallbackSection {
    /// Whole replacement list; an empty list disables inherited fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<FallbackModelSection>>,
    /// Budget until meaningful response progress, shared across retries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_response_timeout_ms: Option<u64>,
}

/// Model-specific overrides for one ordered fallback binding.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FallbackModelSection {
    /// Name of an existing provider declaration.
    pub provider: String,
    /// Model served by that provider.
    pub model: String,
    /// Overrides the profile's startup budget for this candidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_response_timeout_ms: Option<u64>,
    /// Candidate-specific generation ask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// Named window belonging to this candidate's model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<String>,
    /// Candidate-specific reasoning controls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningSection>,
}
