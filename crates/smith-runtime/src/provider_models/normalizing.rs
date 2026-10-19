use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use smith_config::catalog::{
    CatalogLimits, CatalogModality, CatalogModel, CatalogModelCost, CatalogProvider,
    CatalogReasoningControls, PartialCatalogLimits,
};

use super::MAX_MODELS;

const MAX_ID_BYTES: usize = 512;
const MAX_NAME_BYTES: usize = 1024;

pub(super) fn valid_text(text: &str, maximum: usize) -> bool {
    !text.trim().is_empty() && text.len() <= maximum && !text.chars().any(char::is_control)
}

pub(super) fn validate_listing(listing: &Value) -> Result<(), String> {
    let entries = listing
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "the model listing has no data array".to_owned())?;
    if entries.len() > MAX_MODELS {
        return Err("the model listing exceeded its 10,000-model count bound".to_owned());
    }
    let mut ids = BTreeSet::new();
    for entry in entries {
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| valid_text(id, MAX_ID_BYTES))
            .ok_or_else(|| {
                "the model listing contains an invalid or oversized model ID".to_owned()
            })?;
        if !ids.insert(id) {
            return Err("the model listing contains duplicate model IDs".to_owned());
        }
        if entry
            .get("name")
            .filter(|name| !name.is_null())
            .is_some_and(|name| {
                !name
                    .as_str()
                    .is_some_and(|name| valid_text(name, MAX_NAME_BYTES))
            })
        {
            return Err("the model listing contains an invalid or oversized model name".to_owned());
        }
    }
    Ok(())
}

pub(super) fn normalize_listing(
    listing: &Value,
    provider: &str,
) -> Result<CatalogProvider, String> {
    validate_listing(listing)?;
    let mut models = BTreeMap::new();
    for raw in listing["data"].as_array().expect("validated listing") {
        let id = raw["id"].as_str().expect("validated id").to_owned();
        let limits = crate::probe::limits_from_models_entry(raw);
        let has_invalid = invalid_limit(raw);
        let complete = match (limits.context_tokens, limits.max_output_tokens) {
            (Some(context), Some(output))
                if !has_invalid
                    && output <= context
                    && limits.max_input_tokens.is_none_or(|input| input <= context) =>
            {
                Some(CatalogLimits {
                    context_tokens: context,
                    max_input_tokens: limits.max_input_tokens.unwrap_or(context),
                    max_output_tokens: output,
                })
            }
            _ => None,
        };
        let disabled_reason = complete.is_none().then(|| {
            if has_invalid {
                "endpoint model has a zero, malformed, or out-of-range limit"
            } else if limits
                .context_tokens
                .zip(limits.max_output_tokens)
                .is_some_and(|(context, output)| output > context)
                || limits
                    .context_tokens
                    .zip(limits.max_input_tokens)
                    .is_some_and(|(context, input)| input > context)
            {
                "endpoint model input or output limit exceeds its context window"
            } else {
                "endpoint model has no complete context/output limit declaration"
            }
            .to_owned()
        });
        let reasoning = advertised_bool(raw, &["reasoning", "supports_reasoning"])
            .unwrap_or_else(|| supports_parameter(raw, "reasoning_effort"));
        let model = CatalogModel {
            id: id.clone(),
            name: raw
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_owned(),
            limits: complete,
            input_modalities: modalities(raw, "input"),
            output_modalities: modalities(raw, "output"),
            tool_call: advertised_tools(raw).unwrap_or(false),
            reasoning,
            reasoning_controls: reasoning.then(|| reasoning_controls(raw)).flatten(),
            structured_output: advertised_bool(raw, &["structured_output", "structured_outputs"])
                .unwrap_or_else(|| supports_parameter(raw, "structured_outputs")),
            cost: normalize_cost(raw),
            disabled_reason,
        };
        models.insert(id, model);
    }
    Ok(CatalogProvider {
        id: provider.to_owned(),
        name: provider.to_owned(),
        models,
    })
}

pub(super) fn partial_limits(
    listing: &Value,
    catalog: &CatalogProvider,
) -> BTreeMap<String, PartialCatalogLimits> {
    listing["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|raw| {
            let id = raw.get("id")?.as_str()?;
            if catalog.models.get(id)?.limits.is_some() {
                return None;
            }
            let limits = crate::probe::limits_from_models_entry(raw);
            if limits.is_empty() {
                return None;
            }
            Some((
                id.to_owned(),
                PartialCatalogLimits {
                    context_tokens: limits.context_tokens,
                    max_input_tokens: limits.max_input_tokens,
                    max_output_tokens: limits.max_output_tokens,
                },
            ))
        })
        .collect()
}

