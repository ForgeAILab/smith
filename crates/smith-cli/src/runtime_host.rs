//! Resolved host construction and interactive restart ownership.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_runtime_core::ids::SessionId;
use anyhow::{Context, Result};
use smith_client::commands::{AdvisorChoice, SelectionCommand};
use smith_config::credential::CredentialResolver;
use smith_config::inventory::{SelectionInventory, local_inventory_with_catalog};
use smith_config::model::{ApprovalMode, ProfileUse};
use smith_config::resolve::{
    AdvisorOverride, AdvisorTarget, Layer, Resolution, ResolvedAgent, resolve,
};
use smith_host::{
    ApprovalRequests, HeadlessApproval, HeadlessInteraction, HeadlessRotation, InteractionRequests,
    InteractiveApproval, InteractiveInteraction, InteractiveRotation, ProjectWorkspace,
    RotationRequests,
};
use smith_runtime::factory::{
    AVAILABLE_ADAPTER_KINDS, AdvisorProfileRequest, ChildProfileRequest, FactoryError, HostSurface,
    RuntimeRequest,
};
use smith_runtime::host::{HostSession, HostSessionError, HostSessionRequest};
use smith_runtime::journal::DefaultRedactor;
use smith_runtime::model_catalog::{CatalogLoader, runtime_catalog_source};
use smith_runtime::pool::CredentialPool;
use smith_runtime::pool_state::ActiveAccounts;
use smith_runtime::rotation::SharedPool;
use smith_runtime::session::SessionListing;

use crate::cli::{RunArgs, Selection};
use crate::config_command::{prepare, resolution_request};
use crate::tui_driver::{
    InteractiveExit, InteractiveRequests, InteractiveResources, PresentationOptions,
    run_interactive,
};
use crate::{MAX_STDIN_PROMPT_BYTES, connection, terminal};

/// How long an interactive start waits for declared servers before opening the
/// prompt without them.
///
/// Sized for a local command's startup, not for an `npx` download: long enough
/// that a normal server is simply *there* on the first turn, short enough that a
/// slow one is never felt.
const INTERACTIVE_MCP_GRACE_MS: u64 = 1_500;

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
    /// Declared MCP servers and their connections, when any are declared.
    pub(super) mcp: Option<Arc<crate::mcp::McpContext>>,
    /// The skills this composition indexed, and the files it refused.
    pub(super) skills: Arc<crate::skills::SkillContext>,
    /// Layered local cache-miss notice policy.
    pub(super) cache_miss_notices: bool,
}

/// Common workspace, credential, and catalog basis for every preflight.
/// Callers add only their surface-specific host services before composition.
pub(super) fn preflight_request(
    resolution: &Resolution,
    project: &std::path::Path,
    surface: HostSurface,
    catalog: Option<Arc<smith_config::catalog::CatalogSnapshot>>,
) -> Result<RuntimeRequest> {
    let workspace = ProjectWorkspace::new(project).map_err(|error| anyhow::anyhow!("{error}"))?;
    let mut request = RuntimeRequest {
        workspace: Some(Arc::new(workspace)),
        credentials: Some(CredentialResolver::new(&resolution.layout.user_dir)),
        model_catalog: catalog.clone(),
        ..RuntimeRequest::new(resolution.config.clone(), surface)
    };
    if let Some(catalog) = catalog
        && let Some(source) = runtime_catalog_source(
            &catalog,
            &request.config.provider.name.value,
            &request.config.provider.kind.value,
            request
                .config
                .provider
                .base_url
                .as_ref()
                .map(|value| value.value.as_str()),
        )
    {
        request.catalog_sources.push(source);
    }
    Ok(request)
}

