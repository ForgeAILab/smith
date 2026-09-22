//! Bounded best-effort reads of an OpenAI-compatible `/models` listing.
//!
//! Custom providers usually know their own limits and publish them on the
//! listing endpoint every OpenAI-compatible server exposes. Setup uses one
//! bounded GET to prefill reviewable values — never an inference request —
//! and treats every failure as "nothing resolved": a probe is a proposal, not
//! a prerequisite.

use std::time::Duration;

use serde_json::Value;

/// Wall-clock bound for one probe, covering connect and body.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// A listing larger than this is not a model listing Smith wants to trust.
const MAX_LISTING_BYTES: usize = 1024 * 1024;

/// Limit fields one listing entry advertised, each independently absent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenAiModelLimits {
    /// Total context window.
    pub context_tokens: Option<u32>,
    /// Enforced input ceiling.
    pub max_input_tokens: Option<u32>,
    /// Enforced output ceiling.
    pub max_output_tokens: Option<u32>,
}

impl OpenAiModelLimits {
    /// Whether the entry advertised nothing Smith understands.
    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

/// The output-ceiling default when a source published only a context window.
///
/// Mirrors the automatic request budget's percentage rule (`min(cap,
/// context/4)`), so a prefilled ceiling never advertises more room than Smith
/// would request anyway. Shared by setup prefill and the manual-entry default
/// so the two cannot drift apart.
pub fn derived_output_ceiling(context_tokens: u32) -> u32 {
    smith_config::output_budget::AUTOMATIC_REQUEST_OUTPUT_TOKEN_CAP
        .min(context_tokens / 4)
        .max(1)
}

///
/// Returns `Ok(None)` when the listing parses but carries no entry for the
/// model. Every transport, status, size, or parse problem is an `Err` the
/// caller treats as unresolved; the message never echoes the bearer.
pub async fn openai_compatible_model_limits(
    endpoint: &str,
    bearer: Option<&str>,
    model: &str,
) -> Result<Option<OpenAiModelLimits>, String> {
    let url = format!("{}/models", endpoint.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .connect_timeout(PROBE_TIMEOUT)
        .timeout(PROBE_TIMEOUT)
        .user_agent(concat!("smith/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("building the model-listing client failed: {error}"))?;
    let mut request = client.get(&url);
    if let Some(bearer) = bearer {
        request = request.bearer_auth(bearer);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("reading the model listing failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "the model listing answered {}",
            response.status().as_u16()
        ));
    }
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("reading the model listing failed: {error}"))?;
    if body.len() > MAX_LISTING_BYTES {
        return Err("the model listing exceeded Smith's size bound".to_owned());
    }
    let listing: Value = serde_json::from_slice(&body)
        .map_err(|error| format!("the model listing was not JSON: {error}"))?;
    Ok(find_model_limits(&listing, model))
}

/// Finds `model`'s entry in one parsed listing and reads its limit fields.
///
/// The entry whose `id` equals the typed model is the only honest match: that
/// ID is what requests will send. Field names follow what real gateways
/// publish — OpenRouter's `context_length` (plus its nested
/// `top_provider.context_length`), Groq's `context_window`, vLLM's
/// `max_model_len`, LiteLLM's `max_input_tokens`/`max_output_tokens`, and the
/// OpenAI-style completion aliases.
pub fn find_model_limits(listing: &Value, model: &str) -> Option<OpenAiModelLimits> {
    let entry = listing
        .get("data")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(model))?;
    Some(limits_from_models_entry(entry))
}

/// Reads every limit field Smith understands from one listing entry.
pub fn limits_from_models_entry(entry: &Value) -> OpenAiModelLimits {
    let context = [
        "context_length",
        "context_window",
        "max_model_len",
        "max_context_tokens",
    ];
    let input = ["max_input_tokens", "max_prompt_tokens"];
    let output = ["max_output_tokens", "max_completion_tokens", "max_tokens"];
    let nested = entry.get("top_provider");
    let read = |source: &Value, fields: &[&str]| {
        fields
            .iter()
            .find_map(|field| positive_u32(source.get(*field)))
    };
    OpenAiModelLimits {
        context_tokens: read(entry, &context).or_else(|| nested.and_then(|n| read(n, &context))),
        max_input_tokens: read(entry, &input).or_else(|| nested.and_then(|n| read(n, &input))),
        max_output_tokens: read(entry, &output).or_else(|| nested.and_then(|n| read(n, &output))),
    }
}

/// Accepts only a positive integer that fits `u32`.
fn positive_u32(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .and_then(|tokens| u32::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_entry_is_matched_only_by_its_exact_id() {
        let listing = json!({ "data": [ { "id": "llama-3.3-70b", "context_window": 131072 } ] });
        assert_eq!(
            find_model_limits(&listing, "llama-3.3-70b"),
            Some(OpenAiModelLimits {
                context_tokens: Some(131072),
                max_input_tokens: None,
                max_output_tokens: None,
            })
        );
        assert_eq!(find_model_limits(&listing, "Llama-3.3-70B"), None);
    }

    #[test]
    fn openrouter_style_fields_are_read_including_the_nested_provider_ceiling() {
        let listing = json!({ "data": [ {
            "id": "anthropic/claude-sonnet-4.5",
            "context_length": 200000,
            "top_provider": { "context_length": 1000000, "max_completion_tokens": 64000 }
        } ] });
        // The top-level advertisement is the tier every routing honors; the
        // nested provider ceiling can only narrow it, so it stays a fallback.
        assert_eq!(
            find_model_limits(&listing, "anthropic/claude-sonnet-4.5"),
            Some(OpenAiModelLimits {
                context_tokens: Some(200000),
                max_input_tokens: None,
                max_output_tokens: Some(64000),
            })
        );
    }

    #[test]
    fn litellm_and_vllm_style_fields_are_read() {
        let litellm = json!({ "data": [ {
            "id": "gpt-proxy",
            "max_input_tokens": 60000,
            "max_output_tokens": 4000,
            "max_model_len": 64000
        } ] });
        assert_eq!(
            find_model_limits(&litellm, "gpt-proxy"),
            Some(OpenAiModelLimits {
                context_tokens: Some(64000),
                max_input_tokens: Some(60000),
                max_output_tokens: Some(4000),
            })
        );
        let vllm = json!({ "data": [ { "id": "Qwen/Qwen2.5-Coder-32B-Instruct" } ] });
        assert_eq!(
            find_model_limits(&vllm, "Qwen/Qwen2.5-Coder-32B-Instruct"),
            Some(OpenAiModelLimits::default())
        );
    }

    #[test]
    fn zero_negative_and_non_numeric_fields_are_ignored() {
        let listing = json!({ "data": [ {
            "id": "broken",
            "context_length": 0,
            "max_output_tokens": -5,
            "max_tokens": "8000"
        } ] });
        assert_eq!(
            find_model_limits(&listing, "broken"),
            Some(OpenAiModelLimits::default())
        );
        assert!(limits_from_models_entry(&json!({ "id": "x" })).is_empty());
    }

    #[test]
    fn listings_without_a_data_array_resolve_nothing() {
        assert_eq!(find_model_limits(&json!({ "object": "list" }), "m"), None);
        assert_eq!(find_model_limits(&json!(null), "m"), None);
    }
}
