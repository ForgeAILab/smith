//! Smith-owned data used by guided provider and model setup.
//!
//! Descriptors are deliberately static and local. Building this catalog opens
//! no credential service and performs no provider or network I/O. The caller
//! supplies the adapter kinds compiled into its pinned runtime, so a preset can
//! never be offered by silently substituting another wire protocol.

use std::collections::BTreeSet;

use crate::model::{KIND_OPENAI_COMPATIBLE, ReasoningOnlyBehavior};

/// Revision of the trusted model data shipped with this Smith build.
pub const TRUSTED_MODEL_CATALOG_REVISION: u32 = 1;

/// Stable name recorded for Smith's built-in setup model data.
pub const TRUSTED_MODEL_CATALOG_NAME: &str = "smith-trusted-models";
/// Provider name used by the GLM quick start.
pub const GLM_PROVIDER: &str = "zai";
/// Default profile created by the GLM quick start.
pub const GLM_PROFILE: &str = "glm";
/// Z.AI Coding Plan OpenAI-compatible endpoint.
pub const GLM_ENDPOINT: &str = "https://api.z.ai/api/coding/paas/v4";

/// The credential enrollment paths a setup descriptor permits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSetupMethod {
    /// Store an entered value in the operating-system credential service.
    Keychain,
    /// Record an environment-variable reference without reading its value.
    Environment,
}

/// Complete, versioned enforcement metadata for one known model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustedModelRecord {
    /// Provider identity the record is scoped to.
    pub provider: &'static str,
    /// Provider model identifier.
    pub model: &'static str,
    /// Human-facing model label.
    pub label: &'static str,
    /// Stable source name.
    pub catalog: &'static str,
    /// Source revision.
    pub revision: u32,
    /// Total model context window.
    pub context_tokens: u32,
    /// Maximum enforceable model input.
    pub max_input_tokens: u32,
    /// Maximum model output.
    pub max_output_tokens: u32,
    /// Default request-time generation cap.
    pub request_output_tokens: u32,
    /// Default output reserve used by context planning.
    pub output_reserve: u32,
}

/// One provider path shown by setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderSetupDescriptor {
    /// Stable setup choice identifier.
    pub id: &'static str,
    /// Human-facing name.
    pub label: &'static str,
    /// Short explanation shown beside the choice.
    pub description: &'static str,
    /// Provider name proposed in user configuration, if the path has one.
    pub provider: Option<&'static str>,
    /// Profile name proposed for a quick start, if the path has one.
    pub profile: Option<&'static str>,
    /// Shared runtime adapter kind.
    pub adapter: &'static str,
    /// Proposed endpoint; custom paths collect one instead.
    pub endpoint: Option<&'static str>,
    /// Credential enrollment methods this path supports.
    pub credentials: &'static [CredentialSetupMethod],
    /// Trusted models offered by this descriptor.
    pub models: &'static [TrustedModelRecord],
    /// Opt-in reasoning-only response behavior.
    pub reasoning_only: Option<ReasoningOnlyBehavior>,
}

const CREDENTIAL_METHODS: &[CredentialSetupMethod] = &[
    CredentialSetupMethod::Keychain,
    CredentialSetupMethod::Environment,
];

/// GLM-4.7 metadata verified for the Z.AI Coding endpoint.
pub const GLM_4_7: TrustedModelRecord = TrustedModelRecord {
    provider: GLM_PROVIDER,
    model: "glm-4.7",
    label: "GLM-4.7",
    catalog: TRUSTED_MODEL_CATALOG_NAME,
    revision: TRUSTED_MODEL_CATALOG_REVISION,
    context_tokens: 200_000,
    max_input_tokens: 196_000,
    max_output_tokens: 131_072,
    request_output_tokens: 8_192,
    output_reserve: 8_192,
};

const GLM_MODELS: &[TrustedModelRecord] = &[GLM_4_7];

const DESCRIPTORS: &[ProviderSetupDescriptor] = &[
    ProviderSetupDescriptor {
        id: "glm",
        label: "Quick start with GLM",
        description: "Z.AI Coding endpoint with trusted GLM-4.7 limits",
        provider: Some(GLM_PROVIDER),
        profile: Some(GLM_PROFILE),
        adapter: KIND_OPENAI_COMPATIBLE,
        endpoint: Some(GLM_ENDPOINT),
        credentials: CREDENTIAL_METHODS,
        models: GLM_MODELS,
        reasoning_only: Some(ReasoningOnlyBehavior::Text),
    },
    ProviderSetupDescriptor {
        id: "openai-compatible",
        label: "Custom OpenAI-compatible provider",
        description: "Enter an endpoint, model ID, and enforceable model limits",
        provider: None,
        profile: None,
        adapter: KIND_OPENAI_COMPATIBLE,
        endpoint: None,
        credentials: CREDENTIAL_METHODS,
        models: &[],
        reasoning_only: None,
    },
];

/// Returns setup choices whose adapter is present in the pinned runtime.
///
/// Ordering is stable and owned by Smith rather than by the caller's set.
pub fn provider_descriptors(available_adapter_kinds: &[&str]) -> Vec<ProviderSetupDescriptor> {
    let available: BTreeSet<&str> = available_adapter_kinds.iter().copied().collect();
    DESCRIPTORS
        .iter()
        .copied()
        .filter(|descriptor| available.contains(descriptor.adapter))
        .collect()
}

/// Finds trusted model metadata for one provider-qualified identity.
pub fn trusted_model(provider: &str, model: &str) -> Option<&'static TrustedModelRecord> {
    DESCRIPTORS
        .iter()
        .flat_map(|descriptor| descriptor.models)
        .find(|record| record.provider == provider && record.model == model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_are_filtered_without_adapter_substitution() {
        assert!(provider_descriptors(&[]).is_empty());
        let choices = provider_descriptors(&[KIND_OPENAI_COMPATIBLE]);
        assert_eq!(
            choices.iter().map(|choice| choice.id).collect::<Vec<_>>(),
            ["glm", "openai-compatible"]
        );
        assert!(
            choices
                .iter()
                .all(|choice| choice.adapter == KIND_OPENAI_COMPATIBLE)
        );
    }

    #[test]
    fn glm_quick_start_is_complete_and_versioned() {
        let glm = provider_descriptors(&[KIND_OPENAI_COMPATIBLE])
            .into_iter()
            .find(|choice| choice.id == "glm")
            .expect("the GLM descriptor");
        assert_eq!(glm.provider, Some("zai"));
        assert_eq!(glm.profile, Some("glm"));
        assert_eq!(glm.endpoint, Some("https://api.z.ai/api/coding/paas/v4"));
        assert_eq!(glm.reasoning_only, Some(ReasoningOnlyBehavior::Text));
        assert_eq!(glm.models, &[GLM_4_7]);
        assert_eq!(GLM_4_7.catalog, TRUSTED_MODEL_CATALOG_NAME);
        assert_eq!(GLM_4_7.revision, TRUSTED_MODEL_CATALOG_REVISION);
        assert_eq!(
            (
                GLM_4_7.context_tokens,
                GLM_4_7.max_input_tokens,
                GLM_4_7.max_output_tokens,
                GLM_4_7.request_output_tokens,
                GLM_4_7.output_reserve,
            ),
            (200_000, 196_000, 131_072, 8_192, 8_192)
        );
    }

    #[test]
    fn trusted_records_are_provider_scoped() {
        assert_eq!(trusted_model("zai", "glm-4.7"), Some(&GLM_4_7));
        assert_eq!(trusted_model("other", "glm-4.7"), None);
    }
}
