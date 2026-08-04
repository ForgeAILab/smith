//! Provider, model, reasoning, context, and policy validation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::model::{
    ApprovalMode, BackgroundExit, KIND_ANTHROPIC_MESSAGES, KIND_CHATGPT_RESPONSES, KIND_FAKE,
    KIND_GEMINI_INTERACTIONS, KIND_OPENAI_COMPATIBLE, ReasoningDialect, ReasoningOnlyBehavior,
};
use agent_runtime_core::store::Secret;

use super::load::join_key;
use super::provenance::*;
use super::types::*;

pub(super) fn resolve_provider(
    provenance: &Provenance,
    name: Sourced<String>,
) -> Result<ResolvedProvider, ConfigError> {
    let scope = join_key(&["providers", name.value.as_str()]);
    let kind =
        text(provenance, &format!("{scope}.kind"))?.ok_or_else(|| ConfigError::MissingSetting {
            key: format!("{scope}.kind"),
            message: format!(
                "provider `{}` must name the shared adapter it uses",
                name.value
            ),
        })?;
    let base_url = text(provenance, &format!("{scope}.base_url"))?;
    let credential = text(provenance, &format!("{scope}.credential"))?;
    let api_key = secret(provenance, &format!("{scope}.api_key"))?;
    let reasoning_only = text(provenance, &format!("{scope}.response.reasoning_only"))?
        .map(|value| {
            ReasoningOnlyBehavior::parse(&value.value)
                .map(|behavior| Sourced::new(behavior, value.source.clone()))
                .ok_or_else(|| ConfigError::InvalidValue {
                    source: value.source,
                    message: "`reasoning_only` must be `reasoning` or `text`".to_owned(),
                })
        })
        .transpose()?;

    let header_prefix = format!("{scope}.headers.");
    let header_keys: Vec<String> = provenance
        .keys()
        .filter(|key| key.starts_with(&header_prefix))
        .map(str::to_owned)
        .collect();
    let mut headers = BTreeMap::new();
    for key in header_keys {
        let header = unquote_segment(&key[header_prefix.len()..]);
        if let Some(value) = text(provenance, &key)? {
            headers.insert(header, value);
        }
    }

    let provider = ResolvedProvider {
        name,
        kind,
        base_url,
        credential,
        api_key,
        headers,
        response: ResolvedProviderResponse { reasoning_only },
    };
    validate_provider(&provider)?;
    Ok(provider)
}

