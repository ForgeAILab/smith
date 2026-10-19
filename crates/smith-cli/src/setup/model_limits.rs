use super::{
    ConfigFile, CredentialRef, KIND_OPENAI_COMPATIBLE, ModelSection, ProviderSection,
    ResolveModelLimits, ResolvedModelLimits, SetupContext,
};

/// Resolves explicit fields over the exact bound endpoint/catalog snapshot.
/// Numeric defaults are used only when neither source publishes the field.
pub(super) fn prepared_model_limits(
    context: &SetupContext,
    provider: &str,
    model: &str,
) -> Option<ResolvedModelLimits> {
    let explicit = configured_model_section(context, provider, model);
    let configured = configured_provider_section(context, provider);
    let bound = configured.as_ref().and_then(|section| {
        context.catalog.bound_provider(
            provider,
            section.kind.as_deref()?,
            section.base_url.as_deref(),
        )
    });
    let advertised = bound.and_then(|binding| {
        let entry = binding.provider.models.get(model)?;
        let limits = entry.limits.or_else(|| {
            let partial = binding.partial_limits?.get(model)?;
            let context_tokens = partial.context_tokens.or(explicit.context_tokens)?;
            Some(smith_config::catalog::CatalogLimits {
                context_tokens,
                max_input_tokens: partial.max_input_tokens.unwrap_or(context_tokens),
                max_output_tokens: partial.max_output_tokens.unwrap_or_else(|| {
                    smith_runtime::probe::derived_output_ceiling(context_tokens)
                }),
            })
        })?;
        Some(ResolvedModelLimits {
            context_tokens: limits.context_tokens,
            max_input_tokens: limits.max_input_tokens,
            max_output_tokens: limits.max_output_tokens,
            source: format!(
                "{} · {} old · rev {}{}",
                crate::resources::bounded_text(binding.source, 96),
                crate::resources::catalog_age(binding.retrieved_at_ms),
                crate::resources::bounded_text(binding.revision, 16),
                if entry.limits.is_none() {
                    " · missing limits defaulted"
                } else {
                    ""
                }
            ),
        })
    });
    merge_explicit_limits(context, Some(provider), model, advertised)
}

/// Published partial ceilings survive a missing-context recovery, with each
/// existing explicit field still taking precedence over the advertisement.
pub(super) fn known_model_ceilings(
    context: &SetupContext,
    provider: &str,
    model: &str,
) -> (Option<u32>, Option<u32>) {
    let explicit = configured_model_section(context, provider, model);
    let partial = configured_provider_section(context, provider).and_then(|section| {
        context
            .catalog
            .bound_provider(
                provider,
                section.kind.as_deref()?,
                section.base_url.as_deref(),
            )
            .and_then(|binding| binding.partial_limits?.get(model).copied())
    });
    (
        explicit
            .max_input_tokens
            .or(partial.and_then(|limits| limits.max_input_tokens)),
        explicit
            .max_output_tokens
            .or(partial.and_then(|limits| limits.max_output_tokens)),
    )
}

/// Shares the provider discovery snapshot with typed-ID recovery. Add-model
/// never fetches the listing a second time after its initial discovery.
pub(super) async fn resolve_model_limits(
    context: &SetupContext,
    request: &ResolveModelLimits,
) -> Option<ResolvedModelLimits> {
    if let Some(provider) = request.provider.as_deref()
        && context.discovery_attempted.contains(provider)
    {
        return prepared_model_limits(context, provider, &request.model);
    }
    let (endpoint, bearer) = if !request.use_endpoint_listing {
        (None, None)
    } else {
        match request.endpoint.as_deref() {
            Some(endpoint) => (
                Some(endpoint.to_owned()),
                request
                    .bearer
                    .as_ref()
                    .map(|secret| secret.expose().to_owned())
                    .or_else(|| read_environment_bearer(request.environment_variable.as_deref())),
            ),
            None => configured_probe_target(context, request.provider.as_deref()?).await,
        }
    };
    if let Some(endpoint) = endpoint
        && let Ok(Some(limits)) = smith_runtime::probe::openai_compatible_model_limits(
            &endpoint,
            bearer.as_deref(),
            &request.model,
        )
        .await
    {
        let explicit = request
            .provider
            .as_deref()
            .map(|provider| configured_model_section(context, provider, &request.model))
            .unwrap_or_default();
        if let Some(context_tokens) = explicit.context_tokens.or(limits.context_tokens) {
            let missing = limits.max_input_tokens.is_none() || limits.max_output_tokens.is_none();
            let source = if missing {
                "endpoint /models listing · missing ceilings defaulted"
            } else {
                "endpoint /models listing"
            };
            return merge_explicit_limits(
                context,
                request.provider.as_deref(),
                &request.model,
                Some(ResolvedModelLimits {
                    context_tokens,
                    max_input_tokens: limits.max_input_tokens.unwrap_or(context_tokens),
                    max_output_tokens: limits.max_output_tokens.unwrap_or_else(|| {
                        smith_runtime::probe::derived_output_ceiling(context_tokens)
                    }),
                    source: source.to_owned(),
                }),
            );
        }
    }
    if let Some(provider) = request.provider.as_deref()
        && let Some(limits) = prepared_model_limits(context, provider, &request.model)
    {
        return Some(limits);
    }
    legacy_catalog_suggestion(context, &request.model)
}

