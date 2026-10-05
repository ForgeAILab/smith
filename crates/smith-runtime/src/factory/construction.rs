use super::*;

/// Validates setup through the same factory input derivation used by
/// [`build`], without crossing the runtime-construction boundary.
pub async fn preflight(request: &RuntimeRequest) -> Result<FactoryPreflight, FactoryError> {
    authority::require_workspace(request)?;
    // Every member's reference is parsed before any provider I/O, not just the
    // one this session starts on. A pool whose second entry is malformed must
    // fail here, while the user is looking at setup — not in an hour, when the
    // first account is spent and rotation reaches for a reference that was
    // never going to work.
    provider::validate_pool_references(request)?;
    let prepared = provider::prepare(request).await?;
    Ok(FactoryPreflight {
        provider_name: prepared.provider_name,
        provider_kind: prepared.provider_kind,
        endpoint: prepared.endpoint,
        credential: request
            .config
            .provider
            .credential()
            .map(|reference| reference.value.clone()),
        credentials: request
            .config
            .provider
            .credentials
            .iter()
            .map(|reference| reference.value.clone())
            .collect(),
        model: prepared.model,
        model_profile: prepared.profile.profile,
        context_policy: prepared.context_policy,
        command_implementation: prepared.command.and_then(|command| command.implementation),
    })
}

pub(super) fn prepare_prompt_stage(
    request: &RuntimeRequest,
    loop_config: &mut LoopConfig,
) -> Result<PromptStage, FactoryError> {
    let agent_profile = &request.config.agent.profile;
    let project_instructions = request
        .system_prompt
        .is_none()
        .then(|| request.project_instructions.clone())
        .flatten();
    let prompt_context = DynamicPromptContext {
        project_instructions: project_instructions.clone(),
        agent_profile: Some(AgentProfilePrompt {
            name: agent_profile.name.clone(),
            posture: request.config.agent.active_posture(),
            instructions: agent_profile
                .instructions
                .as_ref()
                .map(|instructions| instructions.value.clone()),
            revision: agent_profile.revision.clone(),
        }),
        todo_planning: capabilities::todo_planning_eligible(request),
        questionnaire: capabilities::questionnaire_eligible(request),
        delegation: capabilities::delegation_eligible(request),
        advisor: capabilities::advisor_eligible(request),
        ..DynamicPromptContext::default()
    };
    let contributor = match request.system_prompt.clone() {
        Some(prompt) => SmithPromptContributor::override_prompt(prompt),
        None => SmithPromptContributor::new(&prompt_context),
    };
    let skills = request.skills.resolve().map_err(FactoryError::Runtime)?;
    let memory = request
        .memory
        .clone()
        .map(|source| {
            let source: Arc<dyn MemorySource> = source;
            MemoryContributor::new(source)
        })
        .transpose()
        .map_err(FactoryError::Runtime)?;
    let rendered = render_fragments(contributor.fragments());

    // Product instructions enter the immutable context plan as independently
    // versioned fragments. Keeping this compatibility field populated would
    // send a second, unbudgeted copy through the legacy planner path.
    loop_config.system_prompt = None;

    Ok(PromptStage {
        project_instructions,
        contributor,
        rendered,
        skills,
        memory,
    })
}