/// Checks the options against what the adapter kind can use.
///
/// Kinds Smith does not know are not rejected here: which adapters exist is a
/// property of the pinned runtime's registry, and reporting an unavailable
/// adapter belongs to the step that consults it. The secret rules apply to
/// every kind, because they protect the file rather than the adapter.
pub(super) fn validate_provider(provider: &ResolvedProvider) -> Result<(), ConfigError> {
    if provider.credential.is_some() && provider.api_key.is_some() {
        let source = provider
            .api_key
            .as_ref()
            .map(|value| value.source.clone())
            .expect("the inline key was checked as present");
        return Err(ConfigError::InvalidValue {
            source,
            message: "choose exactly one credential source: `credential` or `api_key`".to_owned(),
        });
    }
    if let Some(credential) = &provider.credential {
        validate_credential(credential)?;
    }
    for (name, value) in &provider.headers {
        if AUTH_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(ConfigError::PlaintextSecret {
                source: value.source.clone(),
                message: format!(
                    "set the provider's `credential` reference instead of writing an `{name}` header"
                ),
            });
        }
    }
    if !matches!(
        provider.kind.value.as_str(),
        KIND_OPENAI_COMPATIBLE
            | KIND_ANTHROPIC_MESSAGES
            | KIND_CHATGPT_RESPONSES
            | KIND_GEMINI_INTERACTIONS
    ) && let Some(policy) = &provider.response.reasoning_only
    {
        return Err(ConfigError::IncompatibleOption {
            source: policy.source.clone(),
            kind: provider.kind.value.clone(),
            message: "`response.reasoning_only` requires an adapter that exposes reasoning events"
                .to_owned(),
        });
    }

    match provider.kind.value.as_str() {
        KIND_OPENAI_COMPATIBLE => {
            if provider.base_url.is_none() {
                return Err(ConfigError::MissingSetting {
                    key: join_key(&["providers", &provider.name.value, "base_url"]),
                    message: format!(
                        "an `{KIND_OPENAI_COMPATIBLE}` provider needs the endpoint it talks to"
                    ),
                });
            }
        }
        // The Messages API has one well-known endpoint, so `base_url` is
        // optional and defaults at provider construction; everything else a
        // provider table carries (credential, headers) applies unchanged.
        KIND_ANTHROPIC_MESSAGES => {}
        KIND_CHATGPT_RESPONSES => {
            let expected_endpoint = crate::setup::CHATGPT_ENDPOINT;
            if provider.base_url.as_ref().map(|value| value.value.as_str())
                != Some(expected_endpoint)
            {
                let source = provider.base_url.as_ref().map_or_else(
                    || provider.kind.source.clone(),
                    |value| value.source.clone(),
                );
                return Err(ConfigError::InvalidValue {
                    source,
                    message:
                        "the experimental ChatGPT provider uses Smith's fixed trusted endpoint"
                            .to_owned(),
                });
            }
            if provider
                .credential
                .as_ref()
                .map(|value| value.value.as_str())
                != Some(crate::setup::CHATGPT_CREDENTIAL)
                || provider.api_key.is_some()
                || !provider.headers.is_empty()
            {
                let source = provider.credential.as_ref().map_or_else(
                    || provider.kind.source.clone(),
                    |value| value.source.clone(),
                );
                return Err(ConfigError::InvalidValue {
                    source,
                    message: "the experimental ChatGPT provider requires Smith OAuth at `authfile:chatgpt`"
                        .to_owned(),
                });
            }
        }
        KIND_GEMINI_INTERACTIONS => {
            if let Some(base_url) = &provider.base_url {
                return Err(ConfigError::IncompatibleOption {
                    source: base_url.source.clone(),
                    kind: KIND_GEMINI_INTERACTIONS.to_owned(),
                    message:
                        "native Gemini uses Smith's fixed endpoint and does not accept `base_url`"
                            .to_owned(),
                });
            }
            if !provider.headers.is_empty() {
                let source = provider
                    .headers
                    .values()
                    .next()
                    .expect("non-empty headers have a first value")
                    .source
                    .clone();
                return Err(ConfigError::IncompatibleOption {
                    source,
                    kind: KIND_GEMINI_INTERACTIONS.to_owned(),
                    message:
                        "native Gemini uses its API-key header internally and does not accept custom headers"
                            .to_owned(),
                });
            }
            if let Some(policy) = &provider.response.reasoning_only {
                return Err(ConfigError::IncompatibleOption {
                    source: policy.source.clone(),
                    kind: KIND_GEMINI_INTERACTIONS.to_owned(),
                    message:
                        "native Gemini does not accept `response.reasoning_only`; reasoning events are preserved by the adapter"
                            .to_owned(),
                });
            }
        }
        KIND_FAKE => {
            for (source, option) in [
                (provider.base_url.as_ref(), "base_url"),
                (provider.credential.as_ref(), "credential"),
            ] {
                if let Some(sourced) = source {
                    return Err(ConfigError::IncompatibleOption {
                        source: sourced.source.clone(),
                        kind: KIND_FAKE.to_owned(),
                        message: format!(
                            "the deterministic provider sends nothing, so `{option}` would never be used"
                        ),
                    });
                }
            }
            if let Some(api_key) = &provider.api_key {
                return Err(ConfigError::IncompatibleOption {
                    source: api_key.source.clone(),
                    kind: KIND_FAKE.to_owned(),
                    message:
                        "the deterministic provider sends nothing, so `api_key` would never be used"
                            .to_owned(),
                });
            }
            if let Some(value) = provider.headers.values().next() {
                return Err(ConfigError::IncompatibleOption {
                    source: value.source.clone(),
                    kind: KIND_FAKE.to_owned(),
                    message: "the deterministic provider sends no requests to add headers to"
                        .to_owned(),
                });
            }
        }
        _ => {}
    }
    Ok(())
}