/// Kept only for the existing direct typed-ID flow. A same-name model on
/// another deployment is a labelled review suggestion, never discovery data.
fn legacy_catalog_suggestion(context: &SetupContext, model: &str) -> Option<ResolvedModelLimits> {
    let (provider, model) = context.catalog.resolve_model_by_id(model)?;
    let limits = model.limits?;
    Some(ResolvedModelLimits {
        context_tokens: limits.context_tokens,
        max_input_tokens: limits.max_input_tokens,
        max_output_tokens: limits.max_output_tokens,
        source: format!("trusted catalog match {provider}/{}", model.id),
    })
}

fn merge_explicit_limits(
    context: &SetupContext,
    provider: Option<&str>,
    model: &str,
    detected: Option<ResolvedModelLimits>,
) -> Option<ResolvedModelLimits> {
    let explicit = provider
        .map(|provider| configured_model_section(context, provider, model))
        .unwrap_or_default();
    let context_tokens = explicit
        .context_tokens
        .or_else(|| detected.as_ref().map(|limits| limits.context_tokens))?;
    let explicit_fields = explicit.context_tokens.is_some()
        || explicit.max_input_tokens.is_some()
        || explicit.max_output_tokens.is_some();
    let source = match (detected.as_ref(), explicit_fields) {
        (Some(limits), true) => format!("{} · existing explicit fields preserved", limits.source),
        (Some(limits), false) => limits.source.clone(),
        (None, _) => "existing explicit model values · missing ceilings defaulted".to_owned(),
    };
    Some(ResolvedModelLimits {
        context_tokens,
        max_input_tokens: explicit
            .max_input_tokens
            .or_else(|| detected.as_ref().map(|limits| limits.max_input_tokens))
            .unwrap_or(context_tokens),
        max_output_tokens: explicit
            .max_output_tokens
            .or_else(|| detected.as_ref().map(|limits| limits.max_output_tokens))
            .unwrap_or_else(|| smith_runtime::probe::derived_output_ceiling(context_tokens)),
        source,
    })
}

/// Reads only explicit limit fields; defaults derived by runtime inventory do
/// not become overwrites of an already configured model ceiling.
pub(super) fn configured_model_section(
    context: &SetupContext,
    provider: &str,
    model: &str,
) -> ModelSection {
    let key = format!("{provider}/{model}");
    let mut limits = ModelSection::default();
    for path in config_layer_paths(context) {
        if let Some(section) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| ConfigFile::parse(&text).ok())
            .and_then(|file| file.models.get(&key).cloned())
        {
            limits.context_tokens = section.context_tokens.or(limits.context_tokens);
            limits.max_input_tokens = section.max_input_tokens.or(limits.max_input_tokens);
            limits.max_output_tokens = section.max_output_tokens.or(limits.max_output_tokens);
        }
    }
    if let Some(entry) = context
        .inventory
        .models
        .iter()
        .find(|entry| entry.provider == provider && entry.model == model)
    {
        use smith_config::inventory::{InventoryLimit, ModelLimitOrigin};
        let configured = |limit: &Option<InventoryLimit>, field: &str| {
            let limit = limit.as_ref()?;
            let ModelLimitOrigin::Configured(source) = &limit.origin else {
                return None;
            };
            // A derived input ceiling can share its context field's origin;
            // only an actual explicit model leaf may replace this leaf.
            (source.key.starts_with("models.")
                && source.key.ends_with(&format!(".{field}"))
                && source.layer != smith_config::resolve::Layer::BuiltIn)
                .then_some(limit.value)
        };
        limits.context_tokens =
            configured(&entry.context_tokens, "context_tokens").or(limits.context_tokens);
        limits.max_input_tokens =
            configured(&entry.max_input_tokens, "max_input_tokens").or(limits.max_input_tokens);
        limits.max_output_tokens =
            configured(&entry.max_output_tokens, "max_output_tokens").or(limits.max_output_tokens);
    }
    limits
}

