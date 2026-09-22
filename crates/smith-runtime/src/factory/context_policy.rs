//! Context-window selection and runtime context-policy derivation.

use super::*;

#[derive(Debug, Clone)]
pub(super) struct ContextWindowSelection {
    pub(super) active: Option<ContextWindowChoice>,
    pub(super) available: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ContextWindowChoice {
    pub(super) name: String,
    pub(super) context_tokens: Option<u32>,
    pub(super) max_input_tokens: Option<u32>,
    pub(super) max_input_source: Option<Source>,
    pub(super) source: Source,
    pub(super) origin: ContextWindowOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ContextWindowOrigin {
    Config,
    Trusted,
    Endpoint,
}

pub(super) fn resolve_context_window_selection(
    config: &ResolvedConfig,
    provider_kind: &str,
    endpoint: Option<&str>,
    model: &str,
) -> Result<ContextWindowSelection, FactoryError> {
    let provider = config.provider.name.value.as_str();
    let trusted = trusted_model(provider, model);
    let endpoint_windows =
        smith_config::catalog::endpoint_context_windows(provider_kind, endpoint, model);
    let mut windows = BTreeMap::<String, ContextWindowChoice>::new();

    for (name, window) in &config.model_limits.context_windows {
        insert_context_window(
            &mut windows,
            ContextWindowChoice {
                name: name.clone(),
                context_tokens: Some(window.context_tokens.value),
                max_input_tokens: window.max_input_tokens.as_ref().map(|value| value.value),
                max_input_source: window
                    .max_input_tokens
                    .as_ref()
                    .map(|value| value.source.clone()),
                source: window.context_tokens.source.clone(),
                origin: ContextWindowOrigin::Config,
            },
        );
    }

    if let Some(record) = trusted
        && let Some(trusted_windows) = record.context_windows
    {
        for window in trusted_windows {
            insert_context_window(
                &mut windows,
                ContextWindowChoice {
                    name: window.name.to_owned(),
                    context_tokens: Some(window.context_tokens),
                    max_input_tokens: window.max_input_tokens,
                    max_input_source: None,
                    source: Source::built_in(format!(
                        "trusted catalog {}@{} models.\"{}/{}\".context_windows.{}",
                        record.catalog, record.revision, record.provider, record.model, window.name
                    )),
                    origin: ContextWindowOrigin::Trusted,
                },
            );
        }
    }

    if let Some(endpoint_windows) = endpoint_windows {
        for window in endpoint_windows {
            insert_context_window(
                &mut windows,
                ContextWindowChoice {
                    name: window.name.to_owned(),
                    context_tokens: window.context_tokens,
                    max_input_tokens: None,
                    max_input_source: None,
                    source: Source::built_in(format!(
                        "OpenAI endpoint model catalog models.\"{provider}/{model}\".context_windows.{}",
                        window.name
                    )),
                    origin: ContextWindowOrigin::Endpoint,
                },
            );
        }
    }

    let pinned_by = config.model_limits.flat_context_source.as_ref();
    if let Some(pin) = pinned_by {
        windows.retain(|_, window| pin.layer.precedence() < window.source.layer.precedence());
    }

    let default_name = config
        .context_window
        .as_ref()
        .map(|window| window.value.clone())
        .or_else(|| {
            config
                .model_limits
                .default_context_window
                .as_ref()
                .map(|window| window.value.clone())
        })
        .or_else(|| trusted.and_then(|record| record.default_context_window.map(str::to_owned)))
        .or_else(|| {
            endpoint_windows
                .and_then(|options| options.iter().find(|window| window.default))
                .map(|window| window.name.to_owned())
        });

    let available = windows.keys().cloned().collect::<Vec<_>>();
    let active = match default_name {
        Some(name) => match windows.get(&name) {
            Some(window) => Some(window.clone()),
            None => {
                if let Some(pin) = pinned_by {
                    return Err(context_window_error(
                        provider,
                        &ModelId::new(model),
                        format!(
                            "window `{name}` is pinned by flat model limit `{}`; remove that limit to select a named window",
                            pin.key
                        ),
                    ));
                }
                let options = if available.is_empty() {
                    "none declared".to_owned()
                } else {
                    available.join(", ")
                };
                return Err(context_window_error(
                    provider,
                    &ModelId::new(model),
                    format!("unknown window `{name}`; available windows: {options}"),
                ));
            }
        },
        None => None,
    };

    Ok(ContextWindowSelection { active, available })
}

pub(super) fn insert_context_window(
    windows: &mut BTreeMap<String, ContextWindowChoice>,
    candidate: ContextWindowChoice,
) {
    match windows.entry(candidate.name.clone()) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(candidate);
        }
        std::collections::btree_map::Entry::Occupied(mut entry)
            if candidate.source.layer.precedence() > entry.get().source.layer.precedence() =>
        {
            entry.insert(candidate);
        }
        _ => {}
    }
}