pub(super) fn valid_partial_limits(
    partial: &BTreeMap<String, PartialCatalogLimits>,
    catalog: &CatalogProvider,
) -> bool {
    partial.len() <= MAX_MODELS
        && partial.iter().all(|(id, limits)| {
            catalog
                .models
                .get(id)
                .is_some_and(|model| model.limits.is_none())
                && [
                    limits.context_tokens,
                    limits.max_input_tokens,
                    limits.max_output_tokens,
                ]
                .into_iter()
                .flatten()
                .all(|limit| limit > 0)
                && [
                    limits.context_tokens,
                    limits.max_input_tokens,
                    limits.max_output_tokens,
                ]
                .into_iter()
                .any(|limit| limit.is_some())
        })
}

fn invalid_limit(raw: &Value) -> bool {
    let top = [
        "context_length",
        "context_window",
        "max_model_len",
        "max_context_tokens",
        "max_input_tokens",
        "max_prompt_tokens",
        "max_output_tokens",
        "max_completion_tokens",
        "max_tokens",
    ];
    let invalid = |source: &Value, names: &[&str]| {
        names.iter().any(|name| {
            source
                .get(*name)
                .filter(|value| !value.is_null())
                .is_some_and(|value| {
                    !value
                        .as_u64()
                        .is_some_and(|tokens| tokens > 0 && u32::try_from(tokens).is_ok())
                })
        })
    };
    invalid(raw, &top)
        || raw
            .get("top_provider")
            .is_some_and(|source| invalid(source, &top))
        || raw
            .get("limits")
            .is_some_and(|source| invalid(source, &["context", "input", "output"]))
}

pub(super) fn unadvertised_capabilities(listing: &Value) -> BTreeSet<String> {
    listing["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|raw| {
            let tools = advertised_tools(raw);
            let text = modality_source(raw, "output")
                .map(|_| modalities(raw, "output").contains(&CatalogModality::Text));
            if tools == Some(false) || text == Some(false) || (tools.is_some() && text.is_some()) {
                return None;
            }
            raw.get("id")?.as_str().map(str::to_owned)
        })
        .collect()
}

fn advertised_tools(raw: &Value) -> Option<bool> {
    advertised_bool(
        raw,
        &[
            "tool_call",
            "tool_calls",
            "supports_tools",
            "function_calling",
        ],
    )
    .or_else(|| supports_parameter(raw, "tools").then_some(true))
}

fn advertised_bool(raw: &Value, names: &[&str]) -> Option<bool> {
    let mut advertised = None;
    for name in names {
        for value in [
            raw.get(*name),
            raw.get("capabilities").and_then(|caps| caps.get(*name)),
        ] {
            match value.and_then(Value::as_bool) {
                Some(false) => return Some(false),
                Some(true) => advertised = Some(true),
                None => {}
            }
        }
    }
    advertised
}

fn supports_parameter(raw: &Value, parameter: &str) -> bool {
    raw.get("supported_parameters")
        .and_then(Value::as_array)
        .is_some_and(|parameters| {
            parameters
                .iter()
                .take(64)
                .any(|value| value.as_str() == Some(parameter))
        })
}

fn modality_source<'a>(raw: &'a Value, direction: &str) -> Option<&'a Value> {
    let field = format!("{direction}_modalities");
    raw.get("modalities")
        .and_then(|modalities| modalities.get(direction))
        .or_else(|| raw.get(&field))
        .or_else(|| raw.get("architecture")?.get(&field))
}

