//! Child provider routes and cache endpoint identity.

use super::*;

use smith_config::resolve::AdvisorTarget;

use super::adapter;
use super::credentials;

/// A single tool-free reviewer binding prepared exactly like a child binding.
pub(in crate::factory) async fn prepare_advisor_route(
    request: &RuntimeRequest,
) -> Result<Option<Arc<AdvisorRoute>>, FactoryError> {
    if matches!(request.surface, HostSurface::Child) {
        return Ok(None);
    }
    let Some(selected) = &request.config.agent.profile.advisor else {
        return Ok(None);
    };
    let advisor =
        request
            .advisor_profile
            .as_ref()
            .ok_or_else(|| FactoryError::MissingHostPolicy {
                what: "resolved advisor profile",
                message: format!(
                    "resolve advisor profile `{}` before composition",
                    selected.value
                ),
            })?;
    let matches_selection = match &selected.value {
        AdvisorTarget::InstalledAgent { .. } => {
            advisor.config.model.value == selected.value.to_string()
                && advisor.config.provider.name.value == request.config.provider.name.value
        }
        AdvisorTarget::Profile(name) => advisor.config.agent.profile.name == *name,
        AdvisorTarget::Model { provider, model } => {
            advisor.config.provider.name.value == *provider && advisor.config.model.value == *model
        }
    };
    if !matches_selection {
        return Err(FactoryError::Runtime(RuntimeError::config(
            "the resolved advisor route does not match the selected advisor",
        )));
    }
    let mut route_request = RuntimeRequest::new(advisor.config.clone(), HostSurface::Child);
    route_request.workspace = request.workspace.clone();
    route_request.approval = request.approval.clone();
    route_request.credentials = request.credentials.clone();
    route_request.transport = request.transport.clone();
    route_request.credential_timeout_ms = request.credential_timeout_ms;
    route_request.catalog_sources = advisor.catalog_sources.clone();
    route_request.model_catalog = request.model_catalog.clone();
    route_request.persistence_redactor = request.persistence_redactor.clone();
    route_request.provider = advisor.provider.clone();
    route_request.built_in_tools = false;
    route_request.modules = super::super::modules::for_config(&request.modules, &advisor.config);
    super::validate_pool_references(&route_request)?;
    let prepared = super::prepare(&route_request).await?;
    let price = route_request.model_catalog.as_ref().and_then(|catalog| {
        smith_config::catalog::catalog_provider_for(
            &prepared.provider_kind,
            prepared.endpoint.as_deref(),
        )
        .and_then(|provider| catalog.provider(provider))
        .and_then(|provider| provider.models.get(prepared.model.as_str()))
        .and_then(|model| model.cost)
    });
    let provider = super::construct_runtime(
        &route_request,
        prepared.adapter,
        prepared.endpoint,
        prepared.secret,
        &prepared.profile.profile,
        &prepared.reasoning,
        prepared.command.map(|command| command.provider),
    )?
    .provider;
    let execution = crate::advisor::AdvisorExecution::from_turn(
        crate::cli_agent::turn_execution(&advisor.config, request.smith_executable.as_deref()),
        provider.clone(),
    );
    Ok(Some(Arc::new(AdvisorRoute {
        execution,
        provider,
        provider_name: prepared.provider_name,
        model: prepared.model,
        model_profile: prepared.profile.profile,
        context_policy: prepared.context_policy,
        reasoning: prepared.reasoning,
        output_budget: prepared.output_budget,
        // A model advisor carries no profile, so no profile's instructions.
        instructions: matches!(selected.value, AdvisorTarget::Profile(_))
            .then(|| advisor.config.agent.profile.instructions.as_ref())
            .flatten()
            .map(|instructions| instructions.value.clone()),
        price: if smith_config::cli_agents::parse_cli_model_id(&advisor.config.model.value)
            .is_some()
        {
            None
        } else {
            price
        },
        plugins: request
            .plugin_profiles
            .get(&advisor.config.agent.profile.name)
            .cloned(),
    })))
}