/// Checks that a credential is a reference rather than the secret itself.
pub(super) fn validate_credential(credential: &Sourced<String>) -> Result<(), ConfigError> {
    let refused = || ConfigError::PlaintextSecret {
        source: credential.source.clone(),
        message: format!(
            "write a reference such as `keychain:smith/<provider>`; the schemes are {}",
            CREDENTIAL_SCHEMES
                .iter()
                .map(|scheme| format!("`{scheme}:`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let Some((scheme, rest)) = credential.value.split_once(':') else {
        return Err(refused());
    };
    if !CREDENTIAL_SCHEMES.contains(&scheme) || rest.trim().is_empty() {
        return Err(refused());
    }
    Ok(())
}

pub(super) fn resolve_model_limits(
    provenance: &Provenance,
    provider: &str,
    model: &str,
) -> Result<ResolvedModelLimits, ConfigError> {
    let scope = join_key(&["models", &format!("{provider}/{model}")]);
    Ok(ResolvedModelLimits {
        context_tokens: optional_u32(provenance, &format!("{scope}.context_tokens"))?,
        max_input_tokens: optional_u32(provenance, &format!("{scope}.max_input_tokens"))?,
        max_output_tokens: optional_u32(provenance, &format!("{scope}.max_output_tokens"))?,
    })
}

pub(super) fn resolve_model_reasoning(
    provenance: &Provenance,
    provider: &str,
    model: &str,
) -> Result<ResolvedModelReasoning, ConfigError> {
    let scope = format!(
        "{}.reasoning",
        join_key(&["models", &format!("{provider}/{model}")])
    );
    let dialect = text(provenance, &format!("{scope}.dialect"))?
        .map(|raw| {
            let Some(value) = ReasoningDialect::ALL
                .into_iter()
                .find(|dialect| dialect.as_str() == raw.value)
            else {
                let supported = ReasoningDialect::ALL
                    .map(|dialect| format!("`{}`", dialect.as_str()))
                    .join(", ");
                return Err(ConfigError::InvalidValue {
                    source: raw.source,
                    message: format!(
                        "`{}` is not a reasoning dialect; use {supported}",
                        raw.value
                    ),
                });
            };
            Ok(Sourced::new(value, raw.source))
        })
        .transpose()?;
    let efforts = list(provenance, &format!("{scope}.efforts"))?;
    if let Some(efforts) = &efforts {
        if efforts.value.is_empty() {
            return Err(ConfigError::InvalidValue {
                source: efforts.source.clone(),
                message: "`efforts` must contain at least one advertised value".to_owned(),
            });
        }
        let mut seen = BTreeSet::new();
        for effort in &efforts.value {
            if effort.is_empty()
                || effort.len() > 32
                || !effort
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(ConfigError::InvalidValue {
                    source: efforts.source.clone(),
                    message: format!(
                        "reasoning effort `{effort}` must contain 1 to 32 ASCII letters, digits, `-`, or `_`"
                    ),
                });
            }
            if !seen.insert(effort) {
                return Err(ConfigError::InvalidValue {
                    source: efforts.source.clone(),
                    message: format!("`efforts` contains duplicate value `{effort}`"),
                });
            }
        }
    }
    Ok(ResolvedModelReasoning {
        toggle: flag(provenance, &format!("{scope}.toggle"))?,
        mandatory: flag(provenance, &format!("{scope}.mandatory"))?,
        efforts,
        default_enabled: flag(provenance, &format!("{scope}.default_enabled"))?,
        default_effort: text(provenance, &format!("{scope}.default_effort"))?,
        dialect,
    })
}

pub(super) fn resolve_context(provenance: &Provenance) -> Result<ResolvedContext, ConfigError> {
    let high = required_percent(provenance, "context.compaction_high_watermark_percent")?;
    let low = required_percent(provenance, "context.compaction_low_watermark_percent")?;
    if low.value >= high.value {
        return Err(ConfigError::InvalidValue {
            source: low.source,
            message: format!(
                "compaction must leave room below the watermark it triggers at ({}%)",
                high.value
            ),
        });
    }
    Ok(ResolvedContext {
        output_reserve: optional_u32(provenance, "context.output_reserve")?,
        reasoning_reserve: required_u32(provenance, "context.reasoning_reserve")?,
        capability_budget: optional_u32(provenance, "context.capability_budget")?,
        max_estimated_slack: optional_u32(provenance, "context.max_estimated_slack")?,
        compaction_high_watermark_percent: high,
        compaction_low_watermark_percent: low,
        idle_compaction_ms: required_u64(provenance, "context.idle_compaction_ms")?,
    })
}

pub(super) fn resolve_limits(provenance: &Provenance) -> Result<ResolvedLimits, ConfigError> {
    Ok(ResolvedLimits {
        max_retries: required_u32(provenance, "limits.max_retries")?,
        max_tool_steps: required_u32(provenance, "limits.max_tool_steps")?,
        turn_time_limit_ms: required_u64(provenance, "limits.turn_time_limit_ms")?,
        tool_output_limit_bytes: required_u64(provenance, "limits.tool_output_limit_bytes")?,
    })
}

pub(super) fn resolve_persistence(
    provenance: &Provenance,
) -> Result<ResolvedPersistence, ConfigError> {
    let sessions_dir = required_text(provenance, "persistence.sessions_dir")?;
    let checkpoint_key = secret(provenance, "persistence.checkpoint_key")?;
    let checkpoint_key_credential = text(provenance, "persistence.checkpoint_key_credential")?;
    if let (Some(key), Some(_credential)) = (&checkpoint_key, &checkpoint_key_credential) {
        return Err(ConfigError::InvalidValue {
            source: key.source.clone(),
            message: "choose exactly one checkpoint key source: `checkpoint_key` or `checkpoint_key_credential`"
                .to_owned(),
        });
    }
    if let Some(key) = &checkpoint_key {
        let exposed = key.value.expose();
        if exposed.len() != 64 || !exposed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ConfigError::InvalidValue {
                source: key.source.clone(),
                message:
                    "`checkpoint_key` must encode exactly 32 bytes as 64 hexadecimal characters"
                        .to_owned(),
            });
        }
    }
    if let Some(credential) = &checkpoint_key_credential {
        validate_credential(credential)?;
    }
    Ok(ResolvedPersistence {
        enabled: required_flag(provenance, "persistence.enabled")?,
        sessions_dir: Sourced::new(PathBuf::from(sessions_dir.value), sessions_dir.source),
        journal_events: required_flag(provenance, "persistence.journal_events")?,
        checkpoint_key,
        checkpoint_key_credential,
    })
}

