//! Resolved host construction and interactive restart ownership.

use super::*;

pub(super) struct StartedHost {
    pub(super) host: HostSession,
    pub(super) approvals: Option<ApprovalRequests>,
    pub(super) headless_approval: Option<Arc<HeadlessApproval>>,
    pub(super) interactions: Option<InteractionRequests>,
    pub(super) headless_interaction: Option<Arc<HeadlessInteraction>>,
    /// Rotation offers awaiting a surface, when this host can answer them.
    pub(super) rotations: Option<RotationRequests>,
    /// The fail-closed policy an unattended run used, for machine output.
    pub(super) headless_rotation: Option<Arc<HeadlessRotation>>,
    /// Live credential-pool state, when the provider declares a pool.
    pub(super) credential_pool: Option<SharedPool>,
    /// Remembered accounts, so a switch survives the session.
    pub(super) accounts: ActiveAccounts,
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
        // `--effort` is chosen against the main binding for the same reason,
        // and a child profile may sit on a binding with no effort ladder at
        // all.
        child_selection.effort = None;
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
    // The pool exists only when the provider declares more than one account;
    // a single-credential provider gets no pool, no policy, and behaves
    // exactly as it did before pools existed.
    let accounts = ActiveAccounts::load(&resolution.layout.user_dir).await;
    let mut credential_pool = None;
    let mut rotations = None;
    let mut headless_rotation = None;
    if resolution.config.provider.has_pool() {
        let references: Vec<String> = resolution
            .config
            .provider
            .credentials
            .iter()
            .map(|reference| reference.value.clone())
            .collect();
        let provider_name = resolution.config.provider.name.value.clone();
        let mut pool = CredentialPool::new(
            provider_name.clone(),
            references.clone(),
            resolution
                .config
                .provider
                .rotate_at_percent
                .as_ref()
                .map(|threshold| threshold.value),
        );
        // Resume onto the account the user was last using. A remembered
        // account that is no longer declared resolves to nothing, which starts
        // on the first member — the same place a first-ever run starts.
        if let Some(position) = accounts.position_in(&provider_name, &references) {
            pool.set_active(position);
        }
        let pool = SharedPool::new(pool);
        runtime.credential_pool = Some(pool.clone());
        credential_pool = Some(pool);

        match surface {
            HostSurface::Terminal => {
                let (policy, requests) = InteractiveRotation::new(4);
                runtime.rotation = Some(Arc::new(policy));
                rotations = Some(requests);
            }
            // A headless run keeps the account it started on: its credential
            // must not change under a script, and there is no surface to ask.
            HostSurface::Headless | HostSurface::Child => {
                let policy = Arc::new(HeadlessRotation::new());
                runtime.rotation = Some(policy.clone());
                headless_rotation = Some(policy);
            }
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

    let mut request = HostSessionRequest::new(runtime, &project)
        .reasoning_reset(
            selection.reasoning_enabled_reset,
            selection.reasoning_effort_reset,
        )
        // `--effort` is this run's answer, so it shadows a resumed session's
        // saved effort without rewriting it: drop the flag on a later resume
        // and the session's own `/effort` choice is back.
        .reasoning_effort_shadowed(selection.effort.is_some());
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
        rotations,
        headless_rotation,
        credential_pool,
        accounts,
        project,
        inventory,
        agents,
        sessions,
        catalog,
    })
}

/// The exit report's cost line, or `None` when the catalog carries no price
/// entry for the active model.
///
/// Per `usage-accounting`'s "A model the catalog does not price": the exit
/// report prints the token lines and no cost line at all in that case — a
/// bare `None` return, never a price substituted from another model,
/// provider, or a hard-coded default. This differs from `/status`, which
/// reports the same absence as `unknown` rather than omitting it (see
/// `local_command::render_status_cost`); the two surfaces share the
/// `SessionCost` computation but not this presentation choice.
fn render_exit_cost_line(
    usage: &smith_tui::status::SessionUsage,
    price: Option<&smith_tui::status::PriceReference>,
) -> Option<String> {
    let price = price?;
    let cost = smith_tui::status::SessionCost::compute(usage, price);
    Some(format!(
        "{} {} · {}/{}",
        cost.render(),
        cost.label.as_str(),
        price.provider,
        price.model,
    ))
}

