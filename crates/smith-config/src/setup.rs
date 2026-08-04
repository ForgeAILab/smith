//! Smith-owned data used by guided provider and model setup.
//!
//! Descriptors are deliberately static and local. Building this catalog opens
//! no credential service and performs no provider or network I/O. The caller
//! supplies the adapter kinds compiled into its pinned runtime, so a preset can
//! never be offered by silently substituting another wire protocol.

use std::collections::BTreeSet;

use crate::model::{
    ANTHROPIC_DEFAULT_ENDPOINT, KIND_ANTHROPIC_MESSAGES, KIND_CHATGPT_RESPONSES,
    KIND_GEMINI_INTERACTIONS, KIND_OPENAI_COMPATIBLE, ReasoningOnlyBehavior,
};

/// Revision of the trusted model data shipped with this Smith build.
pub const TRUSTED_MODEL_CATALOG_REVISION: u32 = 3;

/// Stable name recorded for Smith's built-in setup model data.
pub const TRUSTED_MODEL_CATALOG_NAME: &str = "smith-trusted-models";
/// Provider name used by the GLM quick start.
pub const GLM_PROVIDER: &str = "zai";
/// Default profile created by the GLM quick start.
pub const GLM_PROFILE: &str = "glm";
/// Z.AI Coding Plan OpenAI-compatible endpoint.
pub const GLM_ENDPOINT: &str = "https://api.z.ai/api/coding/paas/v4";
/// Provider name used by the built-in OpenRouter connection descriptor.
pub const OPENROUTER_PROVIDER: &str = "openrouter";
/// Fixed OpenRouter OpenAI-compatible API endpoint.
pub const OPENROUTER_ENDPOINT: &str = crate::catalog::OPENROUTER_ENDPOINT;
/// Provider name used by Smith's experimental ChatGPT integration.
pub const CHATGPT_PROVIDER: &str = "chatgpt";
/// Fixed base URL for the experimental direct ChatGPT Codex Responses API.
pub const CHATGPT_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex";
/// Fixed owner-only plaintext auth-file reference for Smith's renewable ChatGPT bundle.
pub const CHATGPT_CREDENTIAL: &str = "authfile:chatgpt";

/// Provider name for an xAI subscription reached by browser login.
pub const XAI_PROVIDER: &str = "xai";
/// xAI's Responses API base URL.
pub const XAI_ENDPOINT: &str = "https://api.x.ai/v1";
/// Owner-only auth-file reference for Smith's renewable xAI bundle.
pub const XAI_CREDENTIAL: &str = "authfile:xai";

/// Provider name used by the native Gemini connection.
pub const GOOGLE_PROVIDER: &str = "google";
/// Default profile created by the native Gemini connection.
pub const GOOGLE_PROFILE: &str = "gemini";
/// Native Gemini Interactions API-version base URL.
pub const GOOGLE_ENDPOINT: &str = crate::catalog::GEMINI_ENDPOINT;

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

/// GLM-5.2 metadata verified for the Z.AI Coding Plan endpoint.
pub const GLM_5_2: TrustedModelRecord = TrustedModelRecord {
    provider: GLM_PROVIDER,
    model: "glm-5.2",
    label: "GLM-5.2",
    catalog: TRUSTED_MODEL_CATALOG_NAME,
    revision: TRUSTED_MODEL_CATALOG_REVISION,
    context_tokens: 1_000_000,
    max_input_tokens: 1_000_000,
    max_output_tokens: 131_072,
    request_output_tokens: 32_768,
    output_reserve: 32_768,
};

/// First reviewed model binding for Smith's experimental direct ChatGPT path.
///
/// The public Codex catalog publishes the 272k context window. Its output
/// ceiling is not part of that public record, so Smith deliberately enforces a
/// conservative 16k product cap instead of treating the remainder as output.
pub const CHATGPT_TERRA: TrustedModelRecord = TrustedModelRecord {
    provider: CHATGPT_PROVIDER,
    model: "gpt-5.6-terra",
    label: "GPT-5.6 Terra (experimental ChatGPT)",
    catalog: TRUSTED_MODEL_CATALOG_NAME,
    revision: TRUSTED_MODEL_CATALOG_REVISION,
    context_tokens: 272_000,
    max_input_tokens: 255_616,
    max_output_tokens: 16_384,
    request_output_tokens: 16_384,
    output_reserve: 16_384,
};

const GLM_MODELS: &[TrustedModelRecord] = &[GLM_5_2, GLM_4_7];
const CHATGPT_MODELS: &[TrustedModelRecord] = &[CHATGPT_TERRA];

