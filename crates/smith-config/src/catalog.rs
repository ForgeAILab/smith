//! Frozen, credential-free provider model metadata shared by inventory and runtime.
//!
//! The host owns loading and refreshing this data. Configuration and picker
//! code receive only an immutable value, which keeps enumeration pure and
//! prevents a model choice from changing underneath runtime preflight.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::model::KIND_OPENAI_COMPATIBLE;

/// The only public origin Smith accepts provider model metadata from.
pub const MODELS_DEV_SOURCE_URL: &str = "https://models.dev/api.json";

/// The normalized cache/seed schema revision.
pub const CATALOG_SCHEMA_REVISION: u32 = 1;

/// Models.dev's OpenRouter provider identity.
pub const OPENROUTER_CATALOG_PROVIDER: &str = "openrouter";

/// Models.dev's Z.AI Coding Plan provider identity.
pub const ZAI_CODING_PLAN_CATALOG_PROVIDER: &str = "zai-coding-plan";

/// The exact normalized OpenRouter endpoint Smith binds to its catalog.
pub const OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/v1";

/// The exact normalized Z.AI Coding Plan endpoint Smith binds to its catalog.
pub const ZAI_CODING_PLAN_ENDPOINT: &str = "https://api.z.ai/api/coding/paas/v4";

/// An immutable, normalized Models.dev snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogSnapshot {
    /// Smith's normalized snapshot schema.
    pub schema_revision: u32,
    /// Exact public origin used to produce the snapshot.
    pub source_url: String,
    /// SHA-256 digest of the source response.
    pub source_digest: String,
    /// SHA-256 digest of the canonical normalized provider payload.
    pub content_digest: String,
    /// Origin revision such as an ETag, or the source digest when absent.
    pub source_revision: String,
    /// Retrieval time in Unix milliseconds.
    pub retrieved_at_ms: u64,
    /// Only the provider catalogs Smith explicitly supports.
    pub providers: BTreeMap<String, CatalogProvider>,
}

impl CatalogSnapshot {
    /// Returns one supported provider catalog.
    pub fn provider(&self, id: &str) -> Option<&CatalogProvider> {
        self.providers.get(id)
    }

    /// Returns one model from a supported provider catalog.
    pub fn model(&self, provider: &str, model: &str) -> Option<&CatalogModel> {
        self.provider(provider)?.models.get(model)
    }

    /// Stable source revision displayed in provenance.
    pub fn revision(&self) -> &str {
        &self.source_revision
    }
}

/// One provider catalog after Smith-owned normalization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogProvider {
    /// Models.dev provider identity.
    pub id: String,
    /// Human-readable provider name.
    pub name: String,
    /// Provider model IDs in deterministic order.
    pub models: BTreeMap<String, CatalogModel>,
}

/// One advertised model after schema and numeric validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogModel {
    /// Provider-owned model ID, which may contain nested slashes.
    pub id: String,
    /// Human-readable model name.
    pub name: String,
    /// Enforceable limits, absent when the source entry's limits were invalid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<CatalogLimits>,
    /// Accepted input modalities Smith understands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modalities: Vec<CatalogModality>,
    /// Produced output modalities Smith understands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<CatalogModality>,
    /// Whether the source advertises tool/function calling.
    pub tool_call: bool,
    /// Whether the source advertises model reasoning.
    pub reasoning: bool,
    /// Whether the source advertises schema-constrained output.
    pub structured_output: bool,
    /// A bounded validation reason that keeps an advertised entry visible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

impl CatalogModel {
    /// Whether the model produces text.
    pub fn has_text_output(&self) -> bool {
        self.output_modalities.contains(&CatalogModality::Text)
    }
}

/// The three limits Agent Runtime requires before provider I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogLimits {
    /// Total input-plus-output context window.
    pub context_tokens: u32,
    /// Provider-published input ceiling, or the context window when absent.
    pub max_input_tokens: u32,
    /// Provider-published output ceiling.
    pub max_output_tokens: u32,
}

/// A content modality Smith can safely pass into Agent Runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogModality {
    /// Plain text.
    Text,
    /// Raster images.
    Image,
    /// Audio.
    Audio,
    /// Video.
    Video,
    /// Documents such as PDF files.
    Document,
}

/// Returns the supported Models.dev provider for an exact normalized binding.
///
/// The local provider name is deliberately absent: aliases remain local, and
/// a provider with a familiar name but a different endpoint inherits nothing.
pub fn catalog_provider_for(kind: &str, base_url: Option<&str>) -> Option<&'static str> {
    if kind != KIND_OPENAI_COMPATIBLE {
        return None;
    }
    match normalized_endpoint(base_url?)?.as_str() {
        OPENROUTER_ENDPOINT => Some(OPENROUTER_CATALOG_PROVIDER),
        ZAI_CODING_PLAN_ENDPOINT => Some(ZAI_CODING_PLAN_CATALOG_PROVIDER),
        _ => None,
    }
}

fn normalized_endpoint(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let host = url.host_str()?;
    let mut endpoint = format!("{}://{host}", url.scheme());
    if let Some(port) = url.port() {
        endpoint.push(':');
        endpoint.push_str(&port.to_string());
    }
    endpoint.push_str(url.path().trim_end_matches('/'));
    Some(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_uses_adapter_and_exact_normalized_endpoint() {
        assert_eq!(
            catalog_provider_for(
                KIND_OPENAI_COMPATIBLE,
                Some("https://OPENROUTER.ai/api/v1/")
            ),
            Some(OPENROUTER_CATALOG_PROVIDER)
        );
        assert_eq!(
            catalog_provider_for(
                KIND_OPENAI_COMPATIBLE,
                Some("https://api.z.ai/api/coding/paas/v4")
            ),
            Some(ZAI_CODING_PLAN_CATALOG_PROVIDER)
        );
        assert_eq!(
            catalog_provider_for(
                KIND_OPENAI_COMPATIBLE,
                Some("https://openrouter.ai/api/v1/other")
            ),
            None
        );
        assert_eq!(
            catalog_provider_for("fake", Some(OPENROUTER_ENDPOINT)),
            None
        );
    }

    #[test]
    fn binding_refuses_url_credentials_and_options() {
        assert_eq!(
            catalog_provider_for(
                KIND_OPENAI_COMPATIBLE,
                Some("https://key@openrouter.ai/api/v1")
            ),
            None
        );
        assert_eq!(
            catalog_provider_for(
                KIND_OPENAI_COMPATIBLE,
                Some("https://openrouter.ai/api/v1?key=secret")
            ),
            None
        );
    }
}
