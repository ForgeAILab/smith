//! Resolved host construction and interactive restart ownership.

use super::*;

pub(super) struct StartedHost {
    pub(super) host: HostSession,
    pub(super) approvals: Option<ApprovalRequests>,
    pub(super) headless_approval: Option<Arc<HeadlessApproval>>,
    pub(super) interactions: Option<InteractionRequests>,
    pub(super) headless_interaction: Option<Arc<HeadlessInteraction>>,
    pub(super) project: PathBuf,
    pub(super) inventory: SelectionInventory,
    pub(super) agents: ResolvedAgent,
    pub(super) sessions: Vec<SessionListing>,
    pub(super) catalog: Arc<smith_config::catalog::CatalogSnapshot>,
}

pub(super) async fn start_host(
    selection: &Selection,
    resume: Option<&str>,
    surface: HostSurface,
    frozen_catalog: Option<Arc<smith_config::catalog::CatalogSnapshot>>,
) -> Result<StartedHost> {
    let prepared = prepare(selection)?;
    let project = prepared.project;
    let resolution = prepared.resolution;
    let catalog = match frozen_catalog {
        Some(catalog) => catalog,
        None => {
            let loader = CatalogLoader::production(&resolution.layout.user_dir)
                .map_err(|error| anyhow::anyhow!("{error}"))
                .context("preparing the provider model catalog")?;
            let allow_refresh = smith_config::catalog::catalog_provider_for(
                &resolution.config.provider.kind.value,
                resolution
                    .config
                    .provider
                    .base_url
                    .as_ref()
                    .map(|value| value.value.as_str()),
            )
            .is_some();
            loader
                .prepare(allow_refresh)
                .await
                .map_err(|error| anyhow::anyhow!("{error}"))
                .context("preparing the provider model catalog")?
                .snapshot
        }
    };
    let inventory =
        local_inventory_with_catalog(&resolution, AVAILABLE_ADAPTER_KINDS, Some(&catalog))
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("building the local runtime inventory")?;
    let agents = resolution.config.agent.clone();
    for profile in agents
        .profiles
        .values()
        .filter(|profile| profile.legacy && profile.posture.source.layer != Layer::BuiltIn)
    {
        eprintln!(
            "smith: warning: {}: deprecated agent mode/child preset `{}` was adapted as a profile; migrate to [profiles.{}] with explicit posture and use",
            profile.posture.source, profile.name, profile.name,
        );
    }
    let workspace = ProjectWorkspace::new(&project)
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("rooting the project workspace")?;
    let mut runtime = RuntimeRequest {
        workspace: Some(Arc::new(workspace)),
        credentials: Some(CredentialResolver::new(&resolution.layout.user_dir)),
        model_catalog: Some(catalog.clone()),
        ..RuntimeRequest::new(resolution.config.clone(), surface)
    };
    let persistence_redactor = DefaultRedactor::new();
    runtime.persistence_redactor = Some(persistence_redactor.clone());
    if let Some(source) = runtime_catalog_source(
        &catalog,
        &resolution.config.provider.name.value,
        &resolution.config.provider.kind.value,
        resolution
            .config
            .provider
            .base_url
            .as_ref()
            .map(|value| value.value.as_str()),
    ) {
        runtime.catalog_sources.push(source);
    }
    for profile in agents
        .profiles
        .values()
        .filter(|profile| profile.supports(ProfileUse::Child) && !profile.legacy)
    {
        let mut child_selection = selection.clone();
        child_selection.profile = Some(profile.name.clone());
        child_selection.provider = None;
        child_selection.model = None;
        // The session `/think`–`/effort` override belongs to the main
        // binding the user chose it against. Forwarding it here would make a
        // child profile on a non-controllable binding abort startup and clear
        // the parent's valid override.
        child_selection.reasoning_enabled = None;
        child_selection.reasoning_effort = None;
        let (_, child_request) = resolution_request(&child_selection)?;
        let child_resolution = resolve(&child_request.with_profile_use(ProfileUse::Child))
            .map_err(|error| anyhow::anyhow!("{error}"))
            .with_context(|| format!("resolving child profile `{}`", profile.name))?;
        let mut catalog_sources = Vec::new();
        if let Some(source) = runtime_catalog_source(
            &catalog,
            &child_resolution.config.provider.name.value,
            &child_resolution.config.provider.kind.value,
            child_resolution
                .config
                .provider
                .base_url
                .as_ref()
                .map(|value| value.value.as_str()),
        ) {
            catalog_sources.push(source);
        }
        runtime.child_profiles.push(ChildProfileRequest {
            config: child_resolution.config,
            catalog_sources,
        });
    }

    let mut approvals = None;
    let mut headless_approval = None;
    if resolution.config.approval.mode.value == ApprovalMode::Ask {
        if surface == HostSurface::Terminal {
            let (approval, requests) = InteractiveApproval::new(8);
            runtime.approval = Some(Arc::new(approval));
            approvals = Some(requests);
        } else {
            let approval = Arc::new(HeadlessApproval::new());
            runtime.approval = Some(approval.clone());
            headless_approval = Some(approval);
        }
    }
    let (interactions, headless_interaction) = match surface {
        HostSurface::Terminal => {
            let (broker, requests) =
                InteractiveInteraction::with_sensitive_value_sink(Arc::new(persistence_redactor));
            runtime.interaction = Some(Arc::new(broker));
            (Some(requests), None)
        }
        HostSurface::Headless => {
            let broker = Arc::new(HeadlessInteraction::new());
            runtime.interaction = Some(broker.clone());
            (None, Some(broker))
        }
        HostSurface::Child => (None, None),
    };

    let mut request = HostSessionRequest::new(runtime, &project).reasoning_reset(
        selection.reasoning_enabled_reset,
        selection.reasoning_effort_reset,
    );
    if let Some(session) = resume {
        request = request.resume(SessionId::new(session));
    }
    let host = smith_runtime::host::start(request)
        .await
        .map_err(anyhow::Error::new)
        .context("starting the Smith session")?;
    let sessions = smith_runtime::host::list(&resolution.config, &project)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("listing project sessions")?;

    Ok(StartedHost {
        host,
        approvals,
        headless_approval,
        interactions,
        headless_interaction,
        project,
        inventory,
        agents,
        sessions,
        catalog,
    })
}