/// Resolves a tool-free vision provider only for a text-only active model.
pub(in crate::factory) async fn prepare_vision_route(
    request: &RuntimeRequest,
    accounting: Arc<crate::vision::VisionAccounting>,
) -> Result<Option<crate::vision::VisionRoute>, FactoryError> {
    if request.config.model_image_input.value == smith_config::resolve::ImageInput::Accepts
        || !request.built_in_tools
        || (request.config.agent.active_posture().is_read_only()
            && !request
                .config
                .agent
                .read_only_network_modules
                .value
                .iter()
                .any(|id| id == "vision"))
        || !request.modules.enabled.contains("vision")
        || !request
            .modules
            .compiled
            .iter()
            .any(|module| module.module.id() == "vision")
    {
        return Ok(None);
    }
    let Some(selected) = request.config.agent.profile.vision.selected() else {
        return Ok(None);
    };
    let vision =
        request
            .vision_profile
            .as_ref()
            .ok_or_else(|| FactoryError::MissingHostPolicy {
                what: "resolved vision profile",
                message: format!(
                    "resolve vision target `{}` before composition",
                    selected.value
                ),
            })?;
    let matches_selection = match &selected.value {
        AdvisorTarget::InstalledAgent { .. } => false,
        AdvisorTarget::Profile(name) => vision.config.agent.profile.name == *name,
        AdvisorTarget::Model { provider, model } => {
            vision.config.provider.name.value == *provider && vision.config.model.value == *model
        }
    };
    if !matches_selection
        || vision.config.model_image_input.value != smith_config::resolve::ImageInput::Accepts
    {
        return Err(FactoryError::Runtime(RuntimeError::config(
            "the resolved vision route must match the selection and accept image input",
        )));
    }
    let mut route_request = Box::new(RuntimeRequest::new(
        vision.config.clone(),
        HostSurface::Child,
    ));
    route_request.workspace = request.workspace.clone();
    route_request.approval = request.approval.clone();
    route_request.credentials = request.credentials.clone();
    route_request.transport = request.transport.clone();
    route_request.credential_timeout_ms = request.credential_timeout_ms;
    route_request.catalog_sources = vision.catalog_sources.clone();
    route_request.model_catalog = request.model_catalog.clone();
    route_request.persistence_redactor = request.persistence_redactor.clone();
    route_request.provider = vision.provider.clone();
    route_request.built_in_tools = false;
    super::validate_pool_references(&route_request)?;
    let prepared = super::prepare(&route_request).await?;
    let price = route_request.model_catalog.as_ref().and_then(|catalog| {
        smith_config::catalog::catalog_provider_for(
            &prepared.provider_kind,
            prepared.endpoint.as_deref(),
        )
        .and_then(|provider| catalog.provider(provider))
        .and_then(|provider| provider.models.get(prepared.model.as_str()))
        .and_then(|model| model.cost)
    });
    let endpoint = match prepared.endpoint.clone() {
        Some(endpoint) => endpoint,
        None => adapter::endpoint(&route_request.config.provider, None)?,
    };
    let provider = super::construct_runtime(
        &route_request,
        prepared.adapter,
        prepared.endpoint,
        prepared.secret,
        &prepared.profile.profile,
        &prepared.reasoning,
        prepared.command.map(|command| command.provider),
    )?
    .provider;
    Ok(Some(crate::vision::VisionRoute {
        binding: smith_module::VisionBinding {
            provider,
            provider_name: prepared.provider_name,
            model: prepared.model,
            endpoint,
            output_token_cap: prepared.output_budget.request_tokens.min(1024),
            usage: accounting,
        },
        price,
    }))
}