fn prepare_summary_stage(
    request: &RuntimeRequest,
    provider: Arc<dyn Provider>,
    provider_name: &str,
    model: &ModelId,
    limits: ModelLimits,
    clock: Arc<dyn Clock>,
) -> Result<SummaryStage, FactoryError> {
    let Some(config) = request.semantic_summary.clone() else {
        return Ok(None);
    };
    // The coordinator measures context pressure against a budget it cannot
    // discover for itself. Smith already refuses to guess a model's limits, so
    // the resolved input ceiling is the only honest source for it.
    let mut config = config;
    if config.policy.input_budget_tokens == 0 {
        config.policy.input_budget_tokens = u64::from(limits.max_input_tokens);
    }
    config.validate().map_err(FactoryError::Runtime)?;
    let store = request.artifact_store.clone().ok_or_else(|| {
        FactoryError::Runtime(RuntimeError::config(
            "semantic summaries require a protected artifact store for originals",
        ))
    })?;
    let summary_model: Arc<dyn SummaryModel> = match config.model.clone() {
        Some(model) => model,
        None => Arc::new(
            SmithProviderSummaryModel::new(
                provider,
                provider_name.to_owned(),
                model.clone(),
                clock,
                config.max_output_tokens,
                config.timeout_ms,
            )
            .map_err(FactoryError::Runtime)?,
        ),
    };
    // The standard adapter is bound to the active run provider. A custom
    // summary adapter is an independent route and must carry the explicit
    // provider identity validated above; never infer it from the parent.
    let summary_provider = summary_route_provider(&config, provider_name);
    let policy = SemanticSummaryRuntimePolicy {
        purpose: agent_runtime::harness::SEMANTIC_SUMMARY_PURPOSE.into(),
        provider: summary_provider,
        model: summary_model.id().to_owned(),
        revision: config.policy.revision.clone(),
        min_turns: config.policy.min_turns,
        trigger_percent: config.policy.trigger_percent,
        input_budget_tokens: config.policy.input_budget_tokens,
        retain_turns: config.policy.retain_turns,
        max_usage_tokens: config.policy.max_usage_tokens,
        retention: config.policy.retention,
    };
    let coordinator = Arc::new(
        SemanticSummaryCoordinator::new(store, summary_model, config.policy)
            .map_err(FactoryError::Runtime)?,
    );
    Ok(Some((coordinator, policy)))
}