pub(super) async fn run_interactive_command(mut args: RunArgs) -> Result<u8> {
    let mut resume = args.resume.take();
    let mut frozen_catalog = None;
    let mut reasoning_notice = None;
    loop {
        let started = match start_host(
            &args.selection,
            resume.as_deref(),
            HostSurface::Terminal,
            frozen_catalog.clone(),
        )
        .await
        {
            Ok(started) => started,
            Err(error)
                if is_reasoning_startup_error(&error)
                    && (args.selection.reasoning_enabled.is_some()
                        || args.selection.reasoning_effort.is_some()
                        || (resume.is_some()
                            && (!args.selection.reasoning_enabled_reset
                                || !args.selection.reasoning_effort_reset))) =>
            {
                args.selection.reasoning_enabled = None;
                args.selection.reasoning_effort = None;
                args.selection.reasoning_enabled_reset = true;
                args.selection.reasoning_effort_reset = true;
                reasoning_notice = Some(
                    "cleared the saved thinking/effort override because the selected provider/model cannot represent it"
                        .to_owned(),
                );
                continue;
            }
            Err(error) => return Err(error),
        };
        let StartedHost {
            host,
            approvals,
            interactions,
            project,
            inventory,
            agents,
            sessions,
            catalog,
            ..
        } = started;
        let current_session = host.session().id().as_str().to_owned();
        match run_interactive(
            &host,
            approvals,
            interactions,
            &project,
            InteractiveResources {
                inventory,
                agents,
                sessions,
            },
            PresentationOptions {
                no_color: args.no_color,
                no_motion: args.no_motion,
                reasoning_notice: reasoning_notice.take(),
            },
        )
        .await?
        {
            InteractiveExit::Quit => return Ok(0),
            InteractiveExit::Reconfigure(command) => {
                match &command {
                    PaletteCommand::Connect(provider) => {
                        resume = Some(current_session);
                        frozen_catalog = None;
                        let _completed = connection::connect(
                            args.selection.clone(),
                            provider,
                            args.no_color,
                            args.no_motion,
                        )
                        .await?;
                        continue;
                    }
                    PaletteCommand::Disconnect(provider) => {
                        let outcome = connection::disconnect(&args.selection, provider).await?;
                        if outcome == connection::DisconnectOutcome::ActiveDirectProvider {
                            println!(
                                "The active provider was disconnected. The session is saved; restart Smith with a connected provider to resume it."
                            );
                            return Ok(0);
                        }
                        resume = Some(current_session);
                        frozen_catalog = None;
                        continue;
                    }
                    _ => {}
                }
                frozen_catalog = matches!(
                    &command,
                    PaletteCommand::Profile(_)
                        | PaletteCommand::Model { .. }
                        | PaletteCommand::Agent(_)
                        | PaletteCommand::Think(_)
                        | PaletteCommand::Effort(_)
                )
                .then_some(catalog);
                apply_palette_command(&mut args.selection, &mut resume, current_session, command);
            }
        }
    }
}

