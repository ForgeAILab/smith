use super::*;

pub(super) fn parse_snapshot(bytes: &[u8]) -> Result<CatalogSnapshot, CatalogError> {
    if bytes.len() > MAX_NORMALIZED_CATALOG_BYTES {
        return Err(CatalogError::InvalidDocument(
            "normalized document exceeds the 2 MiB limit".to_owned(),
        ));
    }
    let snapshot: CatalogSnapshot = serde_json::from_slice(bytes)
        .map_err(|error| CatalogError::InvalidDocument(error.to_string()))?;
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn validate_snapshot(snapshot: &CatalogSnapshot) -> Result<(), CatalogError> {
    invalid_if(
        snapshot.schema_revision != CATALOG_SCHEMA_REVISION,
        "unsupported schema revision",
    )?;
    invalid_if(
        snapshot.source_url != MODELS_DEV_SOURCE_URL,
        "wrong catalog source origin",
    )?;
    invalid_if(
        !valid_digest(&snapshot.source_digest) || !valid_digest(&snapshot.content_digest),
        "catalog digest is malformed",
    )?;
    invalid_if(
        !valid_revision(&snapshot.source_revision),
        "catalog revision is malformed",
    )?;
    invalid_if(snapshot.retrieved_at_ms == 0, "retrieval time is missing")?;
    let actual: BTreeSet<&str> = snapshot.providers.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = EXPECTED_PROVIDERS.into_iter().collect();
    invalid_if(actual != expected, "supported provider set does not match")?;

    for (provider_id, provider) in &snapshot.providers {
        validate_text(provider_id, 128, "provider id")?;
        invalid_if(
            provider.id != *provider_id,
            "provider id does not match its key",
        )?;
        validate_text(&provider.name, MAX_NAME_BYTES, "provider name")?;
        invalid_if(
            provider.models.len() > MAX_MODELS_PER_PROVIDER,
            "provider model count exceeds the limit",
        )?;
        for (model_id, model) in &provider.models {
            validate_text(model_id, MAX_MODEL_ID_BYTES, "model id")?;
            invalid_if(model.id != *model_id, "model id does not match its key")?;
            validate_text(&model.name, MAX_NAME_BYTES, "model name")?;
            if let Some(reason) = &model.disabled_reason {
                validate_text(reason, MAX_DISABLED_REASON_BYTES, "disabled reason")?;
            }
            invalid_if(
                model.input_modalities.len() > MAX_MODALITIES
                    || model.output_modalities.len() > MAX_MODALITIES,
                "model has too many modalities",
            )?;
            invalid_if(
                !sorted_unique(&model.input_modalities) || !sorted_unique(&model.output_modalities),
                "model modalities are not normalized",
            )?;
            if let Some(limits) = model.limits {
                validate_limits(limits)?;
            } else {
                invalid_if(
                    model.disabled_reason.is_none(),
                    "model without limits has no disabled reason",
                )?;
            }
            if let Some(controls) = &model.reasoning_controls {
                invalid_if(
                    !model.reasoning,
                    "reasoning controls are present on a non-reasoning model",
                )?;
                invalid_if(
                    !controls.toggle && controls.efforts.is_empty(),
                    "reasoning controls advertise neither a switch nor efforts",
                )?;
                invalid_if(
                    controls.efforts.len() > MAX_REASONING_EFFORTS,
                    "model advertises too many reasoning efforts",
                )?;
                invalid_if(
                    !controls.efforts.iter().all(|effort| {
                        valid_effort_name(effort)
                            && controls.efforts.iter().filter(|e| *e == effort).count() == 1
                    }),
                    "reasoning efforts are not normalized",
                )?;
            }
        }
    }
    let value = serde_json::to_value(&snapshot.providers)
        .map_err(|error| CatalogError::InvalidDocument(error.to_string()))?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|error| CatalogError::InvalidDocument(error.to_string()))?;
    invalid_if(
        digest(&canonical) != snapshot.content_digest,
        "normalized content digest does not match",
    )
}

fn validate_limits(limits: CatalogLimits) -> Result<(), CatalogError> {
    invalid_if(
        limits.context_tokens == 0 || limits.max_input_tokens == 0 || limits.max_output_tokens == 0,
        "model limit is zero",
    )?;
    invalid_if(
        limits.max_output_tokens > limits.context_tokens,
        "model output limit exceeds context",
    )?;
    invalid_if(
        limits.max_input_tokens > limits.context_tokens,
        "model input limit exceeds context",
    )
}

