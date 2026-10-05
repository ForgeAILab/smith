use super::{
    ConfigFile, CredentialRef, KIND_OPENAI_COMPATIBLE, ProviderSection, ResolveModelLimits,
    ResolvedModelLimits, SetupContext,
};

/// Runs the bounded automatic limit resolution for one entered model.
///
/// The endpoint's own listing wins when it publishes a context window — the
/// server that will enforce the limit is the most authoritative source for
/// it — with the frozen trusted catalog as the same-name fallback. Either
/// way the input and output ceilings default from the context exactly as the
/// single manual context fallback does, and every failure is simply "nothing
/// resolved".
pub(super) async fn resolve_model_limits(
    context: &SetupContext,
    request: &ResolveModelLimits,
) -> Option<ResolvedModelLimits> {
    let (endpoint, bearer) = if !request.use_endpoint_listing {
        // Native Messages authentication/listing differs from the reviewed
        // OpenAI listing. Reuse catalog resolution and the manual fallback.
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
    if let Some(endpoint) = endpoint {
        let probe = smith_runtime::probe::openai_compatible_model_limits(
            &endpoint,
            bearer.as_deref(),
            &request.model,
        )
        .await;
        if let Ok(Some(limits)) = probe
            && let Some(context_tokens) = limits.context_tokens
        {
            let max_input_tokens = limits.max_input_tokens.unwrap_or(context_tokens);
            let max_output_tokens = limits
                .max_output_tokens
                .unwrap_or_else(|| smith_runtime::probe::derived_output_ceiling(context_tokens));
            let mut source = "endpoint /models listing".to_owned();
            if limits.max_input_tokens.is_none() || limits.max_output_tokens.is_none() {
                source.push_str(" · missing ceilings defaulted");
            }
            return Some(ResolvedModelLimits {
                context_tokens,
                max_input_tokens,
                max_output_tokens,
                source,
            });
        }
    }
    let (provider, model) = context.catalog.resolve_model_by_id(&request.model)?;
    let limits = model.limits?;
    Some(ResolvedModelLimits {
        context_tokens: limits.context_tokens,
        max_input_tokens: limits.max_input_tokens,
        max_output_tokens: limits.max_output_tokens,
        source: format!("trusted catalog match {provider}/{}", model.id),
    })
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
/// user and project files are read directly — the same layered pair setup
/// itself reads and writes, with the user layer winning on a shared name.
pub(super) fn configured_provider_section(
    context: &SetupContext,
    provider: &str,
) -> Option<ProviderSection> {
    let mut section = None;
    for path in [
        context.project.join(".smith/config.toml"),
        context.user_dir.join("config.toml"),
    ] {
        let file = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| ConfigFile::parse(&text).ok());
        if let Some(found) = file.and_then(|file| {
            file.providers
                .into_iter()
                .find(|(name, _)| name == provider)
                .map(|(_, section)| section)
        }) {
            section = Some(found);
        }
    }
    section
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
    let bearer = match reference {
        Some(CredentialRef::Env { variable }) => read_environment_bearer(Some(&variable)),
        Some(reference @ (CredentialRef::Keychain { .. } | CredentialRef::AuthFile { .. })) => {
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