fn modalities(raw: &Value, direction: &str) -> Vec<CatalogModality> {
    modality_source(raw, direction)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(16)
        .filter_map(|value| match value.as_str()? {
            "text" => Some(CatalogModality::Text),
            "image" => Some(CatalogModality::Image),
            "audio" => Some(CatalogModality::Audio),
            "video" => Some(CatalogModality::Video),
            "document" | "pdf" => Some(CatalogModality::Document),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn reasoning_controls(raw: &Value) -> Option<CatalogReasoningControls> {
    let mut toggle = raw
        .get("reasoning_controls")
        .and_then(|controls| controls.get("toggle"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut efforts = Vec::new();
    let mut add_efforts = |values: Option<&Vec<Value>>| {
        for value in values.into_iter().flatten().take(16) {
            let Some(value) = value.as_str() else {
                continue;
            };
            let value = value.to_ascii_lowercase();
            // OpenAI-compatible effort wire values Smith knows how to send.
            if matches!(
                value.as_str(),
                "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
            ) && !efforts.contains(&value)
            {
                efforts.push(value);
            }
        }
    };
    add_efforts(
        raw.get("reasoning_controls")
            .and_then(|controls| controls.get("efforts"))
            .and_then(Value::as_array),
    );
    for option in raw
        .get("reasoning_options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(32)
    {
        match option.get("type").and_then(Value::as_str) {
            Some("toggle") => toggle = true,
            Some("effort") => add_efforts(option.get("values").and_then(Value::as_array)),
            _ => {}
        }
    }
    (toggle || !efforts.is_empty()).then_some(CatalogReasoningControls { toggle, efforts })
}

fn normalize_cost(raw: &Value) -> Option<CatalogModelCost> {
    let schema = |name, unit| {
        raw.get(name)
            .filter(|value| value.is_object())
            .filter(|value| {
                value.get("currency").is_none_or(|currency| {
                    currency
                        .as_str()
                        .is_some_and(|currency| currency.eq_ignore_ascii_case("USD"))
                })
            })
            .filter(|_| {
                raw.get("currency").is_none_or(|currency| {
                    currency
                        .as_str()
                        .is_some_and(|currency| currency.eq_ignore_ascii_case("USD"))
                })
            })
            .filter(|value| {
                value
                    .get("unit")
                    .or_else(|| value.get("units"))
                    .is_none_or(|units| units.as_str() == Some(unit))
            })
    };
    let cost = schema("cost", "usd_per_million_tokens");
    let pricing = schema("pricing", "usd_per_token");
    let counter = |key: &str, pricing_key: &str| {
        cost.and_then(|cost| cost.get(key))
            .filter(|value| value.is_number())
            .and_then(|value| decimal_scaled(&value.to_string(), 6))
            .or_else(|| {
                pricing
                    .and_then(|pricing| pricing.get(pricing_key))
                    .and_then(Value::as_str)
                    .and_then(|value| decimal_scaled(value, 12))
            })
    };
    let cost = CatalogModelCost {
        input: counter("input", "prompt"),
        output: counter("output", "completion"),
        cache_read: counter("cache_read", "input_cache_read"),
        cache_write: counter("cache_write", "input_cache_write"),
    };
    (cost.input.is_some()
        || cost.output.is_some()
        || cost.cache_read.is_some()
        || cost.cache_write.is_some())
    .then_some(cost)
}

/// Exact checked decimal conversion, rounding at half a micro-USD. There is
/// no floating-point multiplication or saturating cast at the price boundary.
fn decimal_scaled(raw: &str, scale: i32) -> Option<u64> {
    if raw.is_empty()
        || raw.len() > 128
        || raw.starts_with('-')
        || raw.starts_with('+')
        || raw.trim() != raw
    {
        return None;
    }
    let (mantissa, exponent) = match raw.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().ok()?),
        None => (raw, 0),
    };
    if !(-128..=128).contains(&exponent) {
        return None;
    }
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte.is_ascii_digit())
        || (mantissa.contains('.') && fraction.is_empty())
    {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Some(0);
    }
    let shift = scale
        .checked_add(exponent)?
        .checked_sub(i32::try_from(fraction.len()).ok()?)?;
    if shift >= 0 {
        let value = digits.parse::<u128>().ok()?;
        let multiplier = 10_u128.checked_pow(u32::try_from(shift).ok()?)?;
        u64::try_from(value.checked_mul(multiplier)?).ok()
    } else {
        let removed = usize::try_from(shift.checked_neg()?).ok()?;
        if removed > digits.len() {
            return Some(0);
        }
        let retained = digits.len() - removed;
        let value = if retained == 0 {
            0
        } else {
            digits[..retained].parse::<u128>().ok()?
        };
        let rounded = value.checked_add(u128::from(digits.as_bytes()[retained] >= b'5'))?;
        u64::try_from(rounded).ok()
    }
}

pub(super) fn valid_catalog(catalog: &CatalogProvider) -> bool {
    if catalog.models.len() > MAX_MODELS {
        return false;
    }
    catalog.models.iter().all(|(id, model)| {
        id == &model.id
            && valid_text(id, MAX_ID_BYTES)
            && valid_text(&model.name, MAX_NAME_BYTES)
            && model
                .disabled_reason
                .as_deref()
                .is_none_or(|reason| valid_text(reason, 256))
            && model.limits.is_none_or(|limits| {
                limits.context_tokens > 0
                    && limits.max_input_tokens > 0
                    && limits.max_output_tokens > 0
                    && limits.max_input_tokens <= limits.context_tokens
                    && limits.max_output_tokens <= limits.context_tokens
            })
            && (model.limits.is_some() || model.disabled_reason.is_some())
            && model.input_modalities.len() <= 5
            && model.output_modalities.len() <= 5
            && model.reasoning_controls.as_ref().is_none_or(|controls| {
                model.reasoning
                    && controls.efforts.len() <= 7
                    && controls.efforts.iter().all(|effort| {
                        matches!(
                            effort.as_str(),
                            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
                        )
                    })
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decimal_conversion_is_checked_and_never_saturates() {
        assert_eq!(decimal_scaled("0.0000001", 12), Some(100_000));
        assert_eq!(decimal_scaled("0.083333", 6), Some(83_333));
        assert_eq!(decimal_scaled("1e-7", 12), Some(100_000));
        assert_eq!(decimal_scaled("1.2345675", 6), Some(1_234_568));
        assert_eq!(decimal_scaled("18446744073709.551615", 6), Some(u64::MAX));
        for invalid in [
            "18446744073709.551616",
            "1e99",
            "-1",
            "NaN",
            " 1",
            "1.",
            "",
            "1e-2147483648",
        ] {
            assert_eq!(decimal_scaled(invalid, 6), None, "{invalid}");
        }
        assert_eq!(decimal_scaled("0", 12), Some(0));
    }
}