pub(super) fn is_reasoning_startup_error(error: &anyhow::Error) -> bool {
    error.chain().any(|source| {
        matches!(
            source.downcast_ref::<FactoryError>(),
            Some(FactoryError::Reasoning { .. })
        )
    })
}

pub(super) fn apply_palette_command(
    selection: &mut Selection,
    resume: &mut Option<String>,
    current_session: String,
    command: PaletteCommand,
) {
    match command {
        PaletteCommand::NewSession => {
            *resume = None;
        }
        PaletteCommand::Resume(session) => {
            *resume = Some(session);
        }
        PaletteCommand::Profile(profile) => {
            selection.profile = Some(profile);
            selection.provider = None;
            selection.model = None;
            *resume = Some(current_session);
        }
        PaletteCommand::Model { provider, model } => {
            selection.profile = None;
            selection.provider = Some(provider);
            selection.model = Some(model);
            *resume = Some(current_session);
        }
        PaletteCommand::Connect(_) | PaletteCommand::Disconnect(_) => {
            unreachable!("connection commands are handled before selection reconfiguration")
        }
        PaletteCommand::Agent(agent) => {
            selection.agent = Some(agent);
            *resume = Some(current_session);
        }
        PaletteCommand::Think(enabled) => {
            selection.reasoning_enabled = enabled;
            selection.reasoning_enabled_reset = enabled.is_none();
            *resume = Some(current_session);
        }
        PaletteCommand::Effort(effort) => {
            selection.reasoning_effort = effort;
            selection.reasoning_effort_reset = selection.reasoning_effort.is_none();
            *resume = Some(current_session);
        }
    }
}

pub(super) fn read_prompt(reader: impl Read) -> Result<String> {
    let mut prompt = String::new();
    reader
        .take(u64::try_from(MAX_STDIN_PROMPT_BYTES + 1).unwrap_or(u64::MAX))
        .read_to_string(&mut prompt)
        .context("reading the UTF-8 prompt from stdin")?;
    if prompt.len() > MAX_STDIN_PROMPT_BYTES {
        anyhow::bail!("stdin prompt exceeds the {MAX_STDIN_PROMPT_BYTES} byte limit");
    }
    if prompt.trim().is_empty() {
        anyhow::bail!("stdin did not contain a prompt");
    }
    Ok(prompt)
}
