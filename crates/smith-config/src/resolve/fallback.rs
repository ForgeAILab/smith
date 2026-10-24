//! Profile-scoped ordered model bindings, resolved without acquiring secrets.

use std::collections::BTreeSet;

use super::load::Declarations;
use super::provenance::*;
use super::types::*;
use crate::model::FallbackModelSection;

fn invalid(source: &Source, message: impl Into<String>) -> ConfigError {
    ConfigError::InvalidValue {
        source: source.clone(),
        message: message.into(),
    }
}

fn timeout(value: u64, source: Source) -> Result<Sourced<u64>, ConfigError> {
    if value == 0 || value > i64::MAX as u64 {
        return Err(invalid(
            &source,
            "first_response_timeout_ms must be a positive representable millisecond count",
        ));
    }
    Ok(Sourced::new(value, source))
}

pub(super) fn resolve(
    provenance: &Provenance,
    primary: &ResolvedConfig,
    declared: &Declarations,
    request: &ResolveRequest,
) -> Result<ResolvedModelFallback, ConfigError> {
    let common_timeout = provenance
        .winner("fallback.first_response_timeout_ms")
        .map(|entry| match entry.value {
            SettingValue::Integer(value) if value > 0 => {
                timeout(value as u64, entry.source.clone())
            }
            _ => Err(invalid(
                &entry.source,
                "first_response_timeout_ms must be positive",
            )),
        })
        .transpose()?;
    let Some(entry) = provenance.winner("fallback.models") else {
        return Ok(ResolvedModelFallback {
            inactive_reason: common_timeout.map(|_| "fallback.models is not configured".to_owned()),
            ..ResolvedModelFallback::default()
        });
    };
    let SettingValue::Text(json) = &entry.value else {
        return Err(invalid(
            &entry.source,
            "fallback.models must be an ordered list of provider/model entries",
        ));
    };
    let models: Vec<FallbackModelSection> = serde_json::from_str(json).map_err(|error| {
        invalid(
            &entry.source,
            format!("invalid fallback model list: {error}"),
        )
    })?;
    let exact = request.cli.provider.is_some()
        || request.cli.model.is_some()
        || request.session.provider.is_some()
        || request.session.model.is_some();
    if models.is_empty() || exact || request.advisor_route.is_some() {
        return Ok(ResolvedModelFallback {
            inactive_reason: Some(
                if exact {
                    "disabled by an explicit provider/model selection"
                } else if request.advisor_route.is_some() {
                    "auxiliary model requests do not inherit profile fallback"
                } else {
                    "fallback.models is empty"
                }
                .to_owned(),
            ),
            ..ResolvedModelFallback::default()
        });
    }
    if primary.model.value.starts_with("cli/") {
        return Err(invalid(
            &entry.source,
            "installed-agent harnesses do not support model fallback",
        ));
    }
    let mut seen = BTreeSet::from([(
        primary.provider.name.value.clone(),
        primary.model.value.clone(),
    )]);
    let mut resolved = Vec::with_capacity(models.len());
    for (index, model) in models.into_iter().enumerate() {
        let source = Source {
            key: format!("{}[{index}]", entry.source.key),
            ..entry.source.clone()
        };
        if model.provider.trim().is_empty() || model.model.trim().is_empty() {
            return Err(invalid(
                &source,
                "fallback provider and model must not be empty",
            ));
        }
        if model.model.starts_with("cli/") {
            return Err(invalid(
                &source,
                "fallback entries must select provider-backed models, not installed-agent harnesses",
            ));
        }
        if !declared.providers.contains_key(&model.provider) {
            return Err(invalid(
                &source,
                format!("fallback provider `{}` is not declared", model.provider),
            ));
        }
        if !seen.insert((model.provider.clone(), model.model.clone())) {
            return Err(invalid(
                &source,
                format!(
                    "duplicate fallback binding `{}/{}`",
                    model.provider, model.model
                ),
            ));
        }
        let first_response_timeout_ms = model
            .first_response_timeout_ms
            .map(|value| {
                timeout(
                    value,
                    Source {
                        key: format!("{}.first_response_timeout_ms", source.key),
                        ..source.clone()
                    },
                )
            })
            .transpose()?
            .or_else(|| common_timeout.clone());
        let mut candidate = provenance.clone();
        candidate.remove("fallback.models");
        candidate.remove("fallback.first_response_timeout_ms");
        if model.reasoning.is_some() {
            for key in ["reasoning.enabled", "reasoning.effort"] {
                if candidate
                    .source(key)
                    .is_some_and(|source| source.layer < Layer::Environment)
                {
                    candidate.remove(key);
                }
            }
        }
        if candidate
            .source("context_window")
            .is_some_and(|source| source.layer < Layer::Environment)
        {
            candidate.remove("context_window");
        }
        let mut overrides = vec![
            Contribution {
                key: "provider".into(),
                value: SettingValue::Text(model.provider),
                source: Source {
                    key: format!("{}.provider", source.key),
                    ..source.clone()
                },
            },
            Contribution {
                key: "model".into(),
                value: SettingValue::Text(model.model),
                source: Source {
                    key: format!("{}.model", source.key),
                    ..source.clone()
                },
            },
        ];
        let mut control = |key: &str, value: Option<SettingValue>| {
            if let Some(value) = value
                && provenance
                    .source(key)
                    .is_none_or(|source| source.layer < Layer::Environment)
            {
                overrides.push(Contribution {
                    key: key.to_owned(),
                    value,
                    source: Source {
                        key: format!("{}.{key}", source.key),
                        ..source.clone()
                    },
                });
            }
        };
        control(
            "context_window",
            model.context_window.map(SettingValue::Text),
        );
        control(
            "max_output_tokens",
            model
                .max_output_tokens
                .map(|value| SettingValue::Integer(i64::from(value))),
        );
        if let Some(reasoning) = model.reasoning {
            control(
                "reasoning.enabled",
                reasoning.enabled.map(SettingValue::Flag),
            );
            control("reasoning.effort", reasoning.effort.map(SettingValue::Text));
        }
        candidate.extend(overrides);
        super::load::apply_product_model_defaults(&mut candidate);
        super::load::apply_image_generation_defaults(&mut candidate);
        super::vision::explain_model_input(&mut candidate, request.catalog.as_deref())?;
        super::modules::legacy_defaults(&mut candidate, &request.known_modules);
        super::modules::mirror_aliases(&mut candidate, &request.known_modules);
        let config = super::agent::extract(
            &candidate,
            primary.user_dir.clone(),
            primary.profile.clone(),
            declared,
            primary.agent.profiles.clone(),
            request,
        )?;
        resolved.push(ResolvedFallbackModel {
            config: Box::new(config),
            first_response_timeout_ms,
            source,
        });
    }
    Ok(ResolvedModelFallback {
        models: resolved,
        first_response_timeout_ms: common_timeout,
        inactive_reason: None,
    })
}

pub(super) fn explain(provenance: &mut Provenance, fallback: &ResolvedModelFallback) {
    let Some(source) = provenance
        .source("fallback.models")
        .or_else(|| provenance.source("fallback.first_response_timeout_ms"))
        .cloned()
    else {
        return;
    };
    let mut contributions = vec![Contribution {
        key: "fallback.active".to_owned(),
        value: SettingValue::Flag(!fallback.models.is_empty()),
        source: source.clone(),
    }];
    if let Some(reason) = &fallback.inactive_reason {
        contributions.push(Contribution {
            key: "fallback.inactive_reason".to_owned(),
            value: SettingValue::Text(reason.clone()),
            source: source.clone(),
        });
    }
    if !fallback.models.is_empty() {
        contributions.push(Contribution {
            key: "fallback.context_disclosure".to_owned(),
            value: SettingValue::Text(
                "Model fallback resends conversation context to each selected configured provider"
                    .into(),
            ),
            source,
        });
    }
    provenance.extend(contributions);
}