pub(super) fn required_window_context(
    choice: &ContextWindowChoice,
    provider: &str,
    model: &ModelId,
) -> Result<u32, FactoryError> {
    choice.context_tokens.ok_or_else(|| {
        context_window_error(
            provider,
            model,
            format!("window `{}` has no context limit to apply", choice.name),
        )
    })
}

pub(super) fn derive_window_input(
    choice: &ContextWindowChoice,
    context_tokens: u32,
    max_output_tokens: u32,
    provider: &str,
    model: &ModelId,
) -> Result<u32, FactoryError> {
    let input = match choice.max_input_tokens {
        Some(input) => input,
        None => context_tokens.checked_sub(max_output_tokens).ok_or_else(|| {
            context_window_error(
                provider,
                model,
                format!(
                    "window `{}` has {context_tokens} context tokens, below the {max_output_tokens}-token output ceiling",
                    choice.name
                ),
            )
        })?,
    };
    if input == 0 || input > context_tokens {
        return Err(context_window_error(
            provider,
            model,
            format!(
                "window `{}` has an invalid maximum input limit",
                choice.name
            ),
        ));
    }
    Ok(input)
}

pub(super) fn apply_configured_window(
    limits: &mut smith_config::resolve::ResolvedModelLimits,
    choice: &ContextWindowChoice,
    context_tokens: u32,
    max_input_tokens: u32,
) {
    limits.context_tokens = Some(Sourced::new(context_tokens, choice.source.clone()));
    limits.max_input_tokens = Some(Sourced::new(
        max_input_tokens,
        choice
            .max_input_source
            .clone()
            .unwrap_or_else(|| choice.source.clone()),
    ));
}

pub(super) fn resolve_catalog_profile(
    layers: CatalogLayers,
    limits: &smith_config::resolve::ResolvedModelLimits,
    provider: &str,
    model: &ModelId,
) -> Result<ProfileResolution, FactoryError> {
    layers
        .with_configured_limits(limits)
        .resolve()
        .map_err(|source| FactoryError::ModelProfile {
            provider: provider.to_owned(),
            model: model.clone(),
            source,
        })
}

pub(super) fn context_window_error(
    provider: &str,
    model: &ModelId,
    message: String,
) -> FactoryError {
    FactoryError::ContextWindow {
        provider: provider.to_owned(),
        model: model.clone(),
        message,
    }
}

/// Resolves Smith's request-output policy against one immutable model profile.
pub(super) fn effective_output_budget(
    config: &ResolvedConfig,
    profile: &ResolvedModelProfile,
) -> Result<OutputBudget, FactoryError> {
    resolve_output_budget(
        profile.limits.context_tokens,
        profile.limits.max_output_tokens,
        config.max_output_tokens.as_ref().map(|value| value.value),
        config
            .context
            .output_reserve
            .as_ref()
            .map(|value| value.value),
        config.context.reasoning_reserve.value,
    )
    .map_err(|error| FactoryError::ContextReserve {
        message: format!("model `{}`: {error}", profile.model),
    })
}

/// Derives context planning from configuration, the model's own limits, and
/// the effective output budget.
pub(super) fn context_policy(
    config: &ResolvedConfig,
    limits: &ModelLimits,
    output_budget: &OutputBudget,
) -> ContextPolicy {
    let reasoning_reserve = config.context.reasoning_reserve.value;
    let output_reserve = output_budget.output_reserve;

    let configured = config.context.capability_budget.as_ref().map(|s| s.value);
    let capability_budget = Some(configured.unwrap_or_else(|| {
        derived_capability_budget(
            limits.input_budget(output_reserve.saturating_add(reasoning_reserve)),
        )
    }));
    let max_estimated_slack = config.context.max_estimated_slack.as_ref().map(|s| s.value);
    let mut policy = ContextPolicy::new(
        RegistryRevision::new(policy_revision(
            output_reserve,
            reasoning_reserve,
            capability_budget,
            max_estimated_slack,
        )),
        output_reserve,
        reasoning_reserve,
    );
    if let Some(budget) = capability_budget {
        policy = policy.with_capability_budget(budget);
    }
    if let Some(slack) = max_estimated_slack {
        policy = policy.with_max_estimated_slack(slack);
    }
    policy
}

/// The capability budget for a model whose configuration does not name one.
///
/// A share of the model's own input budget rather than a fixed count: the
/// same absolute number is a comfortable allowance on a million-token window
/// and an instant failure on a small one, and nothing about a run tells the
/// user which they have. The clamps keep both ends sane — a large window does
/// not hand the capability lane more than it can use, and a small one still
/// gets enough to hold the built-in tools. `ContextBudget::from_limits`
/// narrows the result to the input budget again, so a model too small for the
/// floor is bounded by its own window rather than by this.
pub(super) fn derived_capability_budget(input_budget: u32) -> u32 {
    (input_budget / 100)
        .saturating_mul(CAPABILITY_BUDGET_PERCENT)
        .clamp(MIN_DERIVED_CAPABILITY_BUDGET, MAX_DERIVED_CAPABILITY_BUDGET)
}