pub(in crate::factory) async fn prepare_child_profile_routes(
    request: &RuntimeRequest,
    project_instructions: Option<&ProjectInstructionsSnapshot>,
    session_history: Arc<crate::session_history::LiveSessionHistory>,
    vision_accounting: Arc<crate::vision::VisionAccounting>,
) -> Result<BTreeMap<String, SmithChildRoute>, FactoryError> {
    let mut routes = BTreeMap::new();
    for child in &request.child_profiles {
        let mut route_request = RuntimeRequest::new(child.config.clone(), HostSurface::Child);
        // Child harnesses use the same host-owned Smith binary as the root.
        route_request.smith_executable = request.smith_executable.clone();
        route_request.web_search_session = request.web_search_session.clone();
        route_request.project_instructions = project_instructions.cloned();
        route_request.modules = super::super::modules::for_config(&request.modules, &child.config);
        route_request.vision_profile = child.vision_profile.clone();
        route_request.workspace = request.workspace.clone();
        route_request.approval = request.approval.clone();
        route_request.credentials = request.credentials.clone();
        route_request.transport = request.transport.clone();
        route_request.credential_timeout_ms = request.credential_timeout_ms;
        route_request.catalog_sources = child.catalog_sources.clone();
        route_request.model_catalog = request.model_catalog.clone();
        route_request.persistence_redactor = request.persistence_redactor.clone();

        let vision = Box::pin(prepare_vision_route(
            &route_request,
            vision_accounting.clone(),
        ))
        .await?;
        let PreparedFactoryInputs {
            provider_name,
            provider_kind,
            adapter,
            endpoint,
            secret,
            model,
            profile,
            context_window: _,
            reasoning,
            output_budget: _,
            context_policy,
            compaction_policy: _,
            mut loop_config,
            command,
        } = super::prepare(&route_request).await?;
        let cache_endpoint_identity = cache_endpoint_identity(
            &provider_name,
            &provider_kind,
            endpoint.as_deref(),
            credentials::active_credential_reference(&route_request).as_deref(),
        );
        if let (Some(secret), Some(redactor)) = (&secret, &route_request.persistence_redactor) {
            redactor.register_secret(secret);
        }
        // This is a per-route rebuild, which starts from the route's own
        // configuration rather than the session's live pool.
        let route_pool = credentials::credential_pool_for(&route_request);
        let mut image_binding = None;
        let provider = adapter::construct(
            adapter,
            &route_request,
            &profile.profile,
            adapter::ProviderConstructionInputs {
                endpoint,
                secret,
                supported_thinking_levels: &reasoning.efforts,
                pool: route_pool.as_ref(),
                command: command.map(|command| command.provider),
            },
            Some(&mut image_binding),
        )?;
        let provider = Arc::new(crate::image_fitting::ImageFittingProvider::new(
            provider,
            crate::factory::image_limits_for_provider_kind(&provider_kind),
        )) as Arc<dyn Provider>;
        let provider = crate::response::apply_response_policy(
            provider,
            route_request
                .config
                .provider
                .response
                .reasoning_only
                .as_ref()
                .map(|policy| policy.value),
        );
        let provider = match reasoning.dialect {
            Some(dialect) => {
                Arc::new(ReasoningDialectProvider::new(provider, dialect)) as Arc<dyn Provider>
            }
            None => provider,
        };
        let web = super::super::web::prepare(
            &route_request,
            provider
                .capabilities(&model)
                .map(|caps| caps.hosted_web_search)
                .unwrap_or(false),
        )
        .await;
        let agent_profile = &route_request.config.agent.profile;
        let prompt_context = DynamicPromptContext {
            project_instructions: project_instructions.cloned(),
            agent_profile: Some(AgentProfilePrompt {
                name: agent_profile.name.clone(),
                posture: agent_profile.posture.value,
                instructions: agent_profile
                    .instructions
                    .as_ref()
                    .map(|instructions| instructions.value.clone()),
                revision: agent_profile.revision.clone(),
            }),
            // A child always runs on `HostSurface::Child`, so it registers
            // neither the root questionnaire nor a delegation tool of its own.
            todo_planning: !agent_profile.posture.value.is_read_only(),
            questionnaire: false,
            delegation: false,
            advisor: false,
            ..DynamicPromptContext::default()
        };
        let prompt_contributor = SmithPromptContributor::new(&prompt_context);
        loop_config.system_prompt = None;
        // A child profile that names an installed agent runs its turns on
        // that agent, exactly as the same profile does at the root. Without
        // this the child would be composed against the provider the profile
        // resolved only for model identity and limits, and every child turn
        // would die asking that provider for a `cli/...` model.
        //
        // A missing program is carried into the route rather than refused
        // here: one uninstallable child profile must not stop a session that
        // may never spawn it, and the route reports the real reason if it is
        // ever spawned.
        let execution = crate::cli_agent::turn_execution(
            &route_request.config,
            route_request.smith_executable.as_deref(),
        );
        let route_key =
            crate::delegation::profile_route_key(&agent_profile.name, &agent_profile.revision);
        let replaced = routes.insert(
            route_key.clone(),
            SmithChildRoute {
                hosted_tools: web.hosted,
                web_diagnostic: web.diagnostic,
                read_only_network_modules: route_request
                    .config
                    .agent
                    .read_only_network_modules
                    .value
                    .clone(),
                web_search_session: request.web_search_session.clone(),
                provider,
                provider_name,
                provider_kind,
                cache_endpoint_identity,
                model,
                model_profile: profile.profile.clone(),
                context_policy,
                tool_output_context: ToolOutputContextPolicy::from_config(&route_request.config),
                loop_config,
                prompt_contributor,
                agent_profile_name: agent_profile.name.clone(),
                agent_profile_revision: agent_profile.revision.clone(),
                agent_profile_posture: agent_profile.posture.value,
                read_only: agent_profile.posture.value.is_read_only(),
                capabilities: agent_profile.capabilities.clone(),
                execution,
                plugins: request.plugin_profiles.get(&agent_profile.name).cloned(),
                modules: route_request.modules.clone(),
                module_context: super::super::modules::context(
                    &route_request,
                    web.settings,
                    image_binding.map(|binding| smith_module::ImageBinding {
                        endpoint: binding.endpoint,
                        target: binding.target,
                        credentials: binding.credentials,
                        chatgpt: binding.chatgpt,
                    }),
                    vision.map(|vision| vision.binding),
                    profile.profile.limits.max_input_tokens,
                    request.semantic_summary.is_some() && request.artifact_store.is_some(),
                    session_history.clone(),
                )?,
            },
        );
        if replaced.is_some() {
            return Err(FactoryError::Runtime(RuntimeError::conflict(format!(
                "duplicate child profile route `{route_key}`"
            ))));
        }
    }
    Ok(routes)
}

/// Redaction-safe endpoint/tenant partition consumed into Runtime's cache
/// identity. The normalized endpoint and credential reference are inputs to
/// the digest only; neither is retained by `CacheEndpointIdentity`.
pub(in crate::factory) fn cache_endpoint_identity(
    provider_name: &str,
    provider_kind: &str,
    endpoint: Option<&str>,
    credential_reference: Option<&str>,
) -> Option<CacheEndpointIdentity> {
    let endpoint = endpoint?;
    let label = format!(
        "provider={provider_name}\0kind={provider_kind}\0endpoint={endpoint}\0credential={}",
        credential_reference.unwrap_or("none")
    );
    Some(CacheEndpointIdentity::from_opaque(
        label,
        RegistryRevision::new(CACHE_ENDPOINT_IDENTITY_REVISION),
    ))
}