/// Files in runtime precedence order, with field-level overlay at each layer.
fn config_layer_paths(context: &SetupContext) -> [std::path::PathBuf; 3] {
    [
        context.user_dir.join("config.toml"),
        context.project.join(".smith/config.toml"),
        context.project.join(".smith/config.local.toml"),
    ]
}

/// Reads a bearer from a chosen environment variable, when it is exported.
fn read_environment_bearer(variable: Option<&str>) -> Option<String> {
    std::env::var(variable?)
        .ok()
        .filter(|value| !value.is_empty())
}

/// Reads one provider's effective section from the local config layers.
///
/// The resolved run configuration keeps only the active provider, so the
/// owner and project files are read in the runtime's precedence order, with
/// field-level overlay preserving lower-layer values omitted by a later file.
pub(super) fn configured_provider_section(
    context: &SetupContext,
    provider: &str,
) -> Option<ProviderSection> {
    let mut section = ProviderSection::default();
    let mut found_provider = false;
    for path in config_layer_paths(context) {
        let file = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| ConfigFile::parse(&text).ok());
        if let Some(found) = file.and_then(|file| {
            file.providers
                .into_iter()
                .find(|(name, _)| name == provider)
                .map(|(_, section)| section)
        }) {
            found_provider = true;
            section.kind = found.kind.or(section.kind);
            section.base_url = found.base_url.or(section.base_url);
            section.api_key = found.api_key.or(section.api_key);
            if found.credential.is_some() {
                section.credential = found.credential;
            }
            if !found.credentials.is_empty() {
                section.credentials = found.credentials;
            }
            section.rotate_at_percent = found.rotate_at_percent.or(section.rotate_at_percent);
            section.command = found.command.or(section.command);
            section.headers.extend(found.headers);
            if let Some(response) = found.response {
                let target = section.response.get_or_insert_default();
                target.reasoning_only = response.reasoning_only.or(target.reasoning_only);
            }
        }
    }
    found_provider.then_some(section)
}

/// Resolves a configured provider's OpenAI-compatible endpoint and bearer.
///
/// Add-model flows reach models through a provider that already exists, so
/// the probe target comes from its stored section rather than fresh input.
pub(super) async fn configured_probe_target(
    context: &SetupContext,
    provider: &str,
) -> (Option<String>, Option<String>) {
    let Some(section) = configured_provider_section(context, provider) else {
        return (None, None);
    };
    if section.kind.as_deref() != Some(KIND_OPENAI_COMPATIBLE) {
        // Only the OpenAI-compatible listing is probed: other adapters speak
        // different listing protocols Smith has not reviewed.
        return (None, None);
    }
    let endpoint = section.base_url.clone();
    let reference = section
        .credential
        .clone()
        .or_else(|| section.credentials.first().cloned())
        .and_then(|value| CredentialRef::parse(&value).ok());
    let bearer = match (section.api_key, reference) {
        (Some(key), _) => Some(key.to_secret().expose().to_owned()),
        (None, Some(CredentialRef::Env { variable })) => read_environment_bearer(Some(&variable)),
        (
            None,
            Some(reference @ (CredentialRef::Keychain { .. } | CredentialRef::AuthFile { .. })),
        ) => {
            let resolver = smith_config::credential::CredentialResolver::new(&context.user_dir);
            tokio::task::spawn_blocking(move || resolver.resolve_blocking(&reference))
                .await
                .ok()
                .and_then(|resolved| resolved.ok())
                .map(|secret| secret.expose().to_owned())
        }
        _ => None,
    };
    (endpoint, bearer)
}