pub(super) fn resolve_approval(provenance: &Provenance) -> Result<ResolvedApproval, ConfigError> {
    let raw = required_text(provenance, "approval.mode")?;
    let mode = ApprovalMode::parse(&raw.value).ok_or_else(|| ConfigError::InvalidValue {
        source: raw.source.clone(),
        message: format!(
            "`{}` is not an approval mode; the modes are {}",
            raw.value,
            list_spellings(ApprovalMode::spellings())
        ),
    })?;
    Ok(ResolvedApproval {
        mode: Sourced::new(mode, raw.source),
        auto_approve: list(provenance, "approval.auto_approve")?,
    })
}

pub(super) fn resolve_background(
    provenance: &Provenance,
) -> Result<ResolvedBackground, ConfigError> {
    let raw = required_text(provenance, "background.exit_policy")?;
    let policy = BackgroundExit::parse(&raw.value).ok_or_else(|| ConfigError::InvalidValue {
        source: raw.source.clone(),
        message: format!(
            "`{}` is not a background-exit policy; the policies are {}",
            raw.value,
            list_spellings(BackgroundExit::spellings())
        ),
    })?;
    Ok(ResolvedBackground {
        exit_policy: Sourced::new(policy, raw.source),
        max_children: required_u32(provenance, "background.max_children")?,
        max_monitors: required_u32(provenance, "background.max_monitors")?,
    })
}

pub(super) fn list_spellings(spellings: &[&str]) -> String {
    spellings
        .iter()
        .map(|spelling| format!("`{spelling}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn text(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<String>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::Text(value) => {
                Ok(Some(Sourced::new(value.clone(), entry.source.clone())))
            }
            other => Err(wrong_kind(entry, other, "a string")),
        },
    }
}

pub(super) fn secret(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<Secret>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::Secret(value) => {
                Ok(Some(Sourced::new(value.clone(), entry.source.clone())))
            }
            other => Err(wrong_kind(entry, other, "a secret string")),
        },
    }
}

pub(super) fn integer(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<i64>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::Integer(value) => Ok(Some(Sourced::new(*value, entry.source.clone()))),
            other => Err(wrong_kind(entry, other, "a whole number")),
        },
    }
}

pub(super) fn flag(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<bool>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::Flag(value) => Ok(Some(Sourced::new(*value, entry.source.clone()))),
            other => Err(wrong_kind(entry, other, "`true` or `false`")),
        },
    }
}

pub(super) fn list(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<Vec<String>>>, ConfigError> {
    match provenance.winner(key) {
        None => Ok(None),
        Some(entry) => match &entry.value {
            SettingValue::List(value) => {
                Ok(Some(Sourced::new(value.clone(), entry.source.clone())))
            }
            other => Err(wrong_kind(entry, other, "a list of strings")),
        },
    }
}

pub(super) fn wrong_kind(entry: &Entry, found: &SettingValue, expected: &str) -> ConfigError {
    ConfigError::InvalidValue {
        source: entry.source.clone(),
        message: format!("expected {expected}, found {}", describe(found.kind())),
    }
}

pub(super) fn describe(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Text => "a string",
        ValueKind::Secret => "a secret string",
        ValueKind::Integer => "a whole number",
        ValueKind::Flag => "a boolean",
        ValueKind::List => "a list",
    }
}