pub(super) fn normalize_remote(
    bytes: &[u8],
    retrieved_at_ms: u64,
    source_revision: Option<&str>,
) -> Result<CatalogSnapshot, CatalogError> {
    invalid_if(
        bytes.len() > MAX_REMOTE_CATALOG_BYTES,
        "remote document exceeds the 8 MiB limit",
    )?;
    invalid_if(retrieved_at_ms == 0, "retrieval time is missing")?;
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|error| CatalogError::InvalidDocument(error.to_string()))?;
    let root = root
        .as_object()
        .ok_or_else(|| CatalogError::InvalidDocument("root is not an object".to_owned()))?;
    let mut providers = BTreeMap::new();
    for provider_id in EXPECTED_PROVIDERS {
        let raw = root.get(provider_id).ok_or_else(|| {
            CatalogError::InvalidDocument(format!("supported provider `{provider_id}` is missing"))
        })?;
        providers.insert(
            provider_id.to_owned(),
            normalize_provider(provider_id, raw)?,
        );
    }
    let value = serde_json::to_value(&providers)
        .map_err(|error| CatalogError::InvalidDocument(error.to_string()))?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|error| CatalogError::InvalidDocument(error.to_string()))?;
    let source_digest = digest(bytes);
    let revision = source_revision
        .filter(|revision| valid_revision(revision))
        .unwrap_or(&source_digest)
        .to_owned();
    let snapshot = CatalogSnapshot {
        schema_revision: CATALOG_SCHEMA_REVISION,
        source_url: MODELS_DEV_SOURCE_URL.to_owned(),
        source_digest,
        content_digest: digest(&canonical),
        source_revision: revision,
        retrieved_at_ms,
        providers,
    };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn normalize_provider(provider_id: &str, raw: &Value) -> Result<CatalogProvider, CatalogError> {
    let raw = raw.as_object().ok_or_else(|| {
        CatalogError::InvalidDocument(format!("provider `{provider_id}` is not an object"))
    })?;
    let id = required_text(raw, "id", 128, "provider id")?;
    invalid_if(id != provider_id, "provider id does not match its key")?;
    let name = required_text(raw, "name", MAX_NAME_BYTES, "provider name")?.to_owned();
    let models = raw
        .get("models")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            CatalogError::InvalidDocument(format!("provider `{provider_id}` has no model object"))
        })?;
    invalid_if(
        models.len() > MAX_MODELS_PER_PROVIDER,
        "provider model count exceeds the limit",
    )?;
    let mut normalized = BTreeMap::new();
    for (model_id, model) in models {
        if let Some(model) = normalize_model(model_id, model)? {
            normalized.insert(model_id.clone(), model);
        }
    }
    Ok(CatalogProvider {
        id: provider_id.to_owned(),
        name,
        models: normalized,
    })
}

pub(super) fn normalize_model(
    model_id: &str,
    raw: &Value,
) -> Result<Option<CatalogModel>, CatalogError> {
    validate_text(model_id, MAX_MODEL_ID_BYTES, "model id")?;
    let raw = raw.as_object().ok_or_else(|| {
        CatalogError::InvalidDocument(format!("model `{model_id}` is not an object"))
    })?;
    let id = required_text(raw, "id", MAX_MODEL_ID_BYTES, "model id")?;
    invalid_if(id != model_id, "model id does not match its key")?;
    let name = required_text(raw, "name", MAX_NAME_BYTES, "model name")?.to_owned();
    let status = raw.get("status").filter(|value| !value.is_null());
    if status.is_some_and(|value| value.as_str() == Some("deprecated")) {
        return Ok(None);
    }
    let mut disabled_reason = status.map(|_| "catalog model has an unsupported status".to_owned());

    let (limits, limit_error) = normalize_limits(raw.get("limit"));
    let (input_modalities, output_modalities, modality_error) =
        normalize_modalities(raw.get("modalities"));
    let (tool_call, tool_error) = normalized_bool(raw, "tool_call");
    let (reasoning, reasoning_error) = normalized_bool(raw, "reasoning");
    let (structured_output, structured_error) = normalized_bool(raw, "structured_output");
    disabled_reason = disabled_reason
        .or(limit_error)
        .or(modality_error)
        .or(tool_error)
        .or(reasoning_error)
        .or(structured_error);

    Ok(Some(CatalogModel {
        id: model_id.to_owned(),
        name,
        limits,
        input_modalities,
        output_modalities,
        tool_call,
        reasoning,
        reasoning_controls: normalize_reasoning_controls(raw, reasoning),
        structured_output,
        // Deliberately outside the `disabled_reason` chain above: a price is
        // additive-only and must never disable an otherwise-valid model, so
        // `normalize_cost` has no error path to feed into it.
        cost: normalize_cost(raw.get("cost")),
        disabled_reason,
    }))
}