/// Prints what the session spent and records it for later comparison.
///
/// Analytics must never be able to fail a session, so a log that cannot be
/// written is dropped rather than surfaced: the user is quitting, and there is
/// nothing useful they could do about it.
///
/// `price` is the identical reference `/status` priced against during the
/// session (see `Status::set_price`), not a fresh catalog lookup performed
/// here — and it is `None` whenever the catalog carries no price entry for
/// the active model. Per `usage-accounting`'s "A model the catalog does not
/// price", that case prints the token lines and no cost line at all: never a
/// price substituted from another model, provider, or a hard-coded default.
/// Cost never reaches [`smith_tui::usage_log::SessionUsageRecord`] below —
/// it is presentation only, printed and discarded, and carries no field
/// there for a price to leak into.
fn report_session_usage(
    host: &HostSession,
    session: &str,
    usage: &smith_tui::status::SessionUsage,
    price: Option<&smith_tui::status::PriceReference>,
) {
    if let Some(line) = usage.render() {
        println!("{line}");
        if let Some(cost_line) = render_exit_cost_line(usage, price) {
            println!("{cost_line}");
        }
    }
    // Printed even for a session that spent nothing: an empty session is
    // exactly the one a user is most likely to want to pick back up, and the
    // id is not recoverable from anywhere else on screen.
    if host.paths().is_some() {
        println!("resume with smith --resume {session}");
    }

    if usage.is_empty() {
        return;
    }
    let policy = host.runtime().policy();
    let record = smith_tui::usage_log::SessionUsageRecord::new(
        session,
        Some(policy.provider_name.clone()),
        policy.model.as_str(),
        policy.agent_profile.clone(),
        usage,
    );
    // Beside this project's session state, so the log inherits whatever
    // directory the user already trusts with their transcripts.
    if let Some(paths) = host.paths() {
        let _ = smith_tui::usage_log::append(
            &smith_tui::usage_log::default_path(paths.directory()),
            &record,
        );
    }
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
                    && reasoning_selection_is_recoverable(&args.selection, resume.is_some()) =>
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
        crate::logging::init(started.host.session().id()).await;
        let StartedHost {
            host,
            approvals,
            interactions,
            project,
            inventory,
            agents,
            sessions,
            catalog,
            rotations,
            credential_pool,
            accounts,
            ..
        } = started;
        let current_session = host.session().id().as_str().to_owned();
        match run_interactive(
            &host,
            InteractiveRequests {
                approvals,
                interactions,
                rotations,
                accounts,
            },
            &project,
            InteractiveResources {
                credential_pool: credential_pool.clone(),
                inventory,
                agents,
                sessions,
                catalog: catalog.clone(),
            },
            PresentationOptions {
                no_color: args.no_color,
                no_motion: args.no_motion,
                reasoning_notice: reasoning_notice.take(),
            },
        )
        .await?
        {
            InteractiveExit::Quit(usage, price) => {
                report_session_usage(&host, &current_session, &usage, price.as_ref());
                return Ok(0);
            }
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

/// Whether an unrepresentable reasoning selection may be cleared and retried.
///
/// The recovery path exists for a selection made against a *different*
/// binding: a saved session override, or an in-session `/think`/`/effort` the
/// rebuild has just landed on a binding that cannot express it. Clearing those
/// and continuing with a notice is what the user would ask for.
///
/// An `--effort` typed on this invocation is deliberately not part of it and
/// is never named here. The flag has its own `Selection` field, so the retry
/// still carries it: a binding that cannot honor the flag fails again on the
/// second attempt with the reasoning diagnostic, and no run can start at an
/// effort nobody asked for. What a retry *can* fix is the saved thinking state
/// beside it, which is worth fixing whether or not a flag was supplied.
pub(super) fn reasoning_selection_is_recoverable(selection: &Selection, resuming: bool) -> bool {
    selection.reasoning_enabled.is_some()
        || selection.reasoning_effort.is_some()
        || (resuming && (!selection.reasoning_enabled_reset || !selection.reasoning_effort_reset))
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
        PaletteCommand::Account(_) => {
            // An account switch is live pool state, not a selection: it needs
            // no runtime rebuild, so it is applied before this point.
            unreachable!("account switches are applied to the live pool, not by rebuilding")
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

#[cfg(test)]
mod tests {
    use smith_tui::status::{PriceReference, PriceTable, SessionUsage};

    use super::*;

    fn usage(reported: bool, tokens: &[(CounterKind, u64)]) -> SessionUsage {
        let mut totals = BTreeMap::new();
        for (kind, value) in tokens {
            totals.insert(*kind, *value);
        }
        SessionUsage {
            turns: 1,
            reported,
            totals,
            ..SessionUsage::default()
        }
    }

    fn price() -> PriceReference {
        PriceReference {
            provider: "openai".to_owned(),
            model: "gpt-5.3".to_owned(),
            table: PriceTable {
                input: Some(2_000_000),
                output: Some(8_000_000),
                cache_read: None,
                cache_write: None,
            },
        }
    }

    #[test]
    fn an_exact_session_prints_one_labelled_figure_naming_its_binding() {
        let usage = usage(true, &[(CounterKind::InputUncached, 1_000_000)]);
        let line = render_exit_cost_line(&usage, Some(&price())).expect("a priced line");
        assert_eq!(line, "$2.000 exact · openai/gpt-5.3");
    }

    #[test]
    fn an_estimated_session_prints_the_estimated_glyph_and_word() {
        let usage = usage(false, &[(CounterKind::InputUncached, 1_000_000)]);
        let line = render_exit_cost_line(&usage, Some(&price())).expect("a priced line");
        assert_eq!(line, "~$2.000 estimated · openai/gpt-5.3");
    }

    #[test]
    fn an_unpriced_model_prints_no_cost_line_at_all() {
        // usage-accounting: "A model the catalog does not price" — the exit
        // report prints the token lines and no cost line, never a price
        // substituted from another model, provider, or a hard-coded
        // default.
        let usage = usage(true, &[(CounterKind::InputUncached, 1_000_000)]);
        assert_eq!(render_exit_cost_line(&usage, None), None);
    }
}
