//! Child provider routes and cache endpoint identity.

use super::*;

use super::adapter;
use super::credentials;

pub(in crate::factory) async fn prepare_child_profile_routes(
    request: &RuntimeRequest,
    project_instructions: Option<&ProjectInstructionsSnapshot>,
) -> Result<BTreeMap<String, SmithChildRoute>, FactoryError> {
    let mut routes = BTreeMap::new();
    for child in &request.child_profiles {
        let mut route_request = RuntimeRequest::new(child.config.clone(), HostSurface::Child);
        route_request.project_instructions = project_instructions.cloned();
        route_request.workspace = request.workspace.clone();
        route_request.approval = request.approval.clone();
        route_request.credentials = request.credentials.clone();
        route_request.transport = request.transport.clone();
        route_request.credential_timeout_ms = request.credential_timeout_ms;
        route_request.catalog_sources = child.catalog_sources.clone();
        route_request.model_catalog = request.model_catalog.clone();
        route_request.persistence_redactor = request.persistence_redactor.clone();

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
            None,
        )?;
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
        let execution = crate::cli_agent::turn_execution(&route_request.config);
        let route_key =
            crate::delegation::profile_route_key(&agent_profile.name, &agent_profile.revision);
        let replaced = routes.insert(
            route_key.clone(),
            SmithChildRoute {
                provider,
                provider_name,
                provider_kind,
                cache_endpoint_identity,
                model,
                model_profile: profile.profile,
                context_policy,
                tool_output_context: ToolOutputContextPolicy::from_config(&route_request.config),
                loop_config,
                prompt_contributor,
                agent_profile_name: agent_profile.name.clone(),
                agent_profile_revision: agent_profile.revision.clone(),
                agent_profile_posture: agent_profile.posture.value,
                read_only: agent_profile.posture.value.is_read_only(),
                execution,
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
