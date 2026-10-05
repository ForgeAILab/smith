use super::*;

pub(in crate::resolve) fn resolve_model_limits(
    provenance: &Provenance,
    provider: &str,
    model: &str,
) -> Result<ResolvedModelLimits, ConfigError> {
    let scope = join_key(&["models", &format!("{provider}/{model}")]);
    let context_tokens = optional_u32(provenance, &format!("{scope}.context_tokens"))?;
    let max_input_tokens = optional_u32(provenance, &format!("{scope}.max_input_tokens"))?;
    let max_output_tokens = optional_u32(provenance, &format!("{scope}.max_output_tokens"))?;
    let flat_context_source = [context_tokens.as_ref(), max_input_tokens.as_ref()]
        .into_iter()
        .flatten()
        .max_by_key(|limit| limit.source.layer.precedence())
        .map(|limit| limit.source.clone());
    let declared = ResolvedModelLimits {
        context_tokens,
        max_input_tokens,
        max_output_tokens,
        context_windows: resolve_context_windows(provenance, &scope)?,
        default_context_window: text(provenance, &format!("{scope}.default_context_window"))?,
        flat_context_source,
    };
    if crate::cli_agents::parse_cli_model_id(model).is_none() {
        return Ok(declared);
    }
    // An installed agent owns its own context, so Smith has no limits to
    // discover and nothing to enforce with them. Supply defaults rather than
    // demand a `[models]` table for a number that changes nothing, while
    // still letting an owner override them.
    let built_in = |key: &str, fallback: u32| {
        Sourced::new(fallback, Source::built_in(format!("{scope}.{key}")))
    };
    Ok(ResolvedModelLimits {
        context_tokens: declared.context_tokens.or_else(|| {
            Some(built_in(
                "context_tokens",
                crate::cli_agents::CLI_AGENT_CONTEXT_TOKENS,
            ))
        }),
        max_input_tokens: declared.max_input_tokens.or_else(|| {
            Some(built_in(
                "max_input_tokens",
                crate::cli_agents::CLI_AGENT_MAX_INPUT_TOKENS,
            ))
        }),
        max_output_tokens: declared.max_output_tokens.or_else(|| {
            Some(built_in(
                "max_output_tokens",
                crate::cli_agents::CLI_AGENT_MAX_OUTPUT_TOKENS,
            ))
        }),
        context_windows: declared.context_windows,
        default_context_window: declared.default_context_window,
        flat_context_source: declared.flat_context_source,
    })
}

fn resolve_context_windows(
    provenance: &Provenance,
    model_scope: &str,
) -> Result<BTreeMap<String, ResolvedContextWindow>, ConfigError> {
    let prefix = format!("{model_scope}.context_windows.");
    let names = provenance
        .keys_with_prefix(&prefix)
        .filter_map(|key| {
            key.strip_prefix(&prefix)?
                .split_once('.')
                .map(|(name, _)| name)
        })
        .collect::<BTreeSet<_>>();
    let mut windows = BTreeMap::new();
    for name in names {
        let context_key = format!("{prefix}{name}.context_tokens");
        let context_tokens =
            optional_u32(provenance, &context_key)?.ok_or_else(|| ConfigError::MissingSetting {
                key: context_key.clone(),
                message: "every named context window needs `context_tokens`".to_owned(),
            })?;
        let max_input_tokens =
            optional_u32(provenance, &format!("{prefix}{name}.max_input_tokens"))?;
        windows.insert(
            name.to_owned(),
            ResolvedContextWindow {
                context_tokens,
                max_input_tokens,
            },
        );
    }
    Ok(windows)
}