/// Builds the runtime one resolved Smith run needs.
///
/// Async because credential resolution is: the platform credential service is
/// synchronous and may block on an unlock prompt, so it runs on a blocking
/// thread rather than on the executor a provider stream will share.
pub async fn build(harness: ResolvedHarness) -> Result<SmithRuntime, FactoryError> {
    let resolved = resolve::accept(harness);
    let mut harness_identity = resolved.identity;
    let harness_modules = resolved.modules;
    let mut harness_report = resolved.report;
    let request = resolved.request;
    // Host policy first. It costs nothing to check and everything to get wrong,
    // and failing here means a misconfigured run never reaches a keychain.
    let authority = authority::prepare(&request)?;
    let workspace = authority.workspace;
    let approval = authority.approval;
    let PreparedFactoryInputs {
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
        mut loop_config,
        command,
    } = provider::prepare(&request).await?;
    harness_identity = harness_identity.with_output_budget(&output_budget);
    harness_report.finalize_output_budget(&harness_identity, &output_budget);
    let agent_posture = request.config.agent.active_posture();
    let agent_profile = request.config.agent.profile.clone();
    let agent_profile_name = agent_profile.name.clone();
    let prompt = prepare_prompt_stage(&request, &mut loop_config)?;
    let config = &request.config;
    let cache_endpoint_identity = provider::cache_endpoint_identity(
        &provider_name,
        &provider_kind,
        endpoint.as_deref(),
        provider::active_credential_reference(&request).as_deref(),
    );

    // The only boundary the secret crosses.
    let provider_stage = provider::construct_runtime(
        &request,
        adapter,
        endpoint.clone(),
        secret,
        &profile.profile,
        &reasoning,
        command
            .as_ref()
            .map(|command| Arc::clone(&command.provider)),
    )?;
    let provider = provider_stage.provider;
    let image_backend = provider_stage.image_backend;
    let image_history = Arc::new(crate::image_history::SessionImageHistory::default());

    let clock: Arc<dyn Clock> = request
        .clock
        .clone()
        .unwrap_or_else(|| Arc::new(SystemClock));
    let child_profile_routes =
        provider::prepare_child_profile_routes(&request, prompt.project_instructions.as_ref())
            .await?;
    let advisor_route = provider::prepare_advisor_route(&request).await?;
    // Built from the exact same routes `SmithChildFactory.profile_routes`
    // resolves below, so the model-facing `agent` tool can never advertise or
    // accept a profile name the factory would fail to route.
    let agent_tool_profiles: Vec<AgentToolProfile> = child_profile_routes
        .values()
        .map(|route| AgentToolProfile {
            name: route.agent_profile_name.clone(),
            revision: route.agent_profile_revision.clone(),
            provider: route.provider_name.clone(),
            model: route.model.clone(),
        })
        .collect();
    let semantic_summary = prepare_summary_stage(
        &request,
        provider.clone(),
        &provider_name,
        &model,
        profile.profile.limits,
        clock.clone(),
    )?;
    // Warns the model before the compaction boundary. Only meaningful when
    // summarization is configured — without it there is no boundary to warn
    // about, only the structural watermarks, which reclaim silently.
    let budget_notice = semantic_summary.as_ref().and_then(|_| {
        BudgetNoticeComponent::new(
            u64::from(profile.profile.limits.max_input_tokens),
            DEFAULT_NOTICE_THRESHOLD_TOKENS,
        )
        .ok()
        .map(Arc::new)
    });
    let capabilities = capabilities::prepare(
        &request,
        agent_tool_profiles,
        advisor_route,
        image_backend,
        image_history.clone(),
    )?;
    let durability = persistence::prepare(&request).await?;

    let policy = assemble_policy(RuntimePolicy {
        harness: request.config.harness.clone(),
        agent_profile: agent_profile_name.clone(),
        agent_profile_revision: agent_profile.revision.clone(),
        agent_profile_uses: agent_profile.uses.value.clone(),
        agent_profile_source: agent_profile.posture.source.to_string(),
        agent_delegation: agent_profile.delegation.value,
        agent_delegation_source: agent_profile.delegation.source.to_string(),
        agent_profile_legacy: agent_profile.legacy,
        agent_posture,
        provider_name: provider_name.clone(),
        provider_kind: provider_kind.clone(),
        endpoint: endpoint.clone(),
        cache_endpoint_identity: cache_endpoint_identity.clone(),
        credential: config
            .provider
            .credential()
            .map(|reference| reference.value.clone()),
        approval_mode: config.approval.mode.value,
        model: model.clone(),
        model_profile: profile.profile.clone(),
        context_window: context_window
            .active
            .as_ref()
            .map(|window| window.name.clone()),
        context_windows: context_window.available.clone(),
        reasoning: reasoning.clone(),
        context_policy: context_policy.clone(),
        compaction_policy: compaction_policy.clone(),
        system_prompt: prompt.rendered.clone(),
        project_instructions: prompt
            .project_instructions
            .as_ref()
            .map(ProjectInstructionsSnapshot::identity),
        max_attempts: loop_config.retry.max_attempts,
        max_tool_steps: loop_config.max_tool_steps,
        turn_time_limit_ms: loop_config.turn_time_limit_ms,
        output_limit: loop_config.output_limit,
        tool_output_context: ToolOutputContextPolicy::from_config(config),
        artifact_offloading: request.artifact_store.is_some(),
        max_output_tokens: loop_config.max_output_tokens,
        tools: capabilities
            .tools
            .iter()
            .map(|tool| tool.spec().name)
            .collect(),
        skills: prompt
            .skills
            .abilities()
            .iter()
            .map(|ability| ability.name().to_owned())
            .collect(),
        memory_revision: request.memory.as_ref().map(|source| source.revision()),
        semantic_summary: semantic_summary.as_ref().map(|(_, policy)| policy.clone()),
        event_buffer: request.event_buffer,
        shutdown_timeout_ms: request.shutdown_timeout_ms,
        mid_turn_durability: durability.status,
    });

    let tool_authority = Arc::new(SmithToolAuthority::new(workspace.root()));
    let tool_coverage = tool_authority.coverage().clone();
    let interaction_ready = !matches!(request.surface, HostSurface::Child)
        && request
            .interaction
            .as_ref()
            .is_some_and(|broker| broker.readiness() == InteractionReadiness::Ready);
    let activation_context = if interaction_ready {
        ActivationContext::new().with_ready_config([INTERACTION_READY_CONFIG])
    } else {
        ActivationContext::new()
    };
    let scope_inputs = ScopeInputs::new().with_identity(
        ScopeIdentity::new()
            .with_workspace(workspace.root())
            // Terminal/headless/embedded are projections over one Smith
            // agent policy and must derive the same canonical view. Only a
            // delegated child has a distinct execution identity.
            .with_agent(if matches!(request.surface, HostSurface::Child) {
                "smith-child"
            } else {
                agent_profile_name.as_str()
            }),
    );
    let capability_budget =
        ContextBudget::from_limits(&profile.profile.limits, &context_policy).capability_budget;
    // Skills are the asymmetric half of the capability budget. A tool schema
    // is a few hundred tokens; a reference skill is instruction prose and can
    // be thousands, and activation is monotonic — one bound speculatively on
    // the first turn still holds its tokens on the last. Left uncapped, two
    // or three of them fill the budget and the next tool the task needs
    // overflows it, which fails the turn outright. A tenth keeps discovery
    // possible while leaving the schemas the room they need; a reference too
    // large to fit is a reference that needs splitting, not a larger share.
    let activation_budget = ActivationBudget::new(capability_budget, 8)
        .with_instruction_budget(context_policy::skill_instruction_budget(capability_budget));
    // A harness profile runs its turns on an installed CLI. The provider below
    // is still resolved and still supplies model identity and the limits the
    // runtime enforces before any work runs; it is simply never called to
    // produce a turn.
    let execution = crate::cli_agent::turn_execution(&request.config);
    if let crate::cli_agent::TurnExecution::MissingProgram {
        model,
        kind,
        program,
    } = &execution
    {
        // Refused here rather than composed and left to fail per turn: with
        // no backend to run them, every turn would go to the provider
        // carrying a `cli/...` model id it has never heard of, and the run
        // would report that rejection instead of the missing program.
        return Err(FactoryError::AgentNotInstalled {
            model: model.clone(),
            kind: kind.clone(),
            program: program.clone(),
        });
    }

    let mut builder = RuntimeBuilder::new(model.clone())
        .provider(provider.clone())
        .provider_name(provider_name.clone())
        // The profile rather than the catalog: the run must plan against
        // exactly the limits validated above, and passing a catalog as well
        // would be dead weight because an explicit profile always outranks it.
        .model_profile(profile.profile.clone())
        .loop_config(loop_config.clone())
        .model_interceptor(Arc::new(ReasoningInterceptor::new(&reasoning)))
        .context_policy(context_policy.clone())
        .compactor(StructuralCompactor::new(compaction_policy))
        // Declared explicitly so the shared planner records Smith's answer
        // rather than its own "unspecified" placeholder in plan fingerprints.
        // The answer is the adapter's own declaration: an implicit-prefix
        // provider caches the stable run, and reporting `none` here made
        // every plan claim the provider could reuse nothing.
        .cache_capability(ProviderCacheCapability::from_control(
            RegistryRevision::new(CACHE_CAPABILITY_REVISION),
            provider_kind.clone(),
            provider
                .capabilities(&model)
                .map(|capabilities| capabilities.prompt_cache)
                .unwrap_or_default(),
        ))
        .security_check(
            tool_authority,
            SecurityCheckMode::Authoritative,
            tool_coverage,
            ActionClass::new("smith-built-in-tools"),
        )
        .approval(approval.clone())
        .workspace(workspace.clone())
        .tools(capabilities.tools.clone())
        .live_ability_routing()
        .scope_inputs(scope_inputs)
        .capability_resolver(Arc::new(CapabilityResolver::new()))
        .activation_policy(Arc::new(FailClosedPolicy))
        .activation_context(activation_context)
        .activation_budget(activation_budget)
        .context_contributor(Arc::new(prompt.contributor.clone()))
        .clock(clock.clone())
        .event_buffer(request.event_buffer)
        .shutdown_timeout_ms(request.shutdown_timeout_ms);
    // A harness replaces how a turn is executed, leaving every other piece of
    // the composition above in place.
    if let crate::cli_agent::TurnExecution::InstalledAgent(plan) = &execution {
        builder = builder.external_agent(plan.backend(PathBuf::from(workspace.root()), true));
    }
    if let Some(identity) = cache_endpoint_identity.as_ref() {
        builder = builder.cache_endpoint_identity(identity.clone());
    }
    if let Some(advisor) = &capabilities.advisor {
        builder = builder
            .tool_view_resolver(advisor.clone())
            .turn_commit_hook(advisor.clone());
    }
    if let Some(component) = &capabilities.todo {
        builder = builder
            .context_contributor(component.clone())
            .tool_output_processor(component.clone())
            .turn_commit_hook(component.clone());
    }
    if let Some(component) = &capabilities.goal {
        builder = builder
            .context_contributor(component.clone())
            .model_interceptor(component.clone())
            .tool_output_processor(component.clone())
            .turn_commit_hook(component.clone());
    }
    if let Some(contributor) = prompt.memory.clone() {
        builder = builder.context_contributor(Arc::new(contributor));
    }
    if let Some((coordinator, _)) = &semantic_summary {
        builder = builder
            .history_projector(coordinator.clone())
            .turn_commit_hook(coordinator.clone());
    }
    if let Some(component) = &budget_notice {
        builder = builder
            .context_contributor(component.clone())
            .turn_commit_hook(component.clone());
    }
    for descriptor in capabilities.abilities.descriptors() {
        builder = builder.tool_ability_descriptor(descriptor);
    }
    for skill in prompt.skills.abilities().iter().cloned() {
        builder = builder.ability(skill);
    }
    if let Some(interaction) = request.interaction.clone() {
        builder = builder.interaction_broker(interaction);
    }
    if capabilities.delegation_slot.is_some() {
        let authority = Arc::new(DelegationAuthority::new());
        let coverage = authority.coverage().clone();
        builder = builder.security_check(
            authority,
            SecurityCheckMode::Authoritative,
            coverage,
            ActionClass::new("smith-delegation"),
        );
    }
    if let Some(store) = request.session_store.clone() {
        let store = capabilities.advisor.as_ref().map_or_else(
            || store.clone(),
            |advisor| advisor.accounting_store(store.clone()),
        );
        builder = builder.session_store(store);
    }
    if let Some(store) = durability.root_store.clone() {
        builder = builder.checkpoint_store(store);
    }
    if let Some(store) = request.secret_store.clone() {
        builder = builder.secret_store(store);
    }
    if let Some(store) = request.artifact_store.clone() {
        let offloader = ToolOutputContextPolicy::from_config(config)
            .offloader(store)
            .map_err(FactoryError::Runtime)?;
        builder = builder.tool_output_processor(Arc::new(offloader));
    }
    for observer in request.observers.iter().cloned() {
        builder = builder.observer(observer);
    }

    let built = compose::runtime(builder)?;
    let delegation = delegation::assemble(capabilities.delegation_slot.clone().map(|slot| {
        SmithDelegation {
            factory: Arc::new(SmithChildFactory {
                default_route: SmithChildRoute {
                    provider,
                    provider_name,
                    provider_kind,
                    cache_endpoint_identity,
                    model,
                    model_profile: profile.profile.clone(),
                    context_policy,
                    tool_output_context: ToolOutputContextPolicy::from_config(config),
                    loop_config,
                    prompt_contributor: prompt.contributor.without_advisor(),
                    agent_profile_name: agent_profile.name.clone(),
                    agent_profile_revision: agent_profile.revision.clone(),
                    agent_profile_posture: agent_profile.posture.value,
                    read_only: agent_profile.posture.value.is_read_only(),
                    // An inheriting child is the parent's run narrowed, so it
                    // runs turns wherever the parent's do.
                    execution,
                },
                profile_routes: child_profile_routes,
                approval,
                workspace,
                clock,
                artifact_store: request.artifact_store.clone(),
                session_store: request.session_store.clone(),
                checkpoint_store: durability.child_store,
                skills: prompt.skills.abilities().to_vec(),
                memory: prompt.memory,
                semantic_summary: semantic_summary
                    .as_ref()
                    .map(|(coordinator, _)| coordinator.clone()),
            }),
            slot,
        }
    }));
    Ok(SmithRuntime {
        runtime: built.runtime,
        policy: Arc::new(policy.policy),
        profile: Arc::new(profile),
        abilities: Arc::new(capabilities.abilities),
        skill_index: Arc::from(prompt.skills.index().to_vec().into_boxed_slice()),
        checkpoint_store: durability.root_store,
        artifact_store: request.artifact_store,
        surface: request.surface,
        delegation: delegation.delegation,
        advisor_slot: capabilities.advisor_slot,
        advisor: capabilities.advisor,
        goal_component: capabilities.goal,
        background_services: request.background_services,
        harness_identity,
        harness_modules,
        harness_report,
        image_history,
    })
}