/// Keeps only the advertised control shapes Smith can express: an on/off
/// switch and an ordered effort ladder. `budget_tokens` and unknown option
/// types grant nothing, and invalid entries are dropped rather than
/// disabling an otherwise valid model.
///
/// `scripts/generate-model-catalog.py` implements the same normalization;
/// the two must stay byte-identical for the seed reproducibility check.
fn normalize_reasoning_controls(
    raw: &serde_json::Map<String, Value>,
    reasoning: bool,
) -> Option<CatalogReasoningControls> {
    if !reasoning {
        return None;
    }
    let entries = raw.get("reasoning_options")?.as_array()?;
    let mut toggle = false;
    let mut efforts: Vec<String> = Vec::new();
    for entry in entries.iter().take(MAX_REASONING_OPTION_ENTRIES) {
        match entry.get("type").and_then(Value::as_str) {
            Some("toggle") => toggle = true,
            Some("effort") => {
                let values = entry.get("values").and_then(Value::as_array);
                for value in values.into_iter().flatten() {
                    if efforts.len() == MAX_REASONING_EFFORTS {
                        break;
                    }
                    let Some(text) = value.as_str() else { continue };
                    let lowered = text.to_ascii_lowercase();
                    if valid_effort_name(&lowered) && !efforts.contains(&lowered) {
                        efforts.push(lowered);
                    }
                }
            }
            _ => {}
        }
    }
    (toggle || !efforts.is_empty()).then_some(CatalogReasoningControls { toggle, efforts })
}

fn valid_effort_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_REASONING_EFFORT_BYTES
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn normalize_limits(raw: Option<&Value>) -> (Option<CatalogLimits>, Option<String>) {
    let Some(raw) = raw.and_then(Value::as_object) else {
        return (
            None,
            Some("catalog model has no valid limit declaration".to_owned()),
        );
    };
    let context = positive_u32(raw.get("context"));
    let output = positive_u32(raw.get("output"));
    let input = match raw.get("input") {
        None | Some(Value::Null) => context,
        value => positive_u32(value),
    };
    let (Some(context), Some(input), Some(output)) = (context, input, output) else {
        return (
            None,
            Some("catalog model has a zero, missing, or out-of-range limit".to_owned()),
        );
    };
    if output > context {
        return (
            None,
            Some("catalog output limit exceeds its context window".to_owned()),
        );
    }
    if input > context {
        return (
            None,
            Some("catalog input limit exceeds its context window".to_owned()),
        );
    }
    (
        Some(CatalogLimits {
            context_tokens: context,
            max_input_tokens: input,
            max_output_tokens: output,
        }),
        None,
    )
}

/// Normalizes the Models.dev `cost` block into a per-counter price.
///
/// Unlike every other normalized field, an invalid or missing price is never
/// a reason to disable a model — a price only ever adds information, it
/// never withholds a model that was otherwise selectable. So, unlike
/// `normalize_limits` and its neighbours, this returns no error string and
/// feeds no `disabled_reason`: each counter (`input`, `output`,
/// `cache_read`, `cache_write`) is converted independently, and a negative,
/// infinite, non-numeric, or absent counter is simply dropped rather than
/// invalidating the whole block or the model. An entry whose every counter
/// is invalid or absent normalizes to `None`, identical to a source that
/// published no `cost` block at all.
///
/// `scripts/generate-model-catalog.py` implements the same normalization;
/// the two must stay byte-identical for the seed reproducibility check.
fn normalize_cost(raw: Option<&Value>) -> Option<CatalogModelCost> {
    let raw = raw?.as_object()?;
    let cost = CatalogModelCost {
        input: micro_usd_per_million(raw.get("input")),
        output: micro_usd_per_million(raw.get("output")),
        cache_read: micro_usd_per_million(raw.get("cache_read")),
        cache_write: micro_usd_per_million(raw.get("cache_write")),
    };
    (cost.input.is_some()
        || cost.output.is_some()
        || cost.cache_read.is_some()
        || cost.cache_write.is_some())
    .then_some(cost)
}