pub(in crate::resolve) fn resolve_model_reasoning(
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

pub(in crate::resolve) fn resolve_context(
    provenance: &Provenance,
    model_limits: &ResolvedModelLimits,
    synthetic_cache_spend: SyntheticCacheSpendAuthority,
) -> Result<ResolvedContext, ConfigError> {
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
    let cache = resolve_cache_policy(provenance, model_limits, synthetic_cache_spend)?;
    // Keep the old resolved field as a compatibility projection.  It points
    // at the exact same sourced winner as the replacement field, so no second
    // inactivity timer can be constructed downstream.
    let idle_compaction_ms = cache.inactivity_limit_ms.clone();
    Ok(ResolvedContext {
        tool_output_inline_bytes: bounded_u32(
            provenance,
            "context.tool_output_inline_bytes",
            256,
            1024 * 1024,
        )?,
        output_reserve: optional_u32(provenance, "context.output_reserve")?,
        reasoning_reserve: required_u32(provenance, "context.reasoning_reserve")?,
        capability_budget: optional_u32(provenance, "context.capability_budget")?,
        max_estimated_slack: optional_u32(provenance, "context.max_estimated_slack")?,
        compaction_high_watermark_percent: high,
        compaction_low_watermark_percent: low,
        idle_compaction_ms,
        cache,
    })
}

/// Resolves the bounded parent wait policy. Missing values are deliberately
/// filled with built-in sources so a profile that predates the new table stays
/// usable during the migration window.
pub(in crate::resolve) fn resolve_child_agents(
    provenance: &Provenance,
) -> Result<ResolvedChildAgents, ConfigError> {
    let default_timeout = bounded_u64_or_default(
        provenance,
        "child_agents.wait_default_timeout_ms",
        0,
        300_000,
        300_000,
    )?;
    let max_timeout = bounded_u64_or_default(
        provenance,
        "child_agents.wait_max_timeout_ms",
        1,
        300_000,
        300_000,
    )?;
    if default_timeout.value > max_timeout.value {
        return Err(ConfigError::InvalidValue {
            source: default_timeout.source,
            message: format!(
                "child_agents.wait_default_timeout_ms ({}) must not exceed wait_max_timeout_ms ({})",
                default_timeout.value, max_timeout.value
            ),
        });
    }
    Ok(ResolvedChildAgents {
        wait_default_timeout_ms: default_timeout,
        wait_max_timeout_ms: max_timeout,
    })
}

pub(in crate::resolve) fn resolve_limits(
    provenance: &Provenance,
) -> Result<ResolvedLimits, ConfigError> {
    Ok(ResolvedLimits {
        max_retries: required_u32(provenance, "limits.max_retries")?,
        max_tool_steps: required_u32(provenance, "limits.max_tool_steps")?,
        turn_time_limit_ms: required_u64(provenance, "limits.turn_time_limit_ms")?,
        tool_output_limit_bytes: required_u64(provenance, "limits.tool_output_limit_bytes")?,
    })
}

pub(in crate::resolve) fn resolve_persistence(
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

pub(in crate::resolve) fn resolve_approval(
    provenance: &Provenance,
) -> Result<ResolvedApproval, ConfigError> {
    let raw = required_text(provenance, "approval.mode")?;
    let mode = ApprovalMode::parse(&raw.value).ok_or_else(|| ConfigError::InvalidValue {
        source: raw.source.clone(),
        message: format!(
            "`{}` is not an approval mode; the modes are {}",
            raw.value,
            list_spellings(ApprovalMode::spellings())
        ),
    })?;
    let legacy_auto_approve = list(provenance, "approval.auto_approve")?;
    if let Some(legacy) = &legacy_auto_approve
        && !legacy.value.is_empty()
    {
        return Err(ConfigError::InvalidValue {
            source: legacy.source.clone(),
            message: "non-empty `approval.auto_approve` tool-name lists are no longer supported; migrate to versioned `[[approval.auto]]` prepared-call rules, or use user-owned `approval.mode = \"allow-all\"` only for deliberately unrestricted automation"
                .to_owned(),
        });
    }
    let auto = text(provenance, "approval.auto")?
        .map(resolve_auto_approval_rules)
        .transpose()?
        .unwrap_or_default();
    Ok(ResolvedApproval {
        mode: Sourced::new(mode, raw.source),
        auto_approve: legacy_auto_approve,
        auto,
    })
}

fn resolve_auto_approval_rules(
    encoded: Sourced<String>,
) -> Result<Vec<Sourced<AutoApprovalRule>>, ConfigError> {
    let rules: Vec<AutoApprovalRuleSection> =
        serde_json::from_str(&encoded.value).map_err(|error| ConfigError::InvalidValue {
            source: encoded.source.clone(),
            message: format!("`approval.auto` is not a valid rule list: {error}"),
        })?;
    rules
        .into_iter()
        .enumerate()
        .map(|(index, rule)| {
            let mut source = encoded.source.clone();
            source.key = format!("{}[{index}]", source.key);
            validate_auto_approval_rule(rule, source)
        })
        .collect()
}

fn validate_auto_approval_rule(
    rule: AutoApprovalRuleSection,
    source: Source,
) -> Result<Sourced<AutoApprovalRule>, ConfigError> {
    let invalid = |message: String| ConfigError::InvalidValue {
        source: source.clone(),
        message,
    };
    if rule.revision != 1 {
        return Err(invalid(format!(
            "unsupported automatic approval rule revision {}; only revision 1 is defined",
            rule.revision
        )));
    }
    let Some((module, tool)) = rule.tool.split_once('/') else {
        return Err(invalid(
            "automatic approval rule `tool` must be module-qualified, for example `smith/edit`"
                .to_owned(),
        ));
    };
    if module.is_empty() || tool.is_empty() || tool.contains('/') {
        return Err(invalid(
            "automatic approval rule `tool` must contain exactly one non-empty `/` separator"
                .to_owned(),
        ));
    }
    if rule.tool != "smith/edit" {
        return Err(invalid(format!(
            "automatic approval for `{}` is not supported; revision 1 permits only `smith/edit`",
            rule.tool
        )));
    }
    if rule.operations.is_empty() {
        return Err(invalid(
            "automatic approval rule `operations` cannot be empty".to_owned(),
        ));
    }
    if rule.permissions.is_empty() {
        return Err(invalid(
            "automatic approval rule `permissions` cannot be empty".to_owned(),
        ));
    }
    if rule.paths.is_empty() {
        return Err(invalid(
            "automatic approval rule `paths` cannot be empty".to_owned(),
        ));
    }
    for pattern in &rule.paths {
        let path = Path::new(pattern);
        if pattern.is_empty()
            || path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(invalid(format!(
                "automatic approval path pattern `{pattern}` must be project-relative and cannot contain `..`"
            )));
        }
        globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|error| {
                invalid(format!(
                    "automatic approval path pattern `{pattern}` is invalid: {error}"
                ))
            })?;
    }
    if rule.max_uses == Some(0) {
        return Err(invalid(
            "automatic approval rule `max_uses` must be greater than zero".to_owned(),
        ));
    }
    let expires_at_unix_ms = rule
        .expires_at
        .as_deref()
        .map(|value| {
            time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
                .map(|timestamp| timestamp.unix_timestamp_nanos() / 1_000_000)
                .map_err(|error| {
                    invalid(format!(
                        "automatic approval rule `expires_at` must be RFC 3339: {error}"
                    ))
                })
        })
        .transpose()?;

    Ok(Sourced::new(
        AutoApprovalRule {
            revision: rule.revision,
            tool: rule.tool,
            operations: rule.operations,
            permissions: rule.permissions,
            max_risk: rule.max_risk,
            mount: rule.mount,
            paths: rule.paths,
            expires_at_unix_ms,
            max_uses: rule.max_uses,
        },
        source,
    ))
}

pub(in crate::resolve) fn resolve_background(
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
