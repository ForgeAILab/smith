//! Frozen, credential-free provider model metadata shared by inventory and runtime.
//!
//! The host owns loading and refreshing this data. Configuration and picker
//! code receive only an immutable value, which keeps enumeration pure and
//! prevents a model choice from changing underneath runtime preflight.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::model::{
    KIND_GEMINI_INTERACTIONS, KIND_OPENAI_COMPATIBLE, KIND_OPENAI_RESPONSES, KIND_XAI_RESPONSES,
};

/// The only public origin Smith accepts provider model metadata from.
pub const MODELS_DEV_SOURCE_URL: &str = "https://models.dev/api.json";

/// The normalized cache/seed schema revision.
///
/// Bumped from `1` to `2` when `CatalogModel::cost` was added: a cached
/// snapshot written before prices existed carries no `cost` field, and
/// letting it load unchanged would silently under-report every model as
/// unpriced. The existing revision check rejects it as stale, so it falls
/// back to the embedded seed and schedules a refresh, exactly as it does for
/// any other stale revision.
pub const CATALOG_SCHEMA_REVISION: u32 = 2;

/// Models.dev's OpenAI provider identity.
pub const OPENAI_CATALOG_PROVIDER: &str = "openai";

/// Models.dev's OpenRouter provider identity.
pub const OPENROUTER_CATALOG_PROVIDER: &str = "openrouter";

/// Models.dev's Z.AI Coding Plan provider identity.
pub const ZAI_CODING_PLAN_CATALOG_PROVIDER: &str = "zai-coding-plan";

/// Models.dev's xAI provider identity.
pub const XAI_CATALOG_PROVIDER: &str = "xai";

/// Models.dev's Google provider identity.
pub const GOOGLE_CATALOG_PROVIDER: &str = "google";

/// The exact normalized OpenAI endpoint Smith binds to its catalog.
pub const OPENAI_ENDPOINT: &str = "https://api.openai.com/v1";

/// The exact normalized OpenRouter endpoint Smith binds to its catalog.
pub const OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/v1";

/// The exact normalized xAI endpoint Smith binds to its catalog.
pub const XAI_CATALOG_ENDPOINT: &str = "https://api.x.ai/v1";

/// The fixed API-version base URL for native Gemini Interactions.
pub const GEMINI_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta";

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
    /// Source-advertised reasoning controls, absent when only presence is
    /// known. Never present when `reasoning` is `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_controls: Option<CatalogReasoningControls>,
    /// Whether the source advertises schema-constrained output.
    pub structured_output: bool,
    /// Per-counter Models.dev price, absent when the source published no
    /// individually valid counter. A price is purely additive: its absence,
    /// or the absence of any one counter within it, is never a reason this
    /// model is disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<CatalogModelCost>,
    /// A bounded validation reason that keeps an advertised entry visible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

/// Normalized reasoning controls one catalog source advertises for one model.
///
/// Only control shapes Smith can express survive normalization: an on/off
/// switch and an ordered effort ladder. Token-budget options are dropped
/// until a budget control exists end to end.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogReasoningControls {
    /// Whether the source advertises an explicit on/off switch.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub toggle: bool,
    /// Source-advertised effort names in advertised order, lowercased.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
}

impl CatalogModel {
    /// Whether the model produces text.
    pub fn has_text_output(&self) -> bool {
        self.output_modalities.contains(&CatalogModality::Text)
    }
}

