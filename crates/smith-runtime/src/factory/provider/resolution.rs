//! Ordered provider, credential, profile, and context-policy resolution.

use super::*;

use super::super::context_policy;
use super::adapter;
use super::credentials;

pub(in crate::factory) async fn prepare(
    request: &RuntimeRequest,
) -> Result<PreparedFactoryInputs, FactoryError> {
    let config = &request.config;
    let provider_name = config.provider.name.value.clone();
    let provider_kind = config.provider.kind.value.clone();
    let model = ModelId::new(config.model.value.clone());

    // An adapter this build does not ship is never routed through a different
    // wire protocol.
    let adapter = adapter::adapter(&config.provider)?;
    let endpoint = match adapter {
        Adapter::OpenAiCompatible => Some(adapter::endpoint(&config.provider, None)?),
        // Generic over the Responses protocol, so there is no default to fall
        // back to: the endpoint names which deployment is being talked to.
        Adapter::OpenAiResponses => Some(adapter::endpoint(&config.provider, None)?),
        Adapter::AnthropicMessages => Some(adapter::endpoint(
            &config.provider,
            Some(smith_config::model::ANTHROPIC_DEFAULT_ENDPOINT),
        )?),
        Adapter::ChatGptResponses => Some(adapter::endpoint(
            &config.provider,
            Some(smith_config::setup::CHATGPT_ENDPOINT),
        )?),
        Adapter::XaiResponses => Some(adapter::endpoint(
            &config.provider,
            Some(smith_config::setup::XAI_ENDPOINT),
        )?),
        Adapter::GeminiInteractions => Some(smith_config::catalog::GEMINI_ENDPOINT.to_owned()),
        Adapter::CommandJsonl | Adapter::Fake => None,
    };

    // A bad choice is a local configuration error. Resolve it before opening
    // credentials or contacting a command/provider adapter.
    let context_window = context_policy::resolve_context_window_selection(
        config,
        &provider_kind,
        endpoint.as_deref(),
        model.as_str(),
    )?;

    // Resolve and canonicalize executable authority before any credential
    // lookup. Environment values cross the secret boundary only after model
    // and capability validation below.
    let static_command = if adapter == Adapter::CommandJsonl {
        Some(adapter::prepare_static_command_provider(
            request,
            model.as_str(),
        )?)
    } else {
        None
    };

    let mut layers = CatalogLayers::new(provider_name.clone(), model.clone())
        .with_sources(request.catalog_sources.iter().map(Arc::clone));
    if let Some(command) = &static_command {
        let descriptor = command
            .adapter
            .describe()
            .into_iter()
            .next()
            .expect("a validated command adapter declares its selected model");
        let mut record = ModelRecord::new().with_capabilities(descriptor.capabilities);
        record.input_modalities = Some(vec![Modality::Text]);
        record.output_modalities = Some(vec![Modality::Text]);
        record.revision = Some("smith-command-provider-1".to_owned());
        layers = layers.with_provider_local(record);
    }
    let trusted = trusted_model(&provider_name, model.as_str());
    let active_choice = context_window.active.as_ref();
    let selected_trusted = active_choice
        .filter(|choice| choice.origin == context_policy::ContextWindowOrigin::Trusted);
    if let Some(trusted) = trusted
        && selected_trusted.is_none()
    {
        layers = layers.with_embedded(ModelRecord::new().with_limits(ModelLimits::new(
            trusted.context_tokens,
            trusted.max_input_tokens,
            trusted.max_output_tokens,
        )));
    }

    let mut profile = match active_choice {
        Some(choice) if choice.origin == context_policy::ContextWindowOrigin::Config => {
            let context_tokens =
                context_policy::required_window_context(choice, &provider_name, &model)?;
            let mut selected_limits = config.model_limits.clone();
            let provisional_input = choice.max_input_tokens.unwrap_or(context_tokens);
            context_policy::apply_configured_window(
                &mut selected_limits,
                choice,
                context_tokens,
                provisional_input,
            );
            let provisional = context_policy::resolve_catalog_profile(
                layers.clone(),
                &selected_limits,
                &provider_name,
                &model,
            )?;
            let max_output = provisional.profile.limits.max_output_tokens;
            let input = context_policy::derive_window_input(
                choice,
                context_tokens,
                max_output,
                &provider_name,
                &model,
            )?;
            context_policy::apply_configured_window(
                &mut selected_limits,
                choice,
                context_tokens,
                input,
            );
            context_policy::resolve_catalog_profile(
                layers,
                &selected_limits,
                &provider_name,
                &model,
            )?
        }
        Some(choice) if choice.origin == context_policy::ContextWindowOrigin::Trusted => {
            let trusted = trusted.expect("trusted context windows come from a trusted model");
            let context_tokens =
                context_policy::required_window_context(choice, &provider_name, &model)?;
            let input = choice
                .max_input_tokens
                .unwrap_or_else(|| context_tokens.saturating_sub(trusted.max_output_tokens));
            if input == 0 || input > context_tokens {
                return Err(context_policy::context_window_error(
                    &provider_name,
                    &model,
                    format!(
                        "trusted window `{}` has inconsistent input and output limits",
                        choice.name
                    ),
                ));
            }
            layers = layers.with_embedded(ModelRecord::new().with_limits(ModelLimits::new(
                context_tokens,
                input,
                trusted.max_output_tokens,
            )));
            context_policy::resolve_catalog_profile(
                layers,
                &config.model_limits,
                &provider_name,
                &model,
            )?
        }
        Some(choice)
            if choice.origin == context_policy::ContextWindowOrigin::Endpoint
                && choice.context_tokens.is_some() =>
        {
            let context_tokens =
                context_policy::required_window_context(choice, &provider_name, &model)?;
            let baseline = context_policy::resolve_catalog_profile(
                layers.clone(),
                &config.model_limits,
                &provider_name,
                &model,
            )?;
            let input = context_policy::derive_window_input(
                choice,
                context_tokens,
                baseline.profile.limits.max_output_tokens,
                &provider_name,
                &model,
            )?;
            layers = layers.with_embedded(ModelRecord::new().with_limits(ModelLimits::new(
                context_tokens,
                input,
                baseline.profile.limits.max_output_tokens,
            )));
            context_policy::resolve_catalog_profile(
                layers,
                &config.model_limits,
                &provider_name,
                &model,
            )?
        }
        _ => context_policy::resolve_catalog_profile(
            layers,
            &config.model_limits,
            &provider_name,
            &model,
        )?,
    };
    adapter::apply_adapter_cache_capability(
        adapter,
        endpoint.as_deref(),
        request.config.provider.has_pool(),
        &mut profile.profile.capabilities,
    );
    let catalog_controls = request.model_catalog.as_deref().and_then(|snapshot| {
        let catalog_provider =
            smith_config::catalog::catalog_provider_for(&provider_kind, endpoint.as_deref())?;
        snapshot
            .provider(catalog_provider)?
            .models
            .get(model.as_str())?
            .reasoning_controls
            .as_ref()
    });
    let reasoning = resolve_reasoning_policy(
        config,
        &profile.profile,
        endpoint.as_deref(),
        catalog_controls,
    )
    .map_err(|message| FactoryError::Reasoning {
        provider: provider_name.clone(),
        model: model.clone(),
        message,
    })?;
    if reasoning.support == agent_runtime_core::provider::ReasoningSupport::Controllable {
        profile.profile.capabilities.reasoning =
            agent_runtime_core::provider::ReasoningSupport::Controllable;
    }

    // Resolve model-dependent request and context policy before credentials.
    // The same immutable result feeds both the provider loop and planner, so a
    // choice accepted by inventory cannot acquire a different output budget at
    // execution time.
    let output_budget = context_policy::effective_output_budget(config, &profile.profile)?;
    let context_policy =
        context_policy::context_policy(config, &profile.profile.limits, &output_budget);
    let compaction_policy =
        context_policy::compaction_policy(config, &profile.profile, &context_policy);
    let mut loop_config = context_policy::loop_config(request, &model, &output_budget);
    loop_config.reasoning = reasoning.request_config();

    // Capability and requested-value validation precede credential lookup, so
    // an invalid effort never opens a keychain prompt.
    let secret = match (
        &request.provider,
        credentials::active_credential_reference(request),
        &config.provider.api_key,
    ) {
        (None, _, Some(api_key)) => Some(api_key.value.clone()),
        (None, Some(reference), None) => Some(credentials::secret(request, &reference).await?),
        _ => None,
    };
    if adapter == Adapter::ChatGptResponses {
        let secret = secret.as_ref().ok_or(FactoryError::ChatGptAuth(
            crate::chatgpt::ChatGptAuthError::InvalidBundle,
        ))?;
        ChatGptTokenBundle::from_secret(secret).map_err(FactoryError::ChatGptAuth)?;
    }
    // Rejected here rather than at the first model call, so a login that never
    // completed is reported while the user is still in setup.
    if adapter == Adapter::XaiResponses {
        let secret = secret.as_ref().ok_or(FactoryError::XaiAuth(
            crate::xai::XaiAuthError::InvalidBundle,
        ))?;
        XaiTokenBundle::from_secret(secret).map_err(FactoryError::XaiAuth)?;
    }
    // A stored login on the generic Responses kind is the one shape that fails
    // silently: the adapter would send the whole bundle as the bearer and the
    // endpoint would answer "incorrect API key", pointing the user at their
    // key when the fault is the adapter. No real API key parses as a bundle,
    // so this cannot catch a working configuration.
    if adapter == Adapter::OpenAiResponses
        && let Some(secret) = secret.as_ref()
        && XaiTokenBundle::from_secret(secret).is_ok()
    {
        return Err(FactoryError::Runtime(RuntimeError::config(format!(
            "provider `{}` stores a browser login but uses the `{KIND_OPENAI_RESPONSES}` adapter, \
             which sends its credential verbatim; change its `kind` to `{KIND_XAI_RESPONSES}`",
            request.config.provider.name.value
        ))));
    }
    let command = match static_command {
        Some(command) => Some(adapter::prepare_command_provider(request, command).await?),
        None => None,
    };
    Ok(PreparedFactoryInputs {
        provider_name,
        provider_kind,
        adapter,
        endpoint,
        secret,
        model,
        profile,
        context_window,
        reasoning,
        output_budget,
        context_policy,
        compaction_policy,
        loop_config,
        command,
    })
}