pub(super) async fn start_host(
    selection: &Selection,
    resume: Option<&str>,
    surface: HostSurface,
    frozen_catalog: Option<Arc<smith_config::catalog::CatalogSnapshot>>,
    mcp: Option<Arc<crate::mcp::McpContext>>,
) -> Result<StartedHost> {
    let prepared = prepare(selection)?;
    let project = prepared.project;
    let resolution = prepared.resolution;
    let cache_miss_notices = resolution.cache_miss_notices.value;
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
    let mut runtime = preflight_request(&resolution, &project, surface, Some(catalog.clone()))
        .context("rooting the project workspace")?;
    runtime.capability_denials = selection.capability_denials.clone();
    // Folded on here rather than inside the factory so that a direct embedder
    // still gets exactly the sources it supplied: discovery is a property of
    // the Smith *host*, which is the only layer that knows a user state root, a
    // project root, and the decisions recorded about them.
    let (skills, skill_context) = crate::skills::SkillContext::compose(
        std::mem::take(&mut runtime.skills),
        &resolution.layout.user_dir,
        &project,
    )
    .context("discovering skills")?;
    runtime.skills = skills;
    let skill_context = Arc::new(skill_context);
    let persistence_redactor = DefaultRedactor::new();
    runtime.persistence_redactor = Some(persistence_redactor.clone());
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
        child_selection.context_window = None;
        child_selection.context_window_reset = false;
        // `--effort` is chosen against the main binding for the same reason,
        // and a child profile may sit on a binding with no effort ladder at
        // all.
        child_selection.effort = None;
        child_selection.context_window_flag = None;
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

    if !matches!(surface, HostSurface::Child)
        && let Some(advisor) = &agents.profile.advisor
    {
        // The advisor route selects its own profile or model; main-session
        // overrides were selected against the main binding, so the reviewer
        // uses its own reasoning and context limits.
        let mut advisor_selection = selection.clone();
        advisor_selection.profile = None;
        advisor_selection.provider = None;
        advisor_selection.model = None;
        advisor_selection.reasoning_enabled = None;
        advisor_selection.reasoning_effort = None;
        advisor_selection.context_window = None;
        advisor_selection.context_window_reset = false;
        advisor_selection.effort = None;
        advisor_selection.context_window_flag = None;
        advisor_selection.advisor = None;
        let (_, advisor_request) = resolution_request(&advisor_selection)?;
        let advisor_resolution = resolve(&advisor_request.with_advisor_route(advisor.clone()))
            .map_err(|error| anyhow::anyhow!("{error}"))
            .with_context(|| format!("resolving advisor `{}`", advisor.value))?;
        let mut catalog_sources = Vec::new();
        if let Some(source) = runtime_catalog_source(
            &catalog,
            &advisor_resolution.config.provider.name.value,
            &advisor_resolution.config.provider.kind.value,
            advisor_resolution
                .config
                .provider
                .base_url
                .as_ref()
                .map(|value| value.value.as_str()),
        ) {
            catalog_sources.push(source);
        }
        runtime.advisor_profile = Some(AdvisorProfileRequest {
            config: advisor_resolution.config,
            catalog_sources,
            provider: None,
        });
    }

    // Connections start here, beside the rest of session start, and nothing
    // below waits for them: a server that takes a minute to install itself
    // must not be able to delay the prompt.
    let mcp = match mcp {
        Some(context) => Some(context),
        None => crate::mcp::McpContext::start(
            &resolution.config,
            &resolution.layout.user_dir,
            &project,
            runtime.credentials.clone(),
        )
        .context("planning the declared MCP servers")?,
    };
    if let Some(context) = &mcp {
        runtime.mcp = Some(context.supervisor());
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
        .context_window_reset(selection.context_window_reset)
        // `--effort` is this run's answer, so it shadows a resumed session's
        // saved effort without rewriting it: drop the flag on a later resume
        // and the session's own `/effort` choice is back.
        .reasoning_effort_shadowed(selection.effort.is_some())
        .context_window_shadowed(selection.context_window_flag.is_some());
    if let Some(session) = resume {
        request = request.resume(SessionId::new(session));
    }
    // Two budgets, for two different costs of waiting.
    //
    // A one-shot run has no later boundary to pick a server's tools up at, so
    // it waits for the full startup timeout: a `-p` run that silently dropped
    // a configured server's tools would be worse than a slow one.
    //
    // An interactive run waits only long enough for a *local* server to come
    // up. Nearly every MCP server is a local command that answers in
    // milliseconds, and paying a short grace here means the common case gets
    // its tools on turn one and never crosses the rebuild boundary at all. A
    // server slower than the grace still cannot delay the prompt: the wait
    // ends, the session starts without it, and its tools join at the next idle
    // boundary.
    if let Some(context) = &mcp {
        let budget = match surface {
            HostSurface::Terminal => INTERACTIVE_MCP_GRACE_MS,
            HostSurface::Headless | HostSurface::Child => {
                smith_runtime::mcp::DEFAULT_STARTUP_TIMEOUT_MS
            }
        };
        context
            .supervisor()
            .settle(Duration::from_millis(budget))
            .await;
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
        mcp,
        skills: skill_context,
        cache_miss_notices,
    })
}