/// The share of the capability budget skill instructions may take.
///
/// The percentage is what the budget is *for*; the floor is what keeps it
/// usable. A tenth of a small capability budget cannot hold one reference
/// section, which would make those sections permanently unreachable, so the
/// floor lifts the share to where a single section still fits — capped at
/// half the budget, because skills must never be able to crowd the tool
/// schemas out entirely.
pub(super) fn skill_instruction_budget(capability_budget: u32) -> u32 {
    let share = (capability_budget / 100).saturating_mul(SKILL_BUDGET_PERCENT);
    let floor = MIN_SKILL_INSTRUCTION_BUDGET.min(capability_budget / 2);
    share.max(floor)
}

/// Resolves Smith's percentage watermarks against the same enforceable input
/// budget the shared planner uses, then gives the result a revision identity.
pub(super) fn compaction_policy(
    config: &ResolvedConfig,
    profile: &ResolvedModelProfile,
    context_policy: &ContextPolicy,
) -> CompactionPolicy {
    let input_budget = ContextBudget::from_limits(&profile.limits, context_policy).input_budget;
    let high_percent = config.context.compaction_high_watermark_percent.value;
    let low_percent = config.context.compaction_low_watermark_percent.value;
    let high_watermark = percentage_of(input_budget, high_percent);
    let low_watermark = percentage_of(input_budget, low_percent);
    CompactionPolicy::new(
        RegistryRevision::new(format!(
            "{COMPACTION_POLICY_REVISION}/high={high_watermark}/low={low_watermark}"
        )),
        high_watermark,
        low_watermark,
    )
}

/// Floors one configured percentage of a token budget without overflowing the
/// intermediate multiplication.
pub(super) fn percentage_of(tokens: u32, percent: u8) -> u32 {
    let scaled = u64::from(tokens).saturating_mul(u64::from(percent)) / 100;
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// The revision identifying one resolved context policy.
///
/// The reserves are part of the identity, not just the schema version: the
/// revision is what a plan and cache fingerprint carry, so two runs budgeted
/// differently must not present the same one.
pub(super) fn policy_revision(
    output_reserve: u32,
    reasoning_reserve: u32,
    capability_budget: Option<u32>,
    max_estimated_slack: Option<u32>,
) -> String {
    let optional = |value: Option<u32>| match value {
        Some(value) => value.to_string(),
        None => "none".to_owned(),
    };
    format!(
        "{CONTEXT_POLICY_REVISION}/out={output_reserve}/reason={reasoning_reserve}/cap={}/slack={}",
        optional(capability_budget),
        optional(max_estimated_slack)
    )
}

/// Maps the configured prompt and loop limits onto the shared loop.
///
/// Set as one value rather than through the builder's individual setters
/// because the generation cap has no setter of its own, and splitting one
/// coherent loop configuration across two mechanisms is how half of it ends up
/// forgotten.
pub(super) fn loop_config(
    request: &RuntimeRequest,
    model: &ModelId,
    output_budget: &OutputBudget,
) -> LoopConfig {
    let config = &request.config;
    let mut loop_config = LoopConfig::new(model.clone());
    // Smith installs product instructions through `SmithPromptContributor` so
    // every section remains independently positioned, fingerprinted, and
    // budgeted. The legacy field stays empty to prevent a duplicate copy.
    loop_config.system_prompt = None;
    // A configured value of 0 removes the tool-loop ceiling, the same way it
    // removes the wall-clock one below: the limit is optional on the shared
    // loop, and `None` lets a turn run as many steps as the model asks for.
    loop_config.max_tool_steps =
        (config.limits.max_tool_steps.value != 0).then_some(config.limits.max_tool_steps.value);
    loop_config.retry = RetryPolicy {
        // Configuration counts retries *after* the first attempt; the shared
        // policy counts attempts including it.
        max_attempts: config.limits.max_retries.value.saturating_add(1),
        ..RetryPolicy::default()
    };
    // A configured value of 0 removes the wall-clock ceiling: the limit is
    // optional on the shared loop, and `None` leaves a turn unbounded by time.
    loop_config.turn_time_limit_ms = (config.limits.turn_time_limit_ms.value != 0)
        .then_some(config.limits.turn_time_limit_ms.value);
    loop_config.output_limit =
        usize::try_from(config.limits.tool_output_limit_bytes.value).unwrap_or(usize::MAX);
    loop_config.max_output_tokens = Some(output_budget.request_tokens);
    // Explicit rather than inherited: an unsupported capability must fail
    // before network I/O unless a named downgrade was configured, and Smith
    // configuration has no downgrade keys to configure one with yet.
    loop_config.downgrade = DowngradePolicy::strict();
    loop_config
}