pub(super) fn optional_u32(
    provenance: &Provenance,
    key: &str,
) -> Result<Option<Sourced<u32>>, ConfigError> {
    integer(provenance, key)?.map(narrow_u32).transpose()
}

pub(super) fn required_u32(
    provenance: &Provenance,
    key: &str,
) -> Result<Sourced<u32>, ConfigError> {
    narrow_u32(integer(provenance, key)?.ok_or_else(|| missing(key))?)
}

pub(super) fn required_u64(
    provenance: &Provenance,
    key: &str,
) -> Result<Sourced<u64>, ConfigError> {
    let value = integer(provenance, key)?.ok_or_else(|| missing(key))?;
    match u64::try_from(value.value) {
        Ok(narrowed) => Ok(Sourced::new(narrowed, value.source)),
        Err(_) => Err(out_of_range(value, "0 and 9223372036854775807")),
    }
}

pub(super) fn required_percent(
    provenance: &Provenance,
    key: &str,
) -> Result<Sourced<u8>, ConfigError> {
    let value = integer(provenance, key)?.ok_or_else(|| missing(key))?;
    match u8::try_from(value.value) {
        Ok(narrowed) if narrowed <= 100 => Ok(Sourced::new(narrowed, value.source)),
        _ => Err(out_of_range(value, "0 and 100")),
    }
}

pub(super) fn required_text(
    provenance: &Provenance,
    key: &str,
) -> Result<Sourced<String>, ConfigError> {
    text(provenance, key)?.ok_or_else(|| missing(key))
}

pub(super) fn required_flag(
    provenance: &Provenance,
    key: &str,
) -> Result<Sourced<bool>, ConfigError> {
    flag(provenance, key)?.ok_or_else(|| missing(key))
}

pub(super) fn narrow_u32(value: Sourced<i64>) -> Result<Sourced<u32>, ConfigError> {
    match u32::try_from(value.value) {
        Ok(narrowed) => Ok(Sourced::new(narrowed, value.source)),
        Err(_) => Err(out_of_range(value, "0 and 4294967295")),
    }
}

pub(super) fn out_of_range(value: Sourced<i64>, range: &str) -> ConfigError {
    ConfigError::InvalidValue {
        source: value.source,
        message: format!("{} is outside {range}", value.value),
    }
}

/// A setting no layer supplied. No file is named because none is at fault: the
/// value is absent everywhere, including from Smith's own defaults.
pub(super) fn missing(key: &str) -> ConfigError {
    ConfigError::MissingSetting {
        key: key.to_owned(),
        message: "no layer supplied a value".to_owned(),
    }
}

pub(super) fn unquote_segment(segment: &str) -> String {
    segment
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .map_or_else(|| segment.to_owned(), |inner| inner.replace("\\\"", "\""))
}

/// Known names close enough to `candidate` to be worth suggesting.
///
/// The threshold grows with the length of the word so that `mdel` suggests
/// `model` without a twenty-character key suggesting every other one.
pub(super) fn nearest<'a>(candidate: &str, known: impl Iterator<Item = &'a str>) -> Vec<String> {
    let budget = (candidate.chars().count() / 3).max(1);
    let mut scored: Vec<(usize, &str)> = known
        .map(|name| (distance(candidate, name), name))
        .filter(|(score, _)| *score <= budget)
        .collect();
    scored.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)));
    scored
        .into_iter()
        .take(3)
        .map(|(_, name)| name.to_owned())
        .collect()
}

/// Edit distance, case-insensitive, counting a swap of two neighbours as one
/// edit.
///
/// Plain Levenshtein charges two edits for a transposition, which is the most
/// common typo of all: `modle` would then be as far from `model` as a word
/// with two unrelated mistakes, and the suggestion a user most needs would be
/// the one they never see.
pub(super) fn distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.to_lowercase().chars().collect();
    let right: Vec<char> = right.to_lowercase().chars().collect();
    let mut grid = vec![vec![0usize; right.len() + 1]; left.len() + 1];
    for (i, row) in grid.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in grid[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=left.len() {
        for j in 1..=right.len() {
            let cost = usize::from(left[i - 1] != right[j - 1]);
            let mut best = (grid[i - 1][j - 1] + cost)
                .min(grid[i - 1][j] + 1)
                .min(grid[i][j - 1] + 1);
            if i > 1 && j > 1 && left[i - 1] == right[j - 2] && left[i - 2] == right[j - 1] {
                best = best.min(grid[i - 2][j - 2] + 1);
            }
            grid[i][j] = best;
        }
    }
    grid[left.len()][right.len()]
}