/// A catalog model's per-counter price, in USD per million tokens.
///
/// Stored as micro-USD (1e-6 USD) per million tokens, in a fixed-point `u64`,
/// rather than as `f64`. `CatalogModel` and `CatalogSnapshot` both derive
/// `Eq` so a snapshot can be compared and content-digested exactly; `f64` has
/// no total equality under IEEE 754 (`NaN != NaN`, and `-0.0 == 0.0` despite
/// differing bit patterns), so an `f64` field would force `CatalogModel` to
/// drop `Eq` or to hand-roll a comparison that quietly disagrees with what
/// gets serialized. A micro-USD integer keeps `Eq` for free, round-trips
/// through serde byte-for-byte with no float re-parsing drift feeding into
/// `content_digest`, and losslessly represents the up-to-six-decimal-place
/// granularity Models.dev actually publishes (for example `0.083333`
/// USD/million tokens for a cache write).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogModelCost {
    /// Input token price, in micro-USD per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    /// Output token price, in micro-USD per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
    /// Cache-read token price, in micro-USD per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    /// Cache-write token price, in micro-USD per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<u64>,
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
    // Native Gemini owns one fixed endpoint and intentionally omits
    // `base_url` from ordinary user configuration.
    if kind == KIND_GEMINI_INTERACTIONS && base_url.is_none() {
        return Some(GOOGLE_CATALOG_PROVIDER);
    }
    let endpoint = normalized_endpoint(base_url?)?;
    // The pairing is exact on both sides. A catalog entry describes a model as
    // one deployment serves it, so inheriting limits from a matching name at a
    // different endpoint, or over a different protocol, would be a guess.
    match (kind, endpoint.as_str()) {
        (KIND_OPENAI_COMPATIBLE, OPENAI_ENDPOINT) => Some(OPENAI_CATALOG_PROVIDER),
        (KIND_OPENAI_COMPATIBLE, OPENROUTER_ENDPOINT) => Some(OPENROUTER_CATALOG_PROVIDER),
        (KIND_OPENAI_COMPATIBLE, ZAI_CODING_PLAN_ENDPOINT) => {
            Some(ZAI_CODING_PLAN_CATALOG_PROVIDER)
        }
        // Same deployment either way; the kinds differ only in how the
        // request is authenticated, which the catalog has no opinion about.
        (KIND_OPENAI_RESPONSES | KIND_XAI_RESPONSES, XAI_CATALOG_ENDPOINT) => {
            Some(XAI_CATALOG_PROVIDER)
        }
        (KIND_GEMINI_INTERACTIONS, GEMINI_ENDPOINT) => Some(GOOGLE_CATALOG_PROVIDER),
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

    fn priced_model() -> CatalogModel {
        CatalogModel {
            id: "priced-model".to_owned(),
            name: "Priced Model".to_owned(),
            limits: None,
            input_modalities: Vec::new(),
            output_modalities: Vec::new(),
            tool_call: true,
            reasoning: false,
            reasoning_controls: None,
            structured_output: false,
            cost: Some(CatalogModelCost {
                input: Some(5_000_000),
                output: Some(30_000_000),
                cache_read: Some(1_250_000),
                cache_write: Some(83_333),
            }),
            disabled_reason: None,
        }
    }

    #[test]
    fn priced_model_round_trips_and_serializes_byte_stably() {
        let model = priced_model();
        let first = serde_json::to_string(&model).expect("serialize");
        let second = serde_json::to_string(&model).expect("serialize again");
        assert_eq!(first, second, "serialization must be byte-stable");

        let decoded: CatalogModel = serde_json::from_str(&first).expect("deserialize");
        assert_eq!(decoded, model, "round trip must be exact under Eq");

        let cost_json = serde_json::to_value(model.cost.unwrap()).unwrap();
        assert_eq!(
            cost_json,
            serde_json::json!({
                "input": 5_000_000,
                "output": 30_000_000,
                "cache_read": 1_250_000,
                "cache_write": 83_333,
            }),
            "each counter round-trips as a plain JSON integer, not a float"
        );
    }

    #[test]
    fn absent_cost_is_not_serialized() {
        let model = CatalogModel {
            cost: None,
            ..priced_model()
        };
        let value = serde_json::to_value(&model).unwrap();
        assert!(
            value.as_object().unwrap().get("cost").is_none(),
            "an absent price must not appear in the serialized model at all"
        );
    }

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
            catalog_provider_for(KIND_OPENAI_COMPATIBLE, Some("https://api.openai.com/v1/")),
            Some(OPENAI_CATALOG_PROVIDER)
        );
        assert_eq!(
            catalog_provider_for(KIND_GEMINI_INTERACTIONS, None),
            Some(GOOGLE_CATALOG_PROVIDER)
        );
        assert_eq!(
            catalog_provider_for(KIND_GEMINI_INTERACTIONS, Some(GEMINI_ENDPOINT)),
            Some(GOOGLE_CATALOG_PROVIDER)
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
        assert_eq!(
            catalog_provider_for(KIND_OPENAI_COMPATIBLE, Some(GEMINI_ENDPOINT)),
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