/// Converts one Models.dev USD-per-million-token counter into micro-USD
/// (1e-6 USD) per million tokens, Smith's fixed-point price unit.
///
/// Rejects a missing or `null` counter, a non-numeric value, and a negative
/// or non-finite (`NaN`/infinite) number. Uses "add a half unit, then floor"
/// rather than `f64::round` so the rounding rule is the exact same
/// floating-point operation Python's `math.floor` performs in
/// `scripts/generate-model-catalog.py`, keeping the two implementations
/// byte-identical for every value Models.dev actually publishes.
pub(super) fn micro_usd_per_million(raw: Option<&Value>) -> Option<u64> {
    let value = raw?.as_f64()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let micro = (value * 1_000_000.0 + 0.5).floor();
    if micro > u64::MAX as f64 {
        return None;
    }
    Some(micro as u64)
}

fn normalize_modalities(
    raw: Option<&Value>,
) -> (Vec<CatalogModality>, Vec<CatalogModality>, Option<String>) {
    let Some(raw) = raw.and_then(Value::as_object) else {
        return (
            Vec::new(),
            Vec::new(),
            Some("catalog model has no valid modality declaration".to_owned()),
        );
    };
    let input = normalize_modality_list(raw.get("input"), "input");
    let output = normalize_modality_list(raw.get("output"), "output");
    match (input, output) {
        (Ok(input), Ok(output)) => (input, output, None),
        (Err(error), _) | (_, Err(error)) => (Vec::new(), Vec::new(), Some(error)),
    }
}

fn normalize_modality_list(
    raw: Option<&Value>,
    direction: &str,
) -> Result<Vec<CatalogModality>, String> {
    let Some(raw) = raw.and_then(Value::as_array) else {
        return Err(format!("catalog `{direction}` modalities are invalid"));
    };
    if raw.len() > MAX_MODALITIES {
        return Err(format!("catalog `{direction}` modalities are invalid"));
    }
    let mut modalities = BTreeSet::new();
    for modality in raw {
        let parsed = match modality.as_str() {
            Some("text") => CatalogModality::Text,
            Some("image") => CatalogModality::Image,
            Some("audio") => CatalogModality::Audio,
            Some("video") => CatalogModality::Video,
            Some("pdf" | "document") => CatalogModality::Document,
            _ => return Err(format!("catalog `{direction}` modality is unsupported")),
        };
        modalities.insert(parsed);
    }
    Ok(modalities.into_iter().collect())
}

fn normalized_bool(raw: &Map<String, Value>, field: &str) -> (bool, Option<String>) {
    match raw.get(field) {
        None => (false, None),
        Some(Value::Bool(value)) => (*value, None),
        Some(_) => (
            false,
            Some(format!("catalog field `{field}` is not boolean")),
        ),
    }
}

fn positive_u32(raw: Option<&Value>) -> Option<u32> {
    u32::try_from(raw?.as_u64()?)
        .ok()
        .filter(|value| *value > 0)
}

fn required_text<'a>(
    raw: &'a Map<String, Value>,
    field: &str,
    maximum: usize,
    label: &str,
) -> Result<&'a str, CatalogError> {
    let value = raw
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| CatalogError::InvalidDocument(format!("{label} must be bounded text")))?;
    validate_text(value, maximum, label)?;
    Ok(value)
}

fn validate_text(value: &str, maximum: usize, label: &str) -> Result<(), CatalogError> {
    invalid_if(
        value.is_empty()
            || value.len() > maximum
            || value.chars().any(|character| character.is_control()),
        &format!("{label} must be non-empty bounded text"),
    )
}

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

pub(super) fn valid_revision(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_REVISION_BYTES && !value.chars().any(char::is_control)
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn sorted_unique<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn invalid_if(condition: bool, message: &str) -> Result<(), CatalogError> {
    if condition {
        Err(CatalogError::InvalidDocument(message.to_owned()))
    } else {
        Ok(())
    }
}