const DESCRIPTORS: &[ProviderSetupDescriptor] = &[
    ProviderSetupDescriptor {
        id: "glm",
        label: "Quick start with GLM",
        description: "Z.AI Coding Plan endpoint with trusted GLM-5.2 limits",
        provider: Some(GLM_PROVIDER),
        profile: Some(GLM_PROFILE),
        adapter: KIND_OPENAI_COMPATIBLE,
        endpoint: Some(GLM_ENDPOINT),
        credentials: CREDENTIAL_METHODS,
        models: GLM_MODELS,
        reasoning_only: Some(ReasoningOnlyBehavior::Text),
    },
    ProviderSetupDescriptor {
        id: "openrouter",
        label: "Connect OpenRouter",
        description: "OpenRouter API key with a fixed endpoint and reviewed model limits",
        provider: Some(OPENROUTER_PROVIDER),
        profile: None,
        adapter: KIND_OPENAI_COMPATIBLE,
        endpoint: Some(OPENROUTER_ENDPOINT),
        credentials: CREDENTIAL_METHODS,
        models: &[],
        reasoning_only: None,
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
    ProviderSetupDescriptor {
        id: "anthropic-messages",
        label: "Anthropic Messages API",
        description: "Native Claude endpoint with images and thinking; enter a model ID and limits",
        provider: None,
        profile: None,
        adapter: KIND_ANTHROPIC_MESSAGES,
        endpoint: Some(ANTHROPIC_DEFAULT_ENDPOINT),
        credentials: CREDENTIAL_METHODS,
        models: &[],
        reasoning_only: None,
    },
    ProviderSetupDescriptor {
        id: "chatgpt",
        label: "Connect ChatGPT (experimental)",
        description: "Smith OAuth and direct Responses calls; unsupported public API boundary",
        provider: Some(CHATGPT_PROVIDER),
        profile: None,
        adapter: KIND_CHATGPT_RESPONSES,
        endpoint: Some(CHATGPT_ENDPOINT),
        credentials: &[],
        models: CHATGPT_MODELS,
        reasoning_only: None,
    },
    ProviderSetupDescriptor {
        id: "google",
        label: "Connect Google Gemini",
        description: "AI Studio API key with a fixed native Gemini Interactions endpoint",
        provider: Some(GOOGLE_PROVIDER),
        profile: Some(GOOGLE_PROFILE),
        adapter: KIND_GEMINI_INTERACTIONS,
        endpoint: Some(GOOGLE_ENDPOINT),
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
            ["glm", "openrouter", "openai-compatible"]
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
        assert_eq!(glm.models, &[GLM_5_2, GLM_4_7]);
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
        assert_eq!(GLM_5_2.catalog, TRUSTED_MODEL_CATALOG_NAME);
        assert_eq!(GLM_5_2.revision, TRUSTED_MODEL_CATALOG_REVISION);
        assert_eq!(
            (
                GLM_5_2.context_tokens,
                GLM_5_2.max_input_tokens,
                GLM_5_2.max_output_tokens,
                GLM_5_2.request_output_tokens,
                GLM_5_2.output_reserve,
            ),
            (1_000_000, 1_000_000, 131_072, 32_768, 32_768)
        );
    }

    #[test]
    fn openrouter_descriptor_fixes_the_reviewed_endpoint() {
        let descriptor = provider_descriptors(&[KIND_OPENAI_COMPATIBLE])
            .into_iter()
            .find(|choice| choice.id == "openrouter")
            .expect("OpenRouter descriptor");
        assert_eq!(descriptor.provider, Some(OPENROUTER_PROVIDER));
        assert_eq!(descriptor.endpoint, Some(OPENROUTER_ENDPOINT));
        assert_eq!(descriptor.adapter, KIND_OPENAI_COMPATIBLE);
        assert!(descriptor.models.is_empty());
    }

    #[test]
    fn chatgpt_descriptor_is_fixed_experimental_and_versioned() {
        let descriptor = provider_descriptors(&[KIND_CHATGPT_RESPONSES])
            .into_iter()
            .find(|choice| choice.id == CHATGPT_PROVIDER)
            .expect("ChatGPT descriptor");
        assert_eq!(descriptor.provider, Some(CHATGPT_PROVIDER));
        assert_eq!(descriptor.endpoint, Some(CHATGPT_ENDPOINT));
        assert_eq!(descriptor.adapter, KIND_CHATGPT_RESPONSES);
        assert!(descriptor.credentials.is_empty());
        assert!(descriptor.description.contains("unsupported"));
        assert_eq!(descriptor.models, &[CHATGPT_TERRA]);
        assert_eq!(
            (
                CHATGPT_TERRA.context_tokens,
                CHATGPT_TERRA.max_input_tokens,
                CHATGPT_TERRA.max_output_tokens,
                CHATGPT_TERRA.request_output_tokens,
            ),
            (272_000, 255_616, 16_384, 16_384)
        );
    }

    #[test]
    fn google_descriptor_uses_native_catalog_backed_setup() {
        let descriptor = provider_descriptors(&[KIND_GEMINI_INTERACTIONS])
            .into_iter()
            .find(|choice| choice.id == GOOGLE_PROVIDER)
            .expect("Google descriptor");
        assert_eq!(descriptor.provider, Some(GOOGLE_PROVIDER));
        assert_eq!(descriptor.profile, Some(GOOGLE_PROFILE));
        assert_eq!(descriptor.endpoint, Some(GOOGLE_ENDPOINT));
        assert_eq!(descriptor.adapter, KIND_GEMINI_INTERACTIONS);
        assert!(descriptor.models.is_empty());
    }

    #[test]
    fn trusted_records_are_provider_scoped() {
        assert_eq!(trusted_model("zai", "glm-4.7"), Some(&GLM_4_7));
        assert_eq!(trusted_model("zai", "glm-5.2"), Some(&GLM_5_2));
        assert_eq!(
            trusted_model(CHATGPT_PROVIDER, CHATGPT_TERRA.model),
            Some(&CHATGPT_TERRA)
        );
        assert_eq!(trusted_model("other", "glm-4.7"), None);
    }
}