/// The exit report's cost line, or `None` when the catalog carries no price
/// entry for any contributing binding.
///
/// Per `usage-accounting`'s "A model the catalog does not price": the exit
/// report prints the token lines and no cost line at all in that case — a
/// bare `None` return, never a price substituted from another model,
/// provider, or a hard-coded default. This differs from `/status`, which
/// reports the same absence as `unknown` rather than omitting it (see
/// `local_command::render_status_cost`); the two surfaces share the
/// `SessionCost` computation but not this presentation choice.
fn render_exit_cost_line(
    usage: &smith_client::status::SessionUsage,
    price: Option<&smith_client::status::PriceReference>,
) -> Option<String> {
    let price = usage.cost_price(price)?;
    let cost = smith_client::status::SessionCost::compute(usage, price);
    Some(format!(
        "{} {} · {}",
        cost.render(),
        cost.label.as_str(),
        price.render_sources(usage),
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
/// here. Earlier root and child references travel with `usage`, so an
/// unpriced last model cannot hide their spend. When no contributing binding
/// has a price, the report prints token lines without a cost line.
/// Cost never reaches [`smith_client::usage_log::SessionUsageRecord`] below —
/// it is presentation only, printed and discarded, and carries no field
/// there for a price to leak into.
fn report_session_usage(
    host: &HostSession,
    session: &str,
    usage: &smith_client::status::SessionUsage,
    price: Option<&smith_client::status::PriceReference>,
    cache: Option<&smith_client::cache::CacheTurnSummary>,
) {
    if let Some(line) = usage.render() {
        println!("{line}");
        if let Some(cost_line) = render_exit_cost_line(usage, price) {
            println!("{cost_line}");
        }
    }
    if let Some(line) = cache.and_then(smith_client::cache::CacheTurnSummary::render_usage) {
        println!("{line}");
    }
    if let Some(controller) = host.cache_lifecycle()
        && !controller.synthetic_attempts.is_empty()
    {
        println!(
            "{}",
            crate::local_command::render_cache_controller_summary(&controller)
        );
    }
    // An untouched session has no work to resume. Canonical user history,
    // rather than spend, preserves the hint for prompts that failed early.
    if let Some(line) = session_resume_hint(host) {
        println!("{line}");
    }

    if usage.is_empty() {
        return;
    }
    let policy = host.runtime().policy();
    let record = smith_client::usage_log::SessionUsageRecord::new(
        session,
        Some(policy.provider_name.clone()),
        policy.model.as_str(),
        policy.agent_profile.clone(),
        usage,
    );
    // Beside this project's session state, so the log inherits whatever
    // directory the user already trusts with their transcripts.
    if let Some(paths) = host.paths() {
        let _ = smith_client::usage_log::append(
            &smith_client::usage_log::default_path(paths.directory()),
            &record,
        );
    }
}

/// Checks canonical user history because neither usage nor a text preview can
/// establish whether failed or image-only prompts were submitted.
pub(super) fn session_resume_hint(host: &HostSession) -> Option<String> {
    if host.paths().is_none()
        || !host
            .session()
            .history()
            .iter()
            .any(|message| message.role == agent_runtime_core::content::Role::User)
    {
        return None;
    }
    Some(format!(
        "resume with smith --resume {}",
        host.session().id()
    ))
}

/// Joins host writers before deletion so a late catalog save cannot recreate
/// an empty session. Canonical history preserves failed and image-only prompts;
/// cleanup is best effort so a filesystem error never prevents the terminal
/// surface from exiting.
pub(super) async fn remove_empty_interactive_session(host: &HostSession) {
    if !host
        .session()
        .history()
        .iter()
        .any(|message| message.role == agent_runtime_core::content::Role::User)
        && let Some(paths) = host.paths()
    {
        let _ = host.shutdown().await;
        let _ = paths.remove_session_files(host.session().id()).await;
    }
}

pub(super) async fn run_interactive_command(
    args: RunArgs,
    setup_notice: Option<String>,
) -> Result<u8> {
    let mut terminal = None;
    let result = run_interactive_hosts(args, setup_notice, &mut terminal).await;
    if let Some(terminal) = terminal.as_mut() {
        terminal.restore().context("restoring the terminal")?;
    }
    result
}

async fn run_interactive_hosts(
    mut args: RunArgs,
    mut host_notice: Option<String>,
    terminal: &mut Option<terminal::Terminal>,
) -> Result<u8> {
    let mut resume = args.resume.take();
    let mut frozen_catalog = None;
    let mut reasoning_notice = None;
    // The advisor in effect before a pending `/advisor` change, restored if
    // the rebuilt session cannot start with the new one.
    let mut advisor_change: Option<Option<AdvisorOverride>> = None;
    // Likewise the session's denials before a pending `/capabilities` change.
    let mut capability_change: Option<Vec<String>> = None;
    let mut mcp: Option<Arc<crate::mcp::McpContext>> = None;
    let mut app = None;
    // Initialize after raw mode is entered, then retain input through host
    // shutdown, startup retries, and embedded connection steps.
    let mut keys = None;
    loop {
        let started = match start_host(
            &args.selection,
            resume.as_deref(),
            HostSurface::Terminal,
            frozen_catalog.clone(),
            mcp.clone(),
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
            Err(error) if advisor_change.is_some() => {
                args.selection.advisor = advisor_change.take().flatten();
                host_notice = Some(format!("advisor unchanged · {error:#}"));
                continue;
            }
            Err(error) if capability_change.is_some() => {
                args.selection.capability_denials = capability_change.take().unwrap_or_default();
                host_notice = Some(format!("capabilities unchanged · {error:#}"));
                continue;
            }
            Err(error) => return Err(error),
        };
        if capability_change.take().is_some() {
            host_notice = Some(if args.selection.capability_denials.is_empty() {
                "capabilities · no session denials".to_owned()
            } else {
                format!(
                    "capabilities · denied for this session: {}",
                    args.selection.capability_denials.join(", ")
                )
            });
        }
        if advisor_change.take().is_some() {
            host_notice = Some(format!(
                "advisor {}",
                crate::resources::advisor_status(&started.agents)
            ));
        }
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
            mcp: started_mcp,
            skills,
            cache_miss_notices,
            ..
        } = started;
        mcp = started_mcp;
        let current_session = host.session().id().as_str().to_owned();
        if terminal.is_none() {
            match terminal::enter() {
                Ok(entered) => *terminal = Some(entered),
                Err(error) => {
                    let _ = host.shutdown().await;
                    return Err(error).context("entering the alternate screen");
                }
            }
        }
        let terminal = terminal.as_mut().expect("entered terminal");
        let keys = keys.get_or_insert_with(crate::screen_runner::terminal_events);
        let (exit, retained) = run_interactive(
            terminal,
            app.take(),
            &host,
            InteractiveRequests {
                keys: keys.as_mut(),
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
                mcp: mcp.clone(),
                skills,
                capability_denials: args.selection.capability_denials.clone(),
            },
            PresentationOptions {
                no_color: args.no_color,
                no_motion: args.no_motion,
                reasoning_notice: reasoning_notice.take(),
                host_notice: host_notice.take(),
                cache_miss_notices,
            },
        )
        .await?;
        app = Some(retained);
        match exit {
            InteractiveExit::Quit(usage, price, cache) => {
                terminal.restore().context("restoring the terminal")?;
                report_session_usage(
                    &host,
                    &current_session,
                    &usage,
                    price.as_ref(),
                    cache.as_deref(),
                );
                remove_empty_interactive_session(&host).await;
                return Ok(0);
            }
            // The same identity, recomposed around the tools a server
            // contributed after this session started.
            InteractiveExit::CapabilitiesChanged => {
                resume = Some(current_session);
                frozen_catalog = Some(catalog);
                continue;
            }
            InteractiveExit::Connect(provider) => {
                resume = Some(current_session);
                frozen_catalog = None;
                if let Some(retained) = &mut app {
                    let result = {
                        let mut session = crate::screen_runner::ScreenSession::embedded(
                            terminal,
                            &retained.app,
                            keys.as_mut(),
                            args.no_color,
                            args.no_motion,
                        );
                        connection::connect(
                            args.selection.clone(),
                            &provider,
                            &mut session,
                            args.no_motion,
                        )
                        .await
                    };
                    if let Ok(outcome) = &result
                        && outcome.outcome == crate::setup::SetupOutcome::Cancelled
                    {
                        frozen_catalog = Some(catalog);
                    }
                    connection::push_notices(
                        &mut retained.app,
                        result.map(|outcome| outcome.messages),
                    );
                }
                continue;
            }
            InteractiveExit::Disconnect(provider) => {
                let result = connection::disconnect(&args.selection, &provider).await;
                match result {
                    Ok(result)
                        if result.outcome
                            == connection::DisconnectOutcome::ActiveDirectProvider =>
                    {
                        terminal.restore().context("restoring the terminal")?;
                        for message in result.messages {
                            println!("{message}");
                        }
                        if session_resume_hint(&host).is_some() {
                            println!(
                                "The active provider was disconnected. The session is saved; restart Smith with a connected provider to resume it."
                            );
                        } else {
                            println!(
                                "The active provider was disconnected. Restart Smith with a connected provider."
                            );
                        }
                        remove_empty_interactive_session(&host).await;
                        return Ok(0);
                    }
                    result => {
                        if let Some(retained) = &mut app {
                            connection::push_notices(
                                &mut retained.app,
                                result.map(|result| result.messages),
                            );
                        }
                    }
                }
                resume = Some(current_session);
                frozen_catalog = None;
                continue;
            }
            InteractiveExit::Reconfigure(command) => {
                frozen_catalog = matches!(
                    &command,
                    SelectionCommand::Profile(_)
                        | SelectionCommand::Model { .. }
                        | SelectionCommand::Agent(_)
                        | SelectionCommand::Think(_)
                        | SelectionCommand::Effort(_)
                        | SelectionCommand::ContextWindow(_)
                        | SelectionCommand::Advisor(_)
                        | SelectionCommand::CapabilityDeny(_)
                        | SelectionCommand::CapabilityAllow(_)
                )
                .then_some(catalog);
                if matches!(
                    command,
                    SelectionCommand::NewSession | SelectionCommand::Resume(_)
                ) {
                    remove_empty_interactive_session(&host).await;
                }
                if matches!(command, SelectionCommand::Advisor(_)) {
                    advisor_change = Some(args.selection.advisor.clone());
                }
                if matches!(
                    command,
                    SelectionCommand::CapabilityDeny(_) | SelectionCommand::CapabilityAllow(_)
                ) {
                    capability_change = Some(args.selection.capability_denials.clone());
                }
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
        ) || matches!(
            source.downcast_ref::<HostSessionError>(),
            Some(HostSessionError::Factory(FactoryError::Reasoning { .. }))
        )
    })
}

pub(super) fn apply_palette_command(
    selection: &mut Selection,
    resume: &mut Option<String>,
    current_session: String,
    command: SelectionCommand,
) {
    match command {
        // The advisor choice belongs to the session it was made in.
        SelectionCommand::NewSession => {
            selection.advisor = None;
            selection.capability_denials.clear();
            *resume = None;
        }
        SelectionCommand::Resume(session) => {
            selection.advisor = None;
            selection.capability_denials.clear();
            *resume = Some(session);
        }
        SelectionCommand::Profile(profile) => {
            selection.profile = Some(profile);
            selection.provider = None;
            selection.model = None;
            *resume = Some(current_session);
        }
        SelectionCommand::Model { provider, model } => {
            // A provider-served model narrows the selection to that pair. An
            // installed CLI agent has none: it runs the turn itself, and the
            // profile stays selected because its provider is still what
            // supplies model identity and the limits planned against.
            if let Some(provider) = provider {
                selection.profile = None;
                selection.provider = Some(provider);
            }
            selection.model = Some(model);
            *resume = Some(current_session);
        }
        SelectionCommand::Agent(agent) => {
            selection.agent = Some(agent);
            *resume = Some(current_session);
        }
        SelectionCommand::Think(enabled) => {
            selection.reasoning_enabled = enabled;
            selection.reasoning_enabled_reset = enabled.is_none();
            *resume = Some(current_session);
        }
        SelectionCommand::Effort(effort) => {
            selection.reasoning_effort = effort;
            selection.reasoning_effort_reset = selection.reasoning_effort.is_none();
            *resume = Some(current_session);
        }
        SelectionCommand::ContextWindow(window) => {
            selection.context_window_reset = window.is_none();
            selection.context_window = window;
            *resume = Some(current_session);
        }
        SelectionCommand::CapabilityDeny(pattern) => {
            if !selection.capability_denials.contains(&pattern) {
                selection.capability_denials.push(pattern);
            }
            *resume = Some(current_session);
        }
        SelectionCommand::CapabilityAllow(pattern) => {
            selection
                .capability_denials
                .retain(|denied| *denied != pattern);
            *resume = Some(current_session);
        }
        SelectionCommand::Advisor(choice) => {
            selection.advisor = match choice {
                AdvisorChoice::Default => None,
                AdvisorChoice::Off => Some(AdvisorOverride::Off),
                // The picker offers only parseable targets; an unparseable
                // one leaves the configured advisor in place.
                AdvisorChoice::Target(target) => {
                    AdvisorTarget::parse(&target).map(AdvisorOverride::Target)
                }
            };
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
#[allow(clippy::wildcard_imports)]
mod tests {
    use std::collections::BTreeMap;

    use agent_runtime_core::usage::CounterKind;
    use smith_client::status::{PriceReference, PriceTable, SessionUsage};

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

    #[test]
    fn binding_cost_lines_match_status_and_keep_single_binding_bytes() {
        let mut status = smith_client::status::Status::new("gpt-5.3", "project");
        status.switch_model(Some("openai".into()), "gpt-5.3");
        status.set_price(Some(price()));
        status.record_usage(
            &agent_runtime_core::usage::UsageDelta::new()
                .with(CounterKind::InputUncached, 1_000_000),
        );
        let usage = status.session_usage();
        assert_eq!(
            render_exit_cost_line(&usage, status.price()).as_deref(),
            Some("$2.000 exact · openai/gpt-5.3")
        );
        assert_eq!(
            crate::local_command::render_status_cost(&usage, status.price(), ("openai", "gpt-5.3")),
            "$2.000 exact · openai/gpt-5.3"
        );
        let mut estimated = usage.clone();
        estimated.bindings[0].reported = false;
        assert_eq!(
            render_exit_cost_line(&estimated, status.price()).as_deref(),
            Some("~$2.000 estimated · openai/gpt-5.3")
        );
        assert_eq!(
            crate::local_command::render_status_cost(
                &estimated,
                status.price(),
                ("openai", "gpt-5.3")
            ),
            "~$2.000 estimated · openai/gpt-5.3"
        );
        let mut advisor = usage;
        advisor
            .advisor_totals
            .insert(CounterKind::InputUncached, 1_000_000);
        advisor.advisor_price = Some(PriceReference {
            provider: "advisor".into(),
            model: "reviewer".into(),
            ..price()
        });
        assert_eq!(
            render_exit_cost_line(&advisor, status.price()).as_deref(),
            Some("$4.000 exact · openai/gpt-5.3 · advisor advisor/reviewer")
        );
        assert_eq!(
            crate::local_command::render_status_cost(
                &advisor,
                status.price(),
                ("openai", "gpt-5.3")
            ),
            "$4.000 exact · openai/gpt-5.3 · advisor advisor/reviewer"
        );
        advisor.advisor_price = None;
        assert_eq!(
            render_exit_cost_line(&advisor, status.price()).as_deref(),
            Some("~$2.000 estimated · openai/gpt-5.3 · advisor price unknown")
        );
        assert_eq!(
            crate::local_command::render_status_cost(
                &advisor,
                status.price(),
                ("openai", "gpt-5.3")
            ),
            "~$2.000 estimated · openai/gpt-5.3 · advisor price unknown"
        );
    }

    #[test]
    fn cost_lines_show_each_binding_even_when_the_last_root_price_is_unknown() {
        use agent_runtime_core::usage::UsageDelta;
        use smith_client::status::{BindingUsage, Status};
        let mut status = Status::new("glm-5.3", "project");
        status.switch_model(Some("zai".into()), "glm-5.3");
        status.set_price(Some(PriceReference {
            provider: "zai".into(),
            model: "glm-5.3".into(),
            ..price()
        }));
        status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 11_000));
        status.switch_model(Some("google".into()), "gemini-3.8-flash");
        status.set_price(Some(PriceReference {
            provider: "google".into(),
            model: "gemini-3.8-flash".into(),
            table: PriceTable {
                input: Some(1_000_000),
                ..price().table
            },
        }));
        status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 12_000));
        let usage = status.session_usage();
        let expected = "$0.034 exact · zai/glm-5.3 $0.022 · google/gemini-3.8-flash $0.012";
        assert_eq!(
            render_exit_cost_line(&usage, status.price()).as_deref(),
            Some(expected)
        );
        assert_eq!(
            crate::local_command::render_status_cost(
                &usage,
                status.price(),
                ("google", "gemini-3.8-flash")
            ),
            expected
        );

        status.switch_model(Some("custom".into()), "model");
        status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 999_999));
        let usage = status.session_usage();
        let expected = "~$0.034 estimated · zai/glm-5.3 $0.022 · google/gemini-3.8-flash $0.012 · price unknown for custom/model";
        assert_eq!(
            render_exit_cost_line(&usage, None).as_deref(),
            Some(expected)
        );
        assert_eq!(
            crate::local_command::render_status_cost(&usage, None, ("custom", "model")),
            expected
        );

        let unknown = SessionUsage {
            bindings: vec![BindingUsage {
                totals: BTreeMap::from([(CounterKind::InputUncached, 10)]),
                ..BindingUsage::new(Some("custom".into()), "model", None)
            }],
            totals: BTreeMap::from([(CounterKind::InputUncached, 10)]),
            ..SessionUsage::default()
        };
        assert_eq!(render_exit_cost_line(&unknown, Some(&price())), None);
        assert_eq!(
            crate::local_command::render_status_cost(&unknown, Some(&price()), ("custom", "model")),
            "unknown · no price reference for custom/model"
        );
    }
}
