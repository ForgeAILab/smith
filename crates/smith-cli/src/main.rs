//! The `smith` composition root.
//!
//! Both terminal and one-prompt runs resolve the same configuration, inject
//! the same project/credential policy, and start through
//! [`smith_runtime::host`]. Presentation begins only after that preflight and
//! the session restore have succeeded.

mod cli;
mod headless;
mod interaction;
mod setup;
mod terminal;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use agent_runtime_core::cancel::CancelReason;
use agent_runtime_core::content::{ContentPart, ToolResultBlock, UserInput};
use agent_runtime_core::delegation::{
    ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
};
use agent_runtime_core::event::{EstimationConfidence, EventEnvelope, RuntimeEvent, TurnFinish};
use agent_runtime_core::goal::{GoalCommand, GoalProjection};
use agent_runtime_core::ids::{ChildId, SessionId};
use agent_runtime_core::provider::{ModelId, ReasoningSupport};
use agent_runtime_core::usage::CounterKind;
use anyhow::{Context, Result};
use cli::{Command, Prompt, RunArgs, Selection};
use crossterm::event::{Event as TermEvent, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ignore::WalkBuilder;
use ratatui::layout::Rect;
use smith_config::credential::CredentialResolver;
use smith_config::inventory::{
    InventoryLimit, ModelLimitOrigin, SelectionInventory, local_inventory_with_catalog,
};
use smith_config::model::{ApprovalMode, ProfileUse};
use smith_config::resolve::{
    ConfigReadiness, Layer, Resolution, ResolveRequest, ResolvedAgent, inspect, resolve,
};
use smith_host::{
    ApprovalPrompt, ApprovalRequests, GitChanges, HeadlessApproval, HeadlessInteraction,
    InteractionRequests, InteractiveApproval, InteractiveInteraction, ProjectWorkspace,
};
use smith_runtime::factory::{
    AVAILABLE_ADAPTER_KINDS, ChildProfileRequest, FactoryError, HostSurface, RuntimePolicy,
    RuntimeRequest,
};
use smith_runtime::host::{HostSession, HostSessionRequest};
use smith_runtime::journal::DefaultRedactor;
use smith_runtime::model_catalog::{CatalogLoader, runtime_catalog_source};
use smith_runtime::session::{SNAPSHOT_SCHEMA_VERSION, SessionListing};
use smith_runtime::{ChildDurability, ChildState, ChildStatus, SpawnOutcome};
use smith_tui::app::{Action, App, LEGACY_AGENT_PROFILE_PREFIX, PaletteCommand};
use smith_tui::commands::{CommandAction, GoalAction};
#[cfg(test)]
use smith_tui::status::ContextPlanUpdate;
use smith_tui::status::{Status, TokenCount, render_elapsed};
use smith_tui::theme::{Theme, glyph};
use smith_tui::{
    PickerOutcome, ResourceEntry, ResourcePicker, RuntimeResources, draw_resource_picker,
};

/// The frame budget: `DESIGN.md` §6 caps redraws at 30 fps.
const FRAME: Duration = Duration::from_millis(33);

/// The spinner advances every 100 ms, independently of the frame rate.
const SPINNER_TICK: Duration = Duration::from_millis(100);

/// A piped prompt is bounded before it can consume process memory. The runtime
/// applies the model-specific token budget later.
const MAX_STDIN_PROMPT_BYTES: usize = 1024 * 1024;

#[tokio::main]
async fn main() -> ExitCode {
    let command = match cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("smith: {error}");
            eprintln!("Try `smith --help` for usage.");
            return ExitCode::from(2);
        }
    };

    match execute(command).await {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("smith: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn execute(command: Command) -> Result<u8> {
    match command {
        Command::Help => {
            print!("{}", cli::HELP);
            Ok(0)
        }
        Command::Version => {
            println!("smith {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Command::ConfigExplain { key, selection } => {
            explain_config(&key, &selection)?;
            Ok(0)
        }
        Command::SessionsList { selection } => {
            list_sessions(&selection).await?;
            Ok(0)
        }
        Command::Setup(args) => {
            setup::run_explicit(args).await?;
            Ok(0)
        }
        Command::Run(args) => run_command(args).await,
    }
}

async fn run_command(mut args: RunArgs) -> Result<u8> {
    match inspect_selection(&args.selection)? {
        ConfigReadiness::Ready(_) => {}
        ConfigReadiness::Invalid(error) => {
            return Err(anyhow::anyhow!("{error}")).context("resolving Smith configuration");
        }
        ConfigReadiness::Unconfigured(_) => {
            let interactive = args.prompt.is_none()
                && std::io::stdin().is_terminal()
                && std::io::stdout().is_terminal()
                && std::io::stderr().is_terminal();
            if !interactive {
                anyhow::bail!(
                    "Smith has no configured provider/model. Run `smith setup` in an interactive \
                     terminal, or supply a complete provider, model, and limits through config"
                );
            }
            match setup::run_first_run(args.selection.clone(), args.no_color, args.no_motion)
                .await?
            {
                setup::SetupOutcome::Cancelled => return Ok(0),
                setup::SetupOutcome::Completed => {}
            }
        }
    }

    if args.resume_requested && args.resume.is_none() {
        if args.prompt.is_some() {
            anyhow::bail!(
                "bare `--resume` needs an interactive terminal; use `smith sessions list` and \
                 pass `--resume <SESSION_ID>` for a headless run"
            );
        }
        if !std::io::stdin().is_terminal()
            || !std::io::stdout().is_terminal()
            || !std::io::stderr().is_terminal()
        {
            anyhow::bail!(
                "bare `--resume` needs an interactive terminal; use `smith sessions list` or \
                 pass `--resume <SESSION_ID>`"
            );
        }
        args.resume =
            match choose_resume_session(&args.selection, args.no_color, args.no_motion).await? {
                Some(session) => Some(session),
                None => return Ok(0),
            };
    }

    let prompt = match args.prompt.take() {
        Some(Prompt::Argument(prompt)) => Some(prompt),
        Some(Prompt::Stdin) => Some(read_prompt(std::io::stdin().lock())?),
        None => None,
    };

    match prompt {
        Some(prompt) => {
            let started = start_host(
                &args.selection,
                args.resume.as_deref(),
                HostSurface::Headless,
                None,
            )
            .await?;
            headless::run(
                &started.host,
                prompt,
                args.output,
                started.headless_approval.as_deref(),
                started.headless_interaction.as_deref(),
            )
            .await
            .map(|outcome| outcome.exit_code)
        }
        None => run_interactive_command(args).await,
    }
}

struct StartedHost {
    host: HostSession,
    approvals: Option<ApprovalRequests>,
    headless_approval: Option<Arc<HeadlessApproval>>,
    interactions: Option<InteractionRequests>,
    headless_interaction: Option<Arc<HeadlessInteraction>>,
    project: PathBuf,
    inventory: SelectionInventory,
    agents: ResolvedAgent,
    sessions: Vec<SessionListing>,
    catalog: Arc<smith_config::catalog::CatalogSnapshot>,
}

async fn start_host(
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

async fn run_interactive_command(mut args: RunArgs) -> Result<u8> {
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

fn is_reasoning_startup_error(error: &anyhow::Error) -> bool {
    error.chain().any(|source| {
        matches!(
            source.downcast_ref::<FactoryError>(),
            Some(FactoryError::Reasoning { .. })
        )
    })
}

fn apply_palette_command(
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

fn read_prompt(reader: impl Read) -> Result<String> {
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

enum InteractiveExit {
    Quit,
    Reconfigure(PaletteCommand),
}

struct PresentationOptions {
    no_color: bool,
    no_motion: bool,
    reasoning_notice: Option<String>,
}

struct InteractiveResources {
    inventory: SelectionInventory,
    agents: ResolvedAgent,
    sessions: Vec<SessionListing>,
}

async fn run_interactive(
    host: &HostSession,
    approvals: Option<ApprovalRequests>,
    interactions: Option<InteractionRequests>,
    project: &std::path::Path,
    resources: InteractiveResources,
    presentation: PresentationOptions,
) -> Result<InteractiveExit> {
    let InteractiveResources {
        inventory,
        agents,
        sessions,
    } = resources;
    let policy = host.runtime().policy();
    let snapshot = host.session().snapshot();
    let project_label = GitChanges::discover(project)
        .and_then(|git| git.branch_label())
        .map_or_else(
            |_| abbreviate_home(&project.to_string_lossy()),
            |branch| format!("{}:{branch}", abbreviate_home(&project.to_string_lossy())),
        );
    let mut app = App::new(policy.model.as_str(), project_label);
    app.status
        .switch_model(Some(policy.provider_name.clone()), policy.model.as_str());
    app.status.set_agent(policy.agent_profile.clone());
    match host.goal() {
        Ok(goal) => app.status.set_goal(goal),
        Err(error) => app
            .transcript
            .push_error(format!("persistent goal state unavailable: {error}")),
    }
    app.status
        .set_reasoning_hint(policy.reasoning.has_override().then(|| {
            format!(
                "think {} · effort {}",
                policy.reasoning.effective_state(),
                policy.reasoning.effective_effort(),
            )
        }));
    if policy.reasoning.has_override() {
        app.transcript.push_notice(
            "reasoning",
            format!(
                "thinking {} · effort {} · {} · applies to the next turn",
                policy.reasoning.effective_state(),
                policy.reasoning.effective_effort(),
                policy.reasoning.selection_source,
            ),
        );
    }
    if let Some(notice) = presentation.reasoning_notice {
        app.transcript.push_notice("reasoning", notice);
    }
    app.set_resources(runtime_resources(
        inventory,
        sessions,
        host.session().id().as_str(),
        project,
        &agents,
        &policy.reasoning,
    ));
    app.transcript.replace_from_history(&snapshot.history);
    for (call, display) in host.tool_call_displays() {
        app.set_tool_display(call.as_str(), display);
    }
    for (call, text) in host.tool_result_texts() {
        app.set_tool_result_preview(call.as_str(), text);
    }
    if let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
    {
        for child in coordinator.list() {
            let (state, detail) = child_summary_projection(&child);
            app.restore_child(child.child.as_str(), state, Some(detail));
        }
    }
    if let Some(interruption) = host.recovered_ephemeral_work() {
        app.present_recovered_ephemeral_work(
            interruption.children.len(),
            interruption.monitors.len(),
        );
    }
    let usage = snapshot.usage.total();
    if !usage.is_empty() {
        app.status.record_usage(&usage);
        let cache_read = usage.get(CounterKind::InputCached);
        if cache_read > 0 {
            app.status.record_cache(cache_read);
        }
    }
    if let Some(previous) = snapshot.manifests.last().map(|entry| &entry.manifest.model)
        && (previous.provider != policy.provider_name || previous.model != policy.model)
    {
        app.transcript.push_notice(
            "provider",
            format!(
                "changed · {}/{} → {}/{} · prior cache not transferable",
                previous.provider, previous.model, policy.provider_name, policy.model
            ),
        );
        // The aggregate snapshot usage belongs to the prior provider/model.
        // Keep its magnitude for context, but stop presenting it as a current
        // provider report and clear cache evidence.
        app.status
            .switch_model(Some(policy.provider_name.clone()), policy.model.as_str());
    }

    let mut terminal = match terminal::enter() {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = host.shutdown().await;
            return Err(error).context("entering the alternate screen");
        }
    };
    let mut theme = Theme::from_env();
    if presentation.no_color {
        theme = theme.without_color();
    }
    if presentation.no_motion {
        theme = theme.without_motion();
    }
    let run_result = run_tui(
        &mut terminal,
        app,
        TuiRunInputs {
            host,
            project,
            approvals,
            interactions,
            agents: &agents,
            theme,
        },
    )
    .await;
    let restore_result = terminal.restore().context("restoring the terminal");
    let shutdown_result = host
        .shutdown()
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("shutting the session down");

    restore_result?;
    shutdown_result?;
    run_result
}

struct TuiRunInputs<'a> {
    host: &'a HostSession,
    project: &'a std::path::Path,
    approvals: Option<ApprovalRequests>,
    interactions: Option<InteractionRequests>,
    agents: &'a ResolvedAgent,
    theme: Theme,
}

async fn run_tui(
    terminal: &mut terminal::Terminal,
    mut app: App,
    inputs: TuiRunInputs<'_>,
) -> Result<InteractiveExit> {
    let TuiRunInputs {
        host,
        project,
        mut approvals,
        interactions,
        agents,
        theme,
    } = inputs;
    let session = host.session();
    let mut events = session.subscribe();
    let mut keys = EventStream::new();
    let mut spinner = tokio::time::interval(SPINNER_TICK);
    let mut frame = tokio::time::interval(FRAME);
    let (local_tx, mut local_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut last_change_turn = host.changes().latest().map(|set| set.turn);
    let mut interactions = interaction::InteractionSurface::new(
        interactions,
        host.restored_interaction()
            .map(|restored| restored.request_id().as_str().to_owned()),
    );
    let mut dirty = true;

    let exit = loop {
        tokio::select! {
            // Keyboard first: a provider flood must not starve cancellation.
            biased;

            Some(key) = keys.next() => {
                match key.context("reading a terminal event")? {
                    // `Ctrl+V` is the explicit "attach from clipboard" chord:
                    // terminals deliver ordinary pastes as bracketed text, but
                    // an image on the clipboard can only be fetched by asking
                    // the platform directly.
                    TermEvent::Key(key)
                        if key.kind != KeyEventKind::Release
                            && key.code == KeyCode::Char('v')
                            && key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        attach_from_clipboard(&mut app);
                        dirty = true;
                    }
                    TermEvent::Key(key) => {
                        match app.on_key(key) {
                            Some(Action::Send(text)) => {
                                if let Err(error) = session.send(UserInput::text(text)) {
                                    app.transcript.push_error(format!(
                                        "turn submission was rejected: {error}"
                                    ));
                                }
                            }
                            Some(Action::SendWithImages { text, images }) => {
                                let mut parts = vec![ContentPart::text(text)];
                                parts.extend(images.into_iter().map(|url| {
                                    ContentPart::Image { url, detail: None }
                                }));
                                if let Err(error) = session.send(UserInput { parts }) {
                                    app.transcript.push_error(format!(
                                        "turn submission was rejected: {error}"
                                    ));
                                }
                            }
                            Some(Action::SendWithFiles { text, files }) => {
                                start_prepared_send(
                                    session.clone(),
                                    text,
                                    files,
                                    host.runtime().policy().turn_time_limit_ms.unwrap_or(600_000),
                                    local_tx.clone(),
                                );
                            }
                            Some(Action::RunShell { command }) => {
                                start_local_shell(
                                    session.clone(),
                                    command,
                                    host.runtime().policy().turn_time_limit_ms.unwrap_or(600_000),
                                    local_tx.clone(),
                                );
                            }
                            Some(Action::Interrupt) => {
                                if let Err(error) = session
                                    .interrupt_current_turn(CancelReason::UserRequested)
                                {
                                    app.transcript.push_error(format!(
                                        "turn interruption failed: {error}"
                                    ));
                                }
                            }
                            Some(Action::Quit) => break InteractiveExit::Quit,
                            Some(Action::Reconfigure(command)) => {
                                break InteractiveExit::Reconfigure(command);
                            }
                            Some(Action::Command(command)) => {
                                handle_local_command(&mut app, host, project, command).await;
                            }
                            Some(Action::ApplyUndo) => match host.changes().undo_latest() {
                                Ok(()) => app.transcript.push_notice(
                                    "undo",
                                    "last attributable Smith turn was restored",
                                ),
                                Err(error) => app.transcript.push_error(error.message),
                            },
                            Some(Action::CancelUndo) => {
                                host.changes().record_undo_cancelled();
                                app.transcript.push_notice("undo", "cancelled");
                            }
                            Some(Action::ApplyRedo) => match host.changes().redo_latest() {
                                Ok(()) => app.transcript.push_notice(
                                    "redo",
                                    "newest exact undone Smith turn was reapplied",
                                ),
                                Err(error) => app.transcript.push_error(error.message),
                            },
                            Some(Action::CancelRedo) => {
                                host.changes().record_redo_cancelled();
                                app.transcript.push_notice("redo", "cancelled");
                            }
                            Some(Action::ApplyRevert { scope, fingerprint }) => {
                                let recovery_dir = host.paths().map(|paths| {
                                    paths
                                        .directory()
                                        .join("recovery")
                                        .join(host.session().id().as_str())
                                });
                                match GitChanges::discover(project).and_then(|git| {
                                    git.apply_revert(
                                        &scope,
                                        &fingerprint,
                                        recovery_dir.as_deref(),
                                    )
                                }) {
                                    Ok(applied) => {
                                        host.changes().record_revert_event(
                                            &scope,
                                            &fingerprint,
                                            "applied",
                                        );
                                        host.changes().record_recovery(
                                            applied.path,
                                            applied.before,
                                            applied.after,
                                            "revert",
                                            applied.recovery_path,
                                        );
                                        app.transcript.push_notice(
                                            "revert",
                                            format!(
                                                "`{scope}` reverted · recoverable with /undo"
                                            ),
                                        );
                                    }
                                    Err(error) => {
                                        host.changes().record_revert_event(
                                            &scope,
                                            &fingerprint,
                                            "failed",
                                        );
                                        app.transcript.push_error(error.message);
                                    }
                                }
                            }
                            Some(Action::CancelRevert { scope, fingerprint }) => {
                                host.changes().record_revert_event(
                                    &scope,
                                    &fingerprint,
                                    "cancelled",
                                );
                                app.transcript.push_notice("revert", "cancelled");
                            }
                            Some(Action::StartReview { scope }) => {
                                start_review(host, project, scope, local_tx.clone());
                            }
                            Some(Action::StartAgent { preset, task }) => {
                                start_agent(host, agents, preset, task, local_tx.clone());
                            }
                            Some(Action::FollowUpAgent { child_id, task }) => {
                                follow_up_agent(host, child_id, task, local_tx.clone());
                            }
                            Some(Action::ResumeAgent { child_id }) => {
                                resume_agent(host, child_id, local_tx.clone());
                            }
                            None => {}
                        }
                        dirty = true;
                    }
                    TermEvent::Paste(text) => {
                        app.on_paste(&text);
                        dirty = true;
                    }
                    TermEvent::Mouse(mouse) => {
                        if app.on_mouse(mouse) {
                            dirty = true;
                        }
                    }
                    TermEvent::Resize(_, _) => dirty = true,
                    _ => {}
                }
            }

            prompt = next_approval(&mut approvals) => {
                match prompt {
                    Some(prompt) => {
                        app.present_approval(prompt);
                        dirty = true;
                    }
                    None => approvals = None,
                }
            }

            notice = interactions.next_notice() => {
                match notice {
                    Some(notice) => {
                        interactions.apply_notice(&mut app, notice);
                        dirty = true;
                    }
                    None => interactions.close_receiver(),
                }
            }

            envelope = events.next() => {
                match envelope {
                    Some(envelope) => {
                        let tool_call = tool_call_for_display(&envelope.payload);
                        if matches!(envelope.payload, RuntimeEvent::TurnCompleted { .. })
                            && let Some(set) = host.changes().latest()
                            && last_change_turn != Some(set.turn)
                            && !set.undone
                        {
                                    last_change_turn = Some(set.turn);
                                    let attribution = if set.is_fully_attributable() {
                                        "undo available"
                                    } else {
                                        "contains ambiguous changes; use /diff"
                                    };
                                    app.transcript.push_notice(
                                        "changes",
                                        format!("Smith turn {} · {attribution}", set.turn),
                                    );
                        }
                        let completed_tool =
                            matches!(envelope.payload, RuntimeEvent::ToolCallCompleted { .. });
                        app.apply(&envelope);
                        if let Some(call) = tool_call {
                            if let Some(display) = host.tool_call_display(&call) {
                                app.set_tool_display(call.as_str(), display);
                            }
                            if completed_tool
                                && let Some(text) = host.tool_result_text(&call)
                            {
                                app.set_tool_result_preview(call.as_str(), text);
                            }
                        }
                        dirty = true;
                    }
                    None => break InteractiveExit::Quit,
                }
            }

            outcome = local_rx.recv() => {
                if let Some(outcome) = outcome {
                    match outcome {
                        LocalOutcome::Notice { source, text } => {
                            app.transcript.push_notice(source, text);
                        }
                        LocalOutcome::Error(text) => app.transcript.push_error(text),
                        LocalOutcome::Shell { content, is_error } => {
                            if is_error {
                                app.show_local_error("shell", content);
                            } else {
                                app.show_local_result("shell", content);
                            }
                        }
                        LocalOutcome::PreparedSendFailed { text, error } => {
                            app.composer.replace(text);
                            app.transcript.push_error(error);
                        }
                    }
                    dirty = true;
                }
            }

            _ = spinner.tick() => {
                let exit_hint_expired = app.expire_ctrl_c_exit_hint();
                let busy = app.is_busy();
                if busy {
                    app.tick();
                }
                if exit_hint_expired
                    || (busy && (theme.uses_motion() || app.tick.is_multiple_of(10)))
                {
                    dirty = true;
                }
            }

            _ = frame.tick(), if dirty => {
                terminal.draw(|frame| smith_tui::draw_synced(frame, &mut app, theme))?;
                dirty = false;
            }
        }

        interactions.drain_answers(&mut app);
        if app.should_quit {
            break InteractiveExit::Quit;
        }
    };
    Ok(exit)
}

fn tool_call_for_display(event: &RuntimeEvent) -> Option<agent_runtime_core::ids::ToolCallId> {
    match event {
        RuntimeEvent::ToolCallRequested { call, .. }
        | RuntimeEvent::ToolCallCompleted { call, .. } => Some(call.clone()),
        _ => None,
    }
}

async fn handle_local_command(
    app: &mut App,
    host: &HostSession,
    project: &std::path::Path,
    command: CommandAction,
) {
    match command {
        CommandAction::Context => {
            app.show_local_result(
                "context",
                render_context_view(&app.status, host.runtime().policy()),
            );
        }
        CommandAction::Timeline => {
            let events = match host.timeline_events().await {
                Ok(events) => events,
                Err(error) => {
                    app.show_local_error("timeline", format!("timeline unavailable: {error}"));
                    return;
                }
            };
            let timeline = render_runtime_timeline(&events);
            let mut lines = timeline.lines;
            if lines.is_empty() {
                lines.extend(host.session().snapshot().manifests.iter().map(|manifest| {
                    format!(
                        "root {} · committed · {}/{} · {} activated capability/capabilities",
                        manifest.turn,
                        manifest.manifest.model.provider,
                        manifest.manifest.model.model,
                        manifest.manifest.activation.len(),
                    )
                }));
            }
            if let Some(coordinator) = host
                .runtime()
                .delegation()
                .and_then(|delegation| delegation.coordinator())
            {
                lines.extend(
                    coordinator
                        .list()
                        .into_iter()
                        .filter(|child| !timeline.children.contains(&child.child))
                        .map(|child| {
                            format!(
                                "child {} · session {} · {:?} · {:?} · resumable {} · {}/{} turns",
                                child.child,
                                child.session,
                                child.durability,
                                child.state,
                                child.resumable(),
                                child.turns_used,
                                child.max_turns,
                            )
                        }),
                );
            }
            lines.extend(
                host.changes()
                    .timeline()
                    .into_iter()
                    .enumerate()
                    .map(|(index, entry)| format!("recovery recovery-{} · {entry}", index + 1)),
            );
            if lines.len() > 100 {
                lines.drain(..lines.len().saturating_sub(100));
            }
            if lines.is_empty() {
                app.show_local_empty("timeline", "No turns, children, or recovery actions yet.");
            } else {
                app.show_local_result("timeline", lines.join("\n"));
            }
        }
        CommandAction::Status => {
            let policy = host.runtime().policy();
            let git = GitChanges::discover(project)
                .and_then(|git| git.status_summary())
                .unwrap_or_else(|_| "unavailable (not a Git worktree)".to_owned());
            let child_count = host
                .runtime()
                .delegation()
                .and_then(|delegation| delegation.coordinator())
                .map_or(0, |coordinator| coordinator.list().len());
            let attribution = host.changes().latest().map_or_else(
                || {
                    if host.changes().has_historical_records() {
                        "historical metadata only; not automatically undoable".to_owned()
                    } else {
                        "no attributable turn recorded".to_owned()
                    }
                },
                |set| {
                    if set.is_fully_attributable() && !set.undone {
                        format!("Smith turn {} · undo available", set.turn)
                    } else {
                        format!("Smith turn {} · automatic undo unavailable", set.turn)
                    }
                },
            );
            let context = render_context_status(&app.status, policy);
            let harness = render_harness_status(&app.status);
            let reasoning = render_reasoning_status(policy);
            let goal = host.goal().map_or_else(
                |error| format!("unavailable ({error})"),
                |goal| goal.as_ref().map_or_else(|| "none".to_owned(), render_goal),
            );
            app.show_local_result(
                "status",
                format!(
                    "session: {}\nprofile: {} · posture {} · use {} · rev {} · source {}{}\n\
                     provider: {}\nmodel: {}\npermission: {:?}\n\
                     {reasoning}\n\
                     protected mid-turn recovery: {}\n\
                     {harness}\n{context}\nproject: {}\nGit: {}\n\
                     goal: {goal}\nchildren: {}\nchange attribution: {}",
                    host.session().id(),
                    policy.agent_profile,
                    policy.agent_posture.as_str(),
                    policy
                        .agent_profile_uses
                        .iter()
                        .map(|placement| placement.as_str())
                        .collect::<Vec<_>>()
                        .join("+"),
                    bounded_text(&policy.agent_profile_revision, 12),
                    bounded_text(&policy.agent_profile_source, 80),
                    if policy.agent_profile_legacy {
                        " · legacy adapter; migrate to [profiles]"
                    } else {
                        ""
                    },
                    policy.provider_name,
                    policy.model,
                    policy.approval_mode,
                    policy.mid_turn_durability.as_str(),
                    project.display(),
                    git,
                    child_count,
                    attribution,
                ),
            );
        }
        CommandAction::Goal(action) => {
            let result = match action {
                GoalAction::Show => match host.goal() {
                    Ok(Some(goal)) => {
                        app.show_local_result("goal", render_goal(&goal));
                        return;
                    }
                    Ok(None) if host.runtime().goal_component().is_some() => {
                        app.show_local_empty(
                            "goal",
                            "No persistent goal. Create one with `/goal <objective>`.",
                        );
                        return;
                    }
                    Ok(None) => {
                        app.show_local_error(
                            "goal",
                            "Persistent goals require a persisted root session; they are unavailable in ephemeral and child sessions.",
                        );
                        return;
                    }
                    Err(error) => Err(error),
                },
                GoalAction::Create(objective) => {
                    host.control_goal(GoalCommand::Create {
                        objective,
                        token_budget: None,
                    })
                    .await
                }
                GoalAction::Edit(objective) => match host.goal() {
                    Ok(Some(goal)) => {
                        host.control_goal(GoalCommand::Edit {
                            id: goal.id,
                            generation: goal.generation,
                            objective,
                        })
                        .await
                    }
                    Ok(None) => {
                        app.show_local_error("goal", "No goal to edit; use `/goal <objective>`.");
                        return;
                    }
                    Err(error) => Err(error),
                },
                GoalAction::Budget(token_budget) => match host.goal() {
                    Ok(Some(goal)) => {
                        host.control_goal(GoalCommand::SetBudget {
                            id: goal.id,
                            generation: goal.generation,
                            token_budget,
                        })
                        .await
                    }
                    Ok(None) => {
                        app.show_local_error(
                            "goal",
                            "No goal budget to change; use `/goal <objective>` first.",
                        );
                        return;
                    }
                    Err(error) => Err(error),
                },
                GoalAction::Pause => match host.goal() {
                    Ok(Some(goal)) => {
                        host.control_goal(GoalCommand::Pause {
                            id: goal.id,
                            generation: goal.generation,
                        })
                        .await
                    }
                    Ok(None) => {
                        app.show_local_error("goal", "No active goal to pause.");
                        return;
                    }
                    Err(error) => Err(error),
                },
                GoalAction::Resume => match host.goal() {
                    Ok(Some(goal)) => {
                        host.control_goal(GoalCommand::Resume {
                            id: goal.id,
                            generation: goal.generation,
                        })
                        .await
                    }
                    Ok(None) => {
                        app.show_local_error("goal", "No stopped goal to resume.");
                        return;
                    }
                    Err(error) => Err(error),
                },
                GoalAction::Clear => match host.goal() {
                    Ok(Some(goal)) => {
                        host.control_goal(GoalCommand::Clear {
                            id: goal.id,
                            generation: goal.generation,
                        })
                        .await
                    }
                    Ok(None) => {
                        app.show_local_error("goal", "No goal to clear.");
                        return;
                    }
                    Err(error) => Err(error),
                },
            };
            match result {
                Ok(result) => match result.goal {
                    Some(goal) => app.show_local_result("goal", render_goal(&goal)),
                    None => app.show_local_result("goal", "Goal cleared."),
                },
                Err(error) => app.show_local_error("goal", error.to_string()),
            }
        }
        CommandAction::Agent(selected) => {
            let Some(coordinator) = host
                .runtime()
                .delegation()
                .and_then(|delegation| delegation.coordinator())
            else {
                app.show_local_error(
                    "agents",
                    "Child delegation is unavailable for this session.",
                );
                return;
            };
            let children = coordinator.list();
            let selected = match selected.as_deref() {
                Some("parent") => {
                    app.inspected_child = None;
                    app.show_local_result(
                        "agent",
                        "Returned to the root timeline; the root composer remained focused.",
                    );
                    return;
                }
                Some("next" | "previous") if children.is_empty() => None,
                Some(direction @ ("next" | "previous")) => {
                    let current = app
                        .inspected_child
                        .as_deref()
                        .and_then(|current| {
                            children
                                .iter()
                                .position(|status| status.child.as_str() == current)
                        })
                        .unwrap_or(0);
                    let index = if direction == "next" {
                        (current + 1) % children.len()
                    } else {
                        current.checked_sub(1).unwrap_or(children.len() - 1)
                    };
                    Some(children[index].child.as_str().to_owned())
                }
                Some(selected) => Some(selected.to_owned()),
                None => None,
            };
            if let Some(selected) = selected {
                let Some(status) = children
                    .iter()
                    .find(|status| status.child.as_str() == selected)
                else {
                    app.show_local_error("agents", format!("No child named `{selected}`."));
                    return;
                };
                app.inspected_child = Some(selected);
                app.show_local_result(
                    "agent",
                    format!(
                        "child: {}\nchild session: {}\ndurability: {:?}\nstate: {:?}\nresumable: {}\nturns: {}/{}\ntokens: {}\nworkspace: {:?}\nincompatibility: {}\nresult: {}\n\ncontinue: @{} <new follow-up task>\nexact recovery: /agent resume {}\nnavigation: /agent previous · /agent next · /agent parent",
                        status.child,
                        status.session,
                        status.durability,
                        status.state,
                        status.resumable(),
                        status.turns_used,
                        status.max_turns,
                        status.tokens_used,
                        status.workspace,
                        status.incompatibility.as_deref().unwrap_or("none"),
                        status.last_result.as_deref().unwrap_or("not available"),
                        status.child,
                        status.child,
                    ),
                );
            } else if children.is_empty() {
                app.show_local_empty("agents", "No child agents in this session.");
            } else {
                app.show_local_result(
                    "agents",
                    children
                        .iter()
                        .map(|status| {
                            format!(
                                "{} · {:?} · {:?} · resumable {} · {}/{} turns · {} tokens",
                                status.child,
                                status.durability,
                                status.state,
                                status.resumable(),
                                status.turns_used,
                                status.max_turns,
                                status.tokens_used,
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
        }
        CommandAction::Diff(scope) => {
            if scope.as_deref() == Some("last-turn") {
                match host.changes().undo_preview() {
                    Ok(preview) => app.show_local_result("diff · last Smith turn", preview),
                    Err(error) => app.show_local_error("diff · last Smith turn", error.message),
                }
            } else {
                match GitChanges::discover(project).and_then(|git| git.inspect(scope.as_deref())) {
                    Ok(view) if view.content == "No changes in this scope." => {
                        app.show_local_empty(view.title, view.content);
                    }
                    Ok(view) => app.show_local_result(view.title, view.content),
                    Err(error) => app.show_local_error("diff", error.message),
                }
            }
        }
        CommandAction::Review(scope) => {
            let scope = scope.unwrap_or_else(|| "all".to_owned());
            match GitChanges::discover(project)
                .and_then(|git| git.inspect(Some(scope.as_str())))
            {
                Ok(view) if view.content == "No changes in this scope." => {
                    app.transcript.push_notice("review", view.content);
                }
                Ok(view) => app.confirm_review(
                    scope,
                    format!(
                        "scope: {}\nprovider-backed: yes\nworkspace authority: read-only\n\
                         The reviewer can read, list, and search but cannot edit or run shell commands.\n\n{}",
                        view.title, view.content
                    ),
                ),
                Err(error) => app.transcript.push_error(error.message),
            }
        }
        CommandAction::Undo => match host.changes().undo_preview() {
            Ok(preview) => app.confirm_undo(preview),
            Err(error) => app.transcript.push_error(error.message),
        },
        CommandAction::Redo => match host.changes().redo_preview() {
            Ok(preview) => app.confirm_redo(preview),
            Err(error) => app.show_local_error("redo", error.message),
        },
        CommandAction::Revert(Some(scope)) => {
            match GitChanges::discover(project).and_then(|git| git.preview_revert(&scope)) {
                Ok(mut preview) => {
                    let path = scope.split('#').next().unwrap_or(scope.as_str());
                    if let Ok(canonical) = project.join(path).canonicalize()
                        && host.changes().latest_owns_path(&canonical)
                    {
                        preview.content =
                            preview
                                .content
                                .replacen("origin: unknown", "origin: Smith", 1);
                    }
                    host.changes().record_revert_event(
                        &preview.scope,
                        &preview.fingerprint,
                        "previewed",
                    );
                    app.confirm_revert(preview.scope, preview.fingerprint, preview.content);
                }
                Err(error) => app.transcript.push_error(error.message),
            }
        }
        CommandAction::Revert(None) => app
            .transcript
            .push_error("usage: /revert FILE or /revert FILE#HUNK; use /diff to choose a scope"),
        CommandAction::Help
        | CommandAction::Details
        | CommandAction::NewSession
        | CommandAction::Resume(_)
        | CommandAction::Profile(_)
        | CommandAction::Provider(_)
        | CommandAction::Model(_)
        | CommandAction::Think(_)
        | CommandAction::Effort(_)
        | CommandAction::AgentResume(_)
        | CommandAction::Quit => {
            unreachable!("the reducer handles this command before host dispatch")
        }
    }
}

fn render_goal(goal: &GoalProjection) -> String {
    let status = goal.status.as_str();
    let usage = goal
        .usage
        .charged_tokens
        .map_or_else(|| "unknown".to_owned(), |tokens| tokens.to_string());
    let budget = goal
        .token_budget
        .map_or_else(|| "none".to_owned(), |tokens| tokens.to_string());
    let provenance = goal.usage.provenance.as_str();
    let reason = goal.stopped_reason.as_ref().map_or_else(
        || "none".to_owned(),
        |reason| {
            reason.detail.as_ref().map_or_else(
                || reason.code.clone(),
                |detail| format!("{} · {detail}", reason.code),
            )
        },
    );
    format!(
        "{}\nstatus: {status}\ntokens: {usage} · {provenance}\nbudget: {budget}\nactive elapsed: {}\nreason: {reason}\nid: {} · generation {}",
        goal.objective,
        render_elapsed(Duration::from_millis(goal.usage.active_elapsed_ms)),
        goal.id,
        goal.generation,
    )
}

#[derive(Debug, Default)]
struct RuntimeTimeline {
    lines: Vec<String>,
    children: BTreeSet<ChildId>,
}

#[derive(Debug, Default)]
struct TurnTimelineState {
    plan: Option<BTreeMap<String, u32>>,
    passed_gates: u32,
    failed_gates: u32,
}

fn render_runtime_timeline(events: &[EventEnvelope]) -> RuntimeTimeline {
    let mut timeline = RuntimeTimeline::default();
    let mut turns = BTreeMap::<String, TurnTimelineState>::new();
    let mut tools = BTreeMap::<String, String>::new();

    for event in events {
        match &event.payload {
            RuntimeEvent::PlanUpdated { counts, .. } => {
                if let Some(turn) = &event.turn {
                    turns.entry(turn.as_str().to_owned()).or_default().plan = Some(counts.clone());
                }
            }
            RuntimeEvent::ToolCallRequested { call, name, .. } => {
                tools.insert(call.as_str().to_owned(), name.clone());
            }
            RuntimeEvent::ToolCallCompleted { call, is_error, .. }
                if tools.get(call.as_str()).is_some_and(|name| name == "shell") =>
            {
                if let Some(turn) = &event.turn {
                    let state = turns.entry(turn.as_str().to_owned()).or_default();
                    if *is_error {
                        state.failed_gates = state.failed_gates.saturating_add(1);
                    } else {
                        state.passed_gates = state.passed_gates.saturating_add(1);
                    }
                }
            }
            RuntimeEvent::TurnCompleted { finish, .. } => {
                if let Some(turn) = &event.turn {
                    let state = turns.remove(turn.as_str()).unwrap_or_default();
                    timeline.lines.push(format!(
                        "root {} · {} · {} · gates {} passed/{} failed",
                        turn,
                        turn_finish_label(finish),
                        render_terminal_plan(state.plan.as_ref()),
                        state.passed_gates,
                        state.failed_gates,
                    ));
                }
            }
            RuntimeEvent::ChildSpawned {
                child,
                workspace,
                max_turns,
                ..
            } => {
                timeline.children.insert(child.clone());
                timeline.lines.push(format!(
                    "child {child} · started · {workspace:?} · {max_turns} turn limit"
                ));
            }
            RuntimeEvent::ChildNeedsInput { child, .. } => {
                timeline.children.insert(child.clone());
                timeline.lines.push(format!("child {child} · needs input"));
            }
            RuntimeEvent::ChildCompleted { child, .. } => {
                timeline.children.insert(child.clone());
                timeline
                    .lines
                    .push(format!("child {child} · task completed"));
            }
            RuntimeEvent::ChildStopped { child, reason } => {
                timeline.children.insert(child.clone());
                timeline
                    .lines
                    .push(format!("child {child} · stopped ({reason:?})"));
            }
            RuntimeEvent::ChildFailed { child, .. } => {
                timeline.children.insert(child.clone());
                timeline.lines.push(format!("child {child} · failed"));
            }
            _ => {}
        }
    }

    timeline
}

fn render_terminal_plan(counts: Option<&BTreeMap<String, u32>>) -> String {
    let Some(counts) = counts else {
        return "plan none".to_owned();
    };
    let count = |status: &str| counts.get(status).copied().unwrap_or_default();
    format!(
        "plan {} active/{} pending/{} done/{} cancelled",
        count("in_progress"),
        count("pending"),
        count("completed"),
        count("cancelled")
    )
}

fn turn_finish_label(finish: &TurnFinish) -> String {
    match finish {
        TurnFinish::Completed => "completed".to_owned(),
        TurnFinish::Cancelled { reason } => format!("cancelled ({reason:?})"),
        TurnFinish::LimitReached { limit } => format!("limit reached ({limit:?})"),
        TurnFinish::NeedsInput { request } => format!("needs input ({request})"),
        TurnFinish::Failed => "failed".to_owned(),
    }
}

fn render_harness_status(status: &Status) -> String {
    let capabilities = &status.capabilities;
    let mut lines = Vec::new();
    match &capabilities.registry {
        Some((fingerprint, entries)) => {
            lines.push(format!(
                "registry snapshot: {fingerprint} · {entries} entries"
            ));
        }
        None => lines.push("registry snapshot: waiting for live lifecycle".to_owned()),
    }
    if let Some((fingerprint, visible)) = &capabilities.view {
        lines.push(format!(
            "scoped capability view: {fingerprint} · {visible} visible"
        ));
    }
    if let Some((revision, candidates)) = &capabilities.retrieval {
        let candidates = if candidates.is_empty() {
            "(none)".to_owned()
        } else {
            candidates.join(", ")
        };
        lines.push(format!(
            "latest capability retrieval: {revision} · {candidates}"
        ));
    }
    if let Some((epoch, active)) = &capabilities.activation {
        let active = if active.is_empty() {
            "(none)".to_owned()
        } else {
            active.join(", ")
        };
        lines.push(format!("activation epoch: {epoch} · {active}"));
    }
    if let Some(plan) = &status.context_plan {
        lines.push(format!(
            "context provenance: {} · cache {}",
            plan.fingerprint, plan.cache_fingerprint
        ));
    }
    if capabilities.compactions > 0 {
        lines.push(format!(
            "context compaction: {} run(s) · {} tokens reclaimed",
            capabilities.compactions, capabilities.reclaimed_tokens
        ));
    }
    lines.join("\n")
}

fn render_context_status(status: &Status, policy: &RuntimePolicy) -> String {
    let limits = policy.model_profile.limits;
    let declared_reserve = policy
        .context_policy
        .output_reserve
        .saturating_add(policy.context_policy.reasoning_reserve);
    let input_budget = limits.input_budget(declared_reserve);
    let exact = |tokens: u32| TokenCount::reported(u64::from(tokens)).render();
    let with_confidence = |tokens: u32, confidence: EstimationConfidence| match confidence {
        EstimationConfidence::Exact => TokenCount::reported(u64::from(tokens)).render(),
        EstimationConfidence::Estimated => TokenCount::estimated(u64::from(tokens)).render(),
    };

    let mut lines = Vec::new();
    if let Some(plan) = &status.context_plan {
        let percent_prefix = if plan.confidence == EstimationConfidence::Estimated {
            "~"
        } else {
            ""
        };
        lines.push(format!(
            "context window: {percent_prefix}{}% input left ({} used / {} budget)",
            plan.percent_left(),
            plan.render_input(),
            exact(plan.input_budget_tokens),
        ));
        lines.push(format!(
            "model window: {} total · {} reserved",
            exact(limits.context_tokens),
            exact(plan.reserved_tokens),
        ));
        lines.push(format!(
            "context plan: {} · {} segments",
            plan.confidence_label(),
            plan.segment_count,
        ));
        for (kind, tokens) in &plan.totals {
            lines.push(format!(
                "  {}: {}",
                kind.replace('_', " "),
                with_confidence(*tokens, plan.confidence),
            ));
        }
        let compaction_target = exact(policy.compaction_policy.low_watermark);
        if let Some(summary_tokens) = plan.totals.get("summary").filter(|tokens| **tokens > 0) {
            lines.push(format!(
                "compaction: applied · {} summary · {} recovery target",
                with_confidence(*summary_tokens, plan.confidence),
                compaction_target,
            ));
        } else {
            lines.push(format!(
                "compaction: enabled on overflow · {compaction_target} recovery target"
            ));
        }
    } else {
        lines.push(format!(
            "context window: not planned yet (? used / {} input budget)",
            exact(input_budget),
        ));
        lines.push(format!(
            "model window: {} total · {} reserved",
            exact(limits.context_tokens),
            exact(declared_reserve),
        ));
        lines.push("context plan: waiting for first turn".to_owned());
        lines.push(format!(
            "compaction: enabled on overflow · {} recovery target",
            exact(policy.compaction_policy.low_watermark),
        ));
    }
    lines.push(format!(
        "provider input (session): {}",
        status.context.render()
    ));
    lines.push(format!("cache read (session): {}", status.render_cache()));
    lines.push(render_reasoning_status(policy));
    lines.join("\n")
}

fn render_reasoning_status(policy: &RuntimePolicy) -> String {
    let support = match policy.reasoning.support {
        ReasoningSupport::Unsupported => "unsupported",
        ReasoningSupport::Fixed => "fixed",
        ReasoningSupport::Controllable => "controllable",
    };
    let efforts = if policy.reasoning.efforts.is_empty() {
        "none".to_owned()
    } else {
        policy.reasoning.efforts.join(", ")
    };
    format!(
        "reasoning: {} · effort {} · {}\nreasoning controls: {support} · switch {} · efforts {efforts} · {}",
        policy.reasoning.effective_state(),
        policy.reasoning.effective_effort(),
        policy.reasoning.selection_source,
        policy.reasoning.switch.as_str(),
        policy.reasoning.capability_source,
    )
}

#[derive(Debug, Clone)]
struct ContextDisplayCategory {
    label: String,
    glyph: &'static str,
    tokens: u32,
    rank: u8,
}

fn render_context_view(status: &Status, policy: &RuntimePolicy) -> String {
    let limits = policy.model_profile.limits;
    let declared_reserve = policy
        .context_policy
        .output_reserve
        .saturating_add(policy.context_policy.reasoning_reserve);
    let input_budget = limits.input_budget(declared_reserve);
    let exact = |tokens: u32| TokenCount::reported(u64::from(tokens)).render();
    let with_confidence = |tokens: u32, confidence: EstimationConfidence| match confidence {
        EstimationConfidence::Exact => TokenCount::reported(u64::from(tokens)).render(),
        EstimationConfidence::Estimated => TokenCount::estimated(u64::from(tokens)).render(),
    };

    let mut lines = vec!["Context usage".to_owned()];
    if let Some(plan) = &status.context_plan {
        let percent_prefix = if plan.confidence == EstimationConfidence::Estimated {
            "~"
        } else {
            ""
        };
        lines.push(format!(
            "{} · {} / {} input tokens · {percent_prefix}{}% left",
            policy.model,
            plan.render_input(),
            exact(plan.input_budget_tokens),
            plan.percent_left(),
        ));
        lines.push(String::new());

        let mut categories = context_display_categories(&plan.totals);
        let categorized = categories.iter().fold(0u32, |total, category| {
            total.saturating_add(category.tokens)
        });
        if plan.input_tokens > categorized {
            categories.push(ContextDisplayCategory {
                label: "other context".to_owned(),
                glyph: glyph::CONTEXT_OTHER,
                tokens: plan.input_tokens - categorized,
                rank: u8::MAX,
            });
        }
        let free_tokens = plan.remaining_tokens();
        let mut grid = categories
            .iter()
            .map(|category| (category.glyph, category.tokens))
            .collect::<Vec<_>>();
        grid.push((glyph::CONTEXT_FREE, free_tokens));
        grid.push((glyph::CONTEXT_RESERVE, plan.reserved_tokens));
        lines.extend(render_context_grid(&grid));
        lines.push(String::new());
        lines.push(match plan.confidence {
            EstimationConfidence::Exact => "Exact usage by category".to_owned(),
            EstimationConfidence::Estimated => "Estimated usage by category".to_owned(),
        });
        for category in &categories {
            lines.push(format!(
                "{} {}: {} ({})",
                category.glyph,
                category.label,
                with_confidence(category.tokens, plan.confidence),
                render_percent(category.tokens, plan.input_budget_tokens),
            ));
        }
        lines.push(format!(
            "{} free input: {} ({})",
            glyph::CONTEXT_FREE,
            with_confidence(free_tokens, plan.confidence),
            render_percent(free_tokens, plan.input_budget_tokens),
        ));
        lines.push(format!(
            "{} output/reasoning reserve: {} ({})",
            glyph::CONTEXT_RESERVE,
            exact(plan.reserved_tokens),
            render_percent(
                plan.reserved_tokens,
                plan.input_budget_tokens
                    .saturating_add(plan.reserved_tokens),
            ),
        ));
        lines.push(format!(
            "model window: {} total · {} input budget",
            exact(limits.context_tokens),
            exact(plan.input_budget_tokens),
        ));
        lines.push(format!(
            "counting: {} · {} segments",
            plan.confidence_label(),
            plan.segment_count,
        ));
        let compaction_target = exact(policy.compaction_policy.low_watermark);
        if let Some(summary_tokens) = plan.totals.get("summary").filter(|tokens| **tokens > 0) {
            lines.push(format!(
                "compaction: applied · {} summary · {} recovery target",
                with_confidence(*summary_tokens, plan.confidence),
                compaction_target,
            ));
        } else {
            lines.push(format!(
                "compaction: enabled on overflow · {compaction_target} recovery target"
            ));
        }
    } else {
        lines.push(format!(
            "{} · usage unavailable until the first turn",
            policy.model
        ));
        lines.push(String::new());
        lines.extend(render_context_grid(&[
            (glyph::CONTEXT_FREE, input_budget),
            (glyph::CONTEXT_RESERVE, declared_reserve),
        ]));
        lines.push(String::new());
        lines.push("Available capacity".to_owned());
        lines.push(format!(
            "{} system instructions: ? (not counted yet)",
            glyph::CONTEXT_SYSTEM,
        ));
        lines.push(format!(
            "{} tool schemas: ? (not counted yet)",
            glyph::CONTEXT_TOOL,
        ));
        lines.push(format!(
            "{} free input: {}",
            glyph::CONTEXT_FREE,
            exact(input_budget),
        ));
        lines.push(format!(
            "{} output/reasoning reserve: {}",
            glyph::CONTEXT_RESERVE,
            exact(declared_reserve),
        ));
        lines.push(format!(
            "model window: {} total · {} input budget",
            exact(limits.context_tokens),
            exact(input_budget),
        ));
        lines.push("counting: waiting for first context plan".to_owned());
        lines.push(format!(
            "compaction: enabled on overflow · {} recovery target",
            exact(policy.compaction_policy.low_watermark),
        ));
    }
    lines.push(format!(
        "provider input (session): {}",
        status.context.render()
    ));
    lines.push(format!("cache read (session): {}", status.render_cache()));
    lines.push(render_reasoning_status(policy));
    lines.join("\n")
}

fn context_display_categories(
    totals: &std::collections::BTreeMap<String, u32>,
) -> Vec<ContextDisplayCategory> {
    const INSTRUCTION_KINDS: [&str; 3] = [
        "system_instruction",
        "developer_instruction",
        "ability_instruction",
    ];

    let instruction_tokens = INSTRUCTION_KINDS.iter().fold(0u32, |sum, kind| {
        sum.saturating_add(totals.get(*kind).copied().unwrap_or_default())
    });
    let mut categories = vec![
        ContextDisplayCategory {
            label: "system instructions".to_owned(),
            glyph: glyph::CONTEXT_SYSTEM,
            tokens: instruction_tokens,
            rank: 0,
        },
        ContextDisplayCategory {
            label: "tool schemas".to_owned(),
            glyph: glyph::CONTEXT_TOOL,
            tokens: totals.get("tool_schema").copied().unwrap_or_default(),
            rank: 1,
        },
    ];
    categories.extend(
        totals
            .iter()
            .filter(|(kind, tokens)| {
                **tokens > 0
                    && !INSTRUCTION_KINDS.contains(&kind.as_str())
                    && kind.as_str() != "tool_schema"
            })
            .map(|(kind, tokens)| context_display_category(kind, *tokens)),
    );
    categories.sort_by(|left, right| {
        left.rank
            .cmp(&right.rank)
            .then_with(|| left.label.cmp(&right.label))
    });
    categories
}

fn context_display_category(kind: &str, tokens: u32) -> ContextDisplayCategory {
    let (label, glyph, rank) = match kind {
        "system_instruction" => ("system instructions".to_owned(), glyph::CONTEXT_SYSTEM, 0),
        "developer_instruction" => (
            "developer instructions".to_owned(),
            glyph::CONTEXT_SYSTEM,
            1,
        ),
        "ability_instruction" => ("ability instructions".to_owned(), glyph::CONTEXT_SYSTEM, 2),
        "tool_schema" => ("tool schemas".to_owned(), glyph::CONTEXT_TOOL, 3),
        "memory" => ("memory".to_owned(), glyph::CONTEXT_HISTORY, 4),
        "history" => ("history".to_owned(), glyph::CONTEXT_HISTORY, 5),
        "tool_result" => ("tool results".to_owned(), glyph::CONTEXT_TOOL, 6),
        "retrieval" => ("retrieved context".to_owned(), glyph::CONTEXT_HISTORY, 7),
        "continuation" => ("continuation".to_owned(), glyph::CONTEXT_OTHER, 8),
        "summary" => ("summary".to_owned(), glyph::CONTEXT_SUMMARY, 9),
        "user_input" => ("user input".to_owned(), glyph::CONTEXT_INPUT, 10),
        other => (other.replace('_', " "), glyph::CONTEXT_OTHER, u8::MAX - 1),
    };
    ContextDisplayCategory {
        label,
        glyph,
        tokens,
        rank,
    }
}

fn render_percent(tokens: u32, total: u32) -> String {
    if total == 0 {
        return "0.0%".to_owned();
    }
    let tenths = u64::from(tokens)
        .saturating_mul(1_000)
        .checked_div(u64::from(total))
        .unwrap_or(0);
    format!("{}.{:01}%", tenths / 10, tenths % 10)
}

fn render_context_grid(entries: &[(&'static str, u32)]) -> Vec<String> {
    const CELLS: usize = 50;
    const COLUMNS: usize = 10;

    let entries = entries
        .iter()
        .copied()
        .filter(|(_, tokens)| *tokens > 0)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return vec![format!("{} ", glyph::CONTEXT_FREE).repeat(COLUMNS); CELLS / COLUMNS];
    }

    let weight = entries.iter().fold(0u64, |total, (_, tokens)| {
        total.saturating_add(u64::from(*tokens))
    });
    let remaining = CELLS.saturating_sub(entries.len());
    let mut allocations = vec![1usize; entries.len()];
    let mut remainders = Vec::with_capacity(entries.len());
    let mut distributed = 0usize;
    for (index, (_, tokens)) in entries.iter().enumerate() {
        let numerator = (remaining as u64).saturating_mul(u64::from(*tokens));
        let share = numerator.checked_div(weight).unwrap_or(0) as usize;
        allocations[index] = allocations[index].saturating_add(share);
        distributed = distributed.saturating_add(share);
        remainders.push((index, numerator.checked_rem(weight).unwrap_or(0)));
    }
    remainders.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    for (index, _) in remainders
        .into_iter()
        .take(remaining.saturating_sub(distributed))
    {
        allocations[index] = allocations[index].saturating_add(1);
    }

    let cells = entries
        .iter()
        .zip(allocations)
        .flat_map(|((glyph, _), count)| std::iter::repeat_n(*glyph, count))
        .take(CELLS)
        .collect::<Vec<_>>();
    cells.chunks(COLUMNS).map(|row| row.join(" ")).collect()
}

enum LocalOutcome {
    Notice {
        /// The transcript block label — "agents" for child lifecycle,
        /// "review" for reviewer starts.
        source: &'static str,
        text: String,
    },
    Error(String),
    Shell {
        content: String,
        is_error: bool,
    },
    PreparedSendFailed {
        text: String,
        error: String,
    },
}

fn start_local_shell(
    session: smith_runtime::SessionHandle,
    command: String,
    timeout_ms: u64,
    outcomes: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
) {
    tokio::spawn(async move {
        let outcome = session
            .run_local_tool(
                "shell",
                serde_json::json!({
                    "command": command,
                    "cwd": ".",
                    "timeout_ms": timeout_ms,
                }),
                timeout_ms,
            )
            .await;
        let result = match outcome {
            Ok(block) => LocalOutcome::Shell {
                content: tool_result_text(&block),
                is_error: block.is_error,
            },
            Err(error) => LocalOutcome::Error(format!("shell action failed: {error}")),
        };
        let _ = outcomes.send(result);
    });
}

/// Largest PNG accepted from the clipboard, after encoding.
const MAX_CLIPBOARD_IMAGE_BYTES: usize = 5 * 1024 * 1024;

enum ClipboardContent {
    Image {
        data_uri: String,
        width: u32,
        height: u32,
    },
    Text(String),
    Empty,
}

/// Reads the platform clipboard once and attaches whatever it holds.
///
/// An image becomes a composer attachment; text falls back to the ordinary
/// paste path (covering terminals whose `Ctrl+V` never reaches bracketed
/// paste); an unreadable clipboard reports instead of failing silently.
fn attach_from_clipboard(app: &mut App) {
    match read_clipboard() {
        Ok(ClipboardContent::Image {
            data_uri,
            width,
            height,
        }) => {
            if app.can_attach_image() {
                app.attach_image(data_uri, width, height);
            }
        }
        Ok(ClipboardContent::Text(text)) => app.on_paste(&text),
        Ok(ClipboardContent::Empty) => {
            app.transcript.push_notice("clipboard", "nothing to attach");
        }
        Err(error) => app.transcript.push_error(error),
    }
}

fn read_clipboard() -> Result<ClipboardContent, String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| format!("clipboard unavailable: {error}"))?;
    if let Ok(image) = clipboard.get_image() {
        let (width, height) = (
            u32::try_from(image.width).map_err(|_| "clipboard image is too wide".to_owned())?,
            u32::try_from(image.height).map_err(|_| "clipboard image is too tall".to_owned())?,
        );
        let png = encode_png(width, height, &image.bytes)?;
        if png.len() > MAX_CLIPBOARD_IMAGE_BYTES {
            return Err(format!(
                "clipboard image is {} after PNG encoding; the bound is {}",
                render_byte_size(png.len()),
                render_byte_size(MAX_CLIPBOARD_IMAGE_BYTES),
            ));
        }
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
        return Ok(ClipboardContent::Image {
            data_uri: format!("data:image/png;base64,{encoded}"),
            width,
            height,
        });
    }
    match clipboard.get_text() {
        Ok(text) if !text.is_empty() => Ok(ClipboardContent::Text(text)),
        _ => Ok(ClipboardContent::Empty),
    }
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    use image::ImageEncoder as _;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|error| format!("clipboard image could not be encoded: {error}"))?;
    Ok(png)
}

fn render_byte_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{}KB", bytes.div_ceil(1024))
    }
}

fn start_prepared_send(
    session: smith_runtime::SessionHandle,
    text: String,
    files: Vec<String>,
    timeout_ms: u64,
    outcomes: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
) {
    tokio::spawn(async move {
        match prepare_attached_input(&session, text.clone(), files, timeout_ms).await {
            Ok(input) => {
                if let Err(error) = session.send(input) {
                    let _ = outcomes.send(LocalOutcome::PreparedSendFailed {
                        text,
                        error: format!(
                            "turn submission was rejected after attachment reads: {error}"
                        ),
                    });
                }
            }
            Err(error) => {
                let _ = outcomes.send(LocalOutcome::PreparedSendFailed { text, error });
            }
        }
    });
}

async fn prepare_attached_input(
    session: &smith_runtime::SessionHandle,
    text: String,
    files: Vec<String>,
    timeout_ms: u64,
) -> Result<UserInput, String> {
    const MAX_ATTACHMENT_CHARS: usize = 512 * 1024;
    let mut parts = vec![ContentPart::text(text)];
    let mut attached_chars = 0usize;
    for path in files {
        let block = session
            .run_local_tool(
                "read",
                serde_json::json!({ "path": path, "offset": 1, "limit": 2_000 }),
                timeout_ms,
            )
            .await
            .map_err(|error| format!("attachment `@{path}` was not sent: {error}"))?;
        if block.is_error {
            return Err(format!(
                "attachment `@{path}` was not sent: {}",
                tool_result_text(&block)
            ));
        }
        let content = tool_result_text(&block);
        attached_chars = attached_chars.saturating_add(content.chars().count());
        if attached_chars > MAX_ATTACHMENT_CHARS {
            return Err(format!(
                "prepared attachments exceed the {MAX_ATTACHMENT_CHARS}-character bound"
            ));
        }
        parts.push(ContentPart::text(format!(
            "<smith_file_attachment path=\"{path}\" source=\"prepared_read\">\n{content}\n</smith_file_attachment>"
        )));
    }
    Ok(UserInput { parts })
}

fn tool_result_text(block: &ToolResultBlock) -> String {
    let text = block
        .content
        .iter()
        .filter_map(ContentPart::as_text)
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        "tool completed without text output".to_owned()
    } else {
        text
    }
}

fn child_summary_projection(status: &ChildStatus) -> (&'static str, String) {
    let state = match &status.state {
        ChildState::Running => "working",
        ChildState::Idle => "idle",
        ChildState::Interrupted { .. } => "interrupted",
        ChildState::Stopped { .. } => "stopped",
        ChildState::Failed => "failed",
        ChildState::Expired => "expired",
    };
    let durability = match status.durability {
        ChildDurability::Ephemeral => "ephemeral",
        ChildDurability::Durable => "durable",
    };
    let mut detail = format!(
        "{durability} · session {} · {}/{} turns · {} tokens",
        status.session, status.turns_used, status.max_turns, status.tokens_used
    );
    if status.resumable() {
        detail.push_str(" · resumable");
    }
    if let Some(reason) = &status.incompatibility {
        detail.push_str(" · blocked: ");
        detail.push_str(reason);
    }
    (state, detail)
}

fn start_agent(
    host: &HostSession,
    agents: &ResolvedAgent,
    preset: String,
    task: String,
    outcomes: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
) {
    let Some(profile) = agents.child_profile(&preset).cloned() else {
        let _ = outcomes.send(LocalOutcome::Error(format!(
            "profile `{preset}` is not available for direct-child use"
        )));
        return;
    };
    let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .cloned()
    else {
        let _ = outcomes.send(LocalOutcome::Error(
            "child delegation is unavailable because the coordinator is not wired".to_owned(),
        ));
        return;
    };
    let model = match (&profile.provider, &profile.model) {
        (Some(_provider), Some(model)) => ChildModelSelection::Explicit {
            provider: Some(smith_runtime::delegation::profile_route_key(
                &profile.name,
                &profile.revision,
            )),
            model: ModelId::new(model.value.clone()),
        },
        (None, None) if profile.legacy => ChildModelSelection::Inherit,
        _ => {
            let _ = outcomes.send(LocalOutcome::Error(format!(
                "profile `{preset}` does not resolve a complete provider/model pair"
            )));
            return;
        }
    };
    let profile_revision = profile.revision.clone();
    let posture = profile.posture.value.as_str();
    tokio::spawn(async move {
        let outcome = coordinator
            .spawn(ChildSpec {
                task: UserInput::text(format!(
                    "Run this bounded task under the preflighted `{preset}` agent profile (revision {profile_revision}, posture {posture}) as a read-only direct child. Do not modify the workspace.\n\nTask:\n{task}"
                )),
                model,
                limits: ChildLimits::turns(1),
                tools: ToolViewScope::ReadOnly,
                workspace: WorkspacePolicy::ReadOnlyView,
            })
            .await;
        let message = match outcome {
            Ok(SpawnOutcome::Spawned { child, .. }) => LocalOutcome::Notice {
                source: "agents",
                text: format!("{preset} child {child} started"),
            },
            Ok(SpawnOutcome::Queued { child }) => LocalOutcome::Notice {
                source: "agents",
                text: format!("{preset} child {child} queued"),
            },
            Ok(SpawnOutcome::AtCapacity { running, limit }) => LocalOutcome::Error(format!(
                "{preset} child did not start: {running} children are already running (limit {limit})"
            )),
            Err(error) => {
                LocalOutcome::Error(format!("{preset} child did not start: {}", error.message))
            }
        };
        let _ = outcomes.send(message);
    });
}

fn follow_up_agent(
    host: &HostSession,
    child_id: String,
    task: String,
    outcomes: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
) {
    let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .cloned()
    else {
        let _ = outcomes.send(LocalOutcome::Error(
            "child follow-up is unavailable because the coordinator is not wired".to_owned(),
        ));
        return;
    };
    tokio::spawn(async move {
        let child = agent_runtime_core::ids::ChildId::new(child_id);
        let message = match coordinator.follow_up(&child, UserInput::text(task)).await {
            Ok(()) => LocalOutcome::Notice {
                source: "agents",
                text: format!("{child} follow-up started · same child session and prior history"),
            },
            Err(error) => LocalOutcome::Error(format!(
                "{child} follow-up did not start: {}",
                error.message
            )),
        };
        let _ = outcomes.send(message);
    });
}

fn resume_agent(
    host: &HostSession,
    child_id: String,
    outcomes: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
) {
    let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .cloned()
    else {
        let _ = outcomes.send(LocalOutcome::Error(
            "child resume is unavailable because the coordinator is not wired".to_owned(),
        ));
        return;
    };
    tokio::spawn(async move {
        let child = agent_runtime_core::ids::ChildId::new(child_id);
        let message = match coordinator.resume(&child).await {
            Ok(()) => LocalOutcome::Notice {
                source: "agents",
                text: format!("{child} exact checkpoint resume started · no new child task"),
            },
            Err(error) => LocalOutcome::Error(format!("{child} did not resume: {}", error.message)),
        };
        let _ = outcomes.send(message);
    });
}

fn start_review(
    host: &HostSession,
    project: &std::path::Path,
    scope: String,
    outcomes: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
) {
    let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .cloned()
    else {
        let _ = outcomes.send(LocalOutcome::Error(
            "read-only review is unavailable because delegation is not wired".to_owned(),
        ));
        return;
    };
    let view = match GitChanges::discover(project).and_then(|git| git.inspect(Some(scope.as_str())))
    {
        Ok(view) => view,
        Err(error) => {
            let _ = outcomes.send(LocalOutcome::Error(error.message));
            return;
        }
    };
    let task = format!(
        "Review this bounded Git diff. Do not modify the workspace. Report only actionable \
         findings, ordered by severity, with file and line evidence. If there are no findings, \
         say so explicitly.\n\nScope: {}\n\n{}",
        view.title, view.content
    );
    tokio::spawn(async move {
        let outcome = coordinator
            .spawn(ChildSpec {
                task: UserInput::text(task),
                model: ChildModelSelection::Inherit,
                limits: ChildLimits::turns(1),
                tools: ToolViewScope::ReadOnly,
                workspace: WorkspacePolicy::ReadOnlyView,
            })
            .await;
        let message = match outcome {
            Ok(SpawnOutcome::Spawned { child, .. }) => LocalOutcome::Notice {
                source: "review",
                text: format!("read-only reviewer {child} started"),
            },
            Ok(SpawnOutcome::Queued { child }) => LocalOutcome::Notice {
                source: "review",
                text: format!("read-only reviewer {child} queued"),
            },
            Ok(SpawnOutcome::AtCapacity { running, limit }) => LocalOutcome::Error(format!(
                "review did not start: {running} children are already running (limit {limit})"
            )),
            Err(error) => LocalOutcome::Error(format!("review did not start: {}", error.message)),
        };
        let _ = outcomes.send(message);
    });
}

async fn next_approval(approvals: &mut Option<ApprovalRequests>) -> Option<ApprovalPrompt> {
    match approvals {
        Some(approvals) => approvals.recv().await,
        None => std::future::pending().await,
    }
}

struct Prepared {
    resolution: Resolution,
    project: PathBuf,
}

fn inspect_selection(selection: &Selection) -> Result<ConfigReadiness> {
    let (_, request) = resolution_request(selection)?;
    Ok(inspect(&request))
}

fn prepare(selection: &Selection) -> Result<Prepared> {
    let (start, request) = resolution_request(selection)?;
    let resolution = resolve(&request)
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("resolving Smith configuration")?;
    let project = resolution.layout.project_root.clone().unwrap_or(start);
    Ok(Prepared {
        resolution,
        project,
    })
}

fn resolution_request(selection: &Selection) -> Result<(PathBuf, ResolveRequest)> {
    let start = match &selection.project {
        Some(project) => project.clone(),
        None => std::env::current_dir().context("reading the current directory")?,
    };
    let start = start
        .canonicalize()
        .with_context(|| format!("resolving project path `{}`", start.display()))?;
    if !start.is_dir() {
        anyhow::bail!("project path `{}` is not a directory", start.display());
    }

    let request = ResolveRequest::new(&start)
        .with_env(std::env::vars())
        .with_cli(selection.overrides())
        .with_session(selection.session_overrides());
    Ok((start, request))
}

fn explain_config(key: &str, selection: &Selection) -> Result<()> {
    let prepared = prepare(selection)?;
    let explanation = prepared
        .resolution
        .provenance
        .explain(key)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    println!("{} = {}", explanation.key, explanation.value);
    println!("source: {}", explanation.source);
    for entry in explanation.overridden {
        println!("overrode: {} from {}", entry.value, entry.source);
    }
    Ok(())
}

fn runtime_resources(
    inventory: SelectionInventory,
    sessions: Vec<SessionListing>,
    current_session: &str,
    project: &std::path::Path,
    agents: &ResolvedAgent,
    reasoning: &smith_runtime::reasoning::ReasoningRuntimePolicy,
) -> RuntimeResources {
    let model_limits = inventory
        .models
        .iter()
        .map(|model| {
            (
                model.id(),
                format!(
                    "ctx {} · input {} · output {}",
                    render_optional_inventory_limit(model.context_tokens.as_ref()),
                    render_optional_inventory_limit(model.max_input_tokens.as_ref()),
                    render_optional_inventory_limit(model.max_output_tokens.as_ref()),
                ),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let profile_inventory = inventory.profiles;
    let profiles = profile_inventory
        .iter()
        .filter(|profile| profile.uses.contains(&ProfileUse::Main))
        .map(|profile| {
            let pair = profile
                .pair()
                .unwrap_or_else(|| "incomplete provider/model selection".to_owned());
            let description = profile.description.as_deref().unwrap_or("agent profile");
            let placements = profile
                .uses
                .iter()
                .map(|placement| placement.as_str())
                .collect::<Vec<_>>()
                .join("+");
            let revision = bounded_text(&profile.revision, 12);
            let source = profile
                .source
                .as_ref()
                .map_or_else(|| "unknown source".to_owned(), ToString::to_string);
            let legacy = if profile.legacy {
                format!(
                    " · legacy from {}; migrate to [profiles.{}]",
                    bounded_text(&source, 48),
                    profile.name
                )
            } else {
                format!(" · source {}", bounded_text(&source, 48))
            };
            let detail = format!(
                "{} · use {placements} · {pair} · {description} · rev {revision}{legacy}",
                profile.posture.as_str(),
            );
            let id = if profile.legacy {
                format!("{LEGACY_AGENT_PROFILE_PREFIX}{}", profile.name)
            } else {
                profile.name.clone()
            };
            let entry = ResourceEntry::new(id, profile.name.clone(), detail).active(profile.active);
            if profile.selectable {
                entry
            } else {
                entry.disabled("profile does not resolve to a usable provider/model pair")
            }
        })
        .collect();
    let providers = inventory
        .providers
        .into_iter()
        .map(|provider| {
            let kind = provider.kind.as_deref().unwrap_or("missing adapter kind");
            let detail = format!(
                "{kind} · {} {}",
                provider.model_count,
                if provider.model_count == 1 {
                    "model"
                } else {
                    "models"
                }
            );
            let entry = ResourceEntry::new(provider.name.clone(), provider.name, detail)
                .active(provider.active);
            if provider.selectable {
                entry
            } else {
                entry.disabled("adapter unavailable or no model with enforceable limits")
            }
        })
        .collect();
    let models = inventory
        .models
        .into_iter()
        .map(|model| {
            let id = model.id();
            let profiles = if model.profiles.is_empty() {
                String::new()
            } else {
                format!(" · profiles {}", model.profiles.join(","))
            };
            let capabilities = [
                model
                    .tool_call
                    .is_some_and(|enabled| enabled)
                    .then_some("tools"),
                model
                    .reasoning
                    .is_some_and(|enabled| enabled)
                    .then_some("reasoning"),
                model
                    .structured_output
                    .is_some_and(|enabled| enabled)
                    .then_some("structured"),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            let capabilities = if capabilities.is_empty() {
                String::new()
            } else {
                format!(" · {}", capabilities.join("+"))
            };
            let provenance = match (
                model.catalog_provider.as_deref(),
                model.catalog_revision.as_deref(),
                model.catalog_retrieved_at_ms,
            ) {
                (Some(provider), Some(revision), Some(retrieved)) => format!(
                    " · models.dev/{provider} advertised · rev {} · {} old",
                    bounded_text(revision, 12),
                    catalog_age(retrieved)
                ),
                _ => String::new(),
            };
            let entry = ResourceEntry::new(
                id.clone(),
                model.label,
                format!(
                    "{id} · ctx {} · input {} · output {}{capabilities}{provenance}{profiles}",
                    render_optional_inventory_limit(model.context_tokens.as_ref()),
                    render_optional_inventory_limit(model.max_input_tokens.as_ref()),
                    render_optional_inventory_limit(model.max_output_tokens.as_ref()),
                ),
            )
            .active(model.active);
            match model.disabled_reason {
                Some(reason) => entry.disabled(reason),
                None if model.selectable => entry,
                None => entry.disabled("model is not locally selectable"),
            }
        })
        .collect();

    let session_entries = session_resource_entries(sessions, Some(current_session));
    let files = workspace_file_entries(project, 4_096);
    let child_agents = profile_inventory
        .iter()
        .filter(|profile| profile.uses.contains(&ProfileUse::Child))
        .map(|profile| {
            let description = profile
                .description
                .as_deref()
                .unwrap_or("read-only child profile");
            let pair = profile.pair().unwrap_or_else(|| {
                format!(
                    "{}/{}",
                    agents
                        .profile
                        .provider
                        .as_ref()
                        .map_or("current", |value| value.value.as_str()),
                    agents
                        .profile
                        .model
                        .as_ref()
                        .map_or("model", |value| value.value.as_str())
                )
            });
            let limits = model_limits
                .get(&pair)
                .map_or("limits inherited from active runtime", String::as_str);
            let instructions = agents
                .profiles
                .get(&profile.name)
                .and_then(|resolved| resolved.instructions.as_ref())
                .map_or("default instructions", |_| "custom instructions configured");
            let revision = bounded_text(&profile.revision, 12);
            let source = profile
                .source
                .as_ref()
                .map_or_else(|| "unknown source".to_owned(), ToString::to_string);
            let legacy = if profile.legacy {
                format!(
                    " · legacy from {}; migrate to [profiles.{}] use=[\"child\"]",
                    bounded_text(&source, 48),
                    profile.name
                )
            } else {
                format!(" · source {}", bounded_text(&source, 48))
            };
            let entry = ResourceEntry::new(
                format!("agent:{}", profile.name),
                profile.name.clone(),
                format!(
                    "child profile · {} · {pair} · {limits} · {instructions} · {description} · rev {revision}{legacy}",
                    profile.posture.as_str(),
                ),
            );
            if profile.selectable {
                entry
            } else {
                entry.disabled("child profile does not resolve a usable provider/model pair")
            }
        })
        .collect();
    let main_profiles = agents
        .profile_order
        .value
        .iter()
        .filter_map(|name| {
            let profile = profile_inventory
                .iter()
                .find(|profile| profile.name == *name)?;
            let description = profile
                .description
                .as_deref()
                .unwrap_or("main agent profile");
            Some(
                ResourceEntry::new(
                    if profile.legacy {
                        format!("{LEGACY_AGENT_PROFILE_PREFIX}{name}")
                    } else {
                        name.clone()
                    },
                    name.clone(),
                    format!(
                        "main profile · {} · {description} · rev {}",
                        profile.posture.as_str(),
                        bounded_text(&profile.revision, 12)
                    ),
                )
                .active(agents.profile.name == *name),
            )
        })
        .collect();

    let capability_reason = || match reasoning.support {
        ReasoningSupport::Unsupported => {
            "this model does not advertise reasoning support".to_owned()
        }
        ReasoningSupport::Fixed => {
            format!("reasoning is fixed; {}", reasoning.capability_source)
        }
        ReasoningSupport::Controllable => format!(
            "the active binding has no explicit switch; {}",
            reasoning.capability_source
        ),
    };
    let mut thinking = vec![
        ResourceEntry::new(
            "default",
            "provider default",
            "clear the session thinking override",
        )
        .active(reasoning.selected_enabled.is_none()),
    ];
    let on = ResourceEntry::new("on", "on", "enable thinking for the next turn")
        .active(reasoning.selected_enabled == Some(true));
    thinking.push(match reasoning.switch {
        smith_runtime::reasoning::ReasoningSwitch::Optional
        | smith_runtime::reasoning::ReasoningSwitch::MandatoryOn
            if reasoning.dialect != Some(smith_config::model::ReasoningDialect::OpenaiEffort)
                || reasoning.selected_effort.is_some()
                || reasoning.default_effort.is_some() =>
        {
            on
        }
        smith_runtime::reasoning::ReasoningSwitch::Optional
        | smith_runtime::reasoning::ReasoningSwitch::MandatoryOn => {
            on.disabled("choose an advertised /effort to turn reasoning on")
        }
        smith_runtime::reasoning::ReasoningSwitch::Unavailable => on.disabled(capability_reason()),
    });
    let off = ResourceEntry::new("off", "off", "disable thinking for the next turn")
        .active(reasoning.selected_enabled == Some(false));
    thinking.push(match reasoning.switch {
        // The OpenAI-effort dialect sends off as the effort `none`, so off is
        // selectable only when that effort is advertised. Mirrors the
        // validation in `smith_runtime::reasoning::resolve_reasoning_policy`.
        smith_runtime::reasoning::ReasoningSwitch::Optional
            if reasoning.dialect == Some(smith_config::model::ReasoningDialect::OpenaiEffort)
                && !reasoning.efforts.iter().any(|effort| effort == "none") =>
        {
            off.disabled("off requires this binding to advertise the `none` effort")
        }
        smith_runtime::reasoning::ReasoningSwitch::Optional => off,
        smith_runtime::reasoning::ReasoningSwitch::MandatoryOn => {
            off.disabled("reasoning is mandatory for this provider/model")
        }
        smith_runtime::reasoning::ReasoningSwitch::Unavailable => off.disabled(capability_reason()),
    });

    let mut efforts = vec![
        ResourceEntry::new(
            "default",
            "provider default",
            "clear the session effort override",
        )
        .active(reasoning.selected_effort.is_none()),
    ];
    efforts.extend(reasoning.efforts.iter().map(|effort| {
        ResourceEntry::new(
            effort.clone(),
            effort.clone(),
            "applies to every request in the next turn",
        )
        .active(reasoning.selected_effort.as_deref() == Some(effort.as_str()))
    }));
    if reasoning.efforts.is_empty() {
        efforts.push(
            ResourceEntry::new("unavailable", "not adjustable", capability_reason())
                .disabled(capability_reason()),
        );
    }

    RuntimeResources {
        models,
        providers,
        profiles,
        sessions: session_entries,
        files,
        child_agents,
        main_profiles,
        thinking,
        efforts,
        current_session: Some(current_session.to_owned()),
    }
}

fn workspace_file_entries(project: &std::path::Path, limit: usize) -> Vec<ResourceEntry> {
    let Ok(root) = project.canonicalize() else {
        return Vec::new();
    };
    let mut walker = WalkBuilder::new(&root);
    walker
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git");

    let mut entries = Vec::new();
    for entry in walker.build().filter_map(Result::ok) {
        if entries.len() >= limit {
            break;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(canonical) = entry.path().canonicalize() else {
            continue;
        };
        if !canonical.starts_with(&root) {
            continue;
        }
        let Ok(relative) = canonical.strip_prefix(&root) else {
            continue;
        };
        let path = relative.to_string_lossy().replace('\\', "/");
        if path.is_empty() {
            continue;
        }
        let bytes = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        entries.push(ResourceEntry::new(
            format!("file:{path}"),
            path,
            format!("file · {bytes} bytes"),
        ));
    }
    entries.sort_by(|left, right| left.label.cmp(&right.label));
    entries
}

fn session_resource_entries(
    sessions: Vec<SessionListing>,
    current: Option<&str>,
) -> Vec<ResourceEntry> {
    sessions
        .into_iter()
        .map(|session| {
            let id = session.id.as_str().to_owned();
            let active = current.is_some_and(|session| id == session);
            let preview = session
                .user_preview
                .as_deref()
                .map(|preview| bounded_text(preview, 64))
                .unwrap_or_else(|| "No user preview".to_owned());
            let turns = session
                .turn_count
                .map_or_else(|| "? turns".to_owned(), |count| format!("{count} turns"));
            let pair = match (session.provider.as_deref(), session.model.as_deref()) {
                (Some(provider), Some(model)) => format!("{provider}/{model}"),
                _ => "unknown provider/model".to_owned(),
            };
            let updated = session.updated.map_or_else(
                || "unknown update".to_owned(),
                |updated| updated.to_string(),
            );
            let entry = ResourceEntry::new(
                &id,
                format!("{} · {preview}", short_session_id(&id)),
                format!("{turns} · {pair} · {updated}"),
            )
            .active(active);
            if session.schema_version == SNAPSHOT_SCHEMA_VERSION {
                entry
            } else {
                entry.disabled(format!(
                    "snapshot schema {} is newer than this build",
                    session.schema_version
                ))
            }
        })
        .collect()
}

fn render_inventory_limit(limit: &InventoryLimit) -> String {
    let provenance = match &limit.origin {
        ModelLimitOrigin::Configured(source) => source.layer.label().to_owned(),
        ModelLimitOrigin::Trusted { catalog, revision } => {
            format!("{catalog} r{revision}")
        }
        ModelLimitOrigin::Catalog {
            catalog,
            revision: _,
            retrieved_at_ms: _,
        } => catalog.clone(),
    };
    format!("{} [{provenance}]", token_quantity(limit.value))
}

fn render_optional_inventory_limit(limit: Option<&InventoryLimit>) -> String {
    limit.map_or_else(|| "unknown".to_owned(), render_inventory_limit)
}

fn catalog_age(retrieved_at_ms: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        });
    let seconds = now.saturating_sub(retrieved_at_ms) / 1_000;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 60 * 60 {
        format!("{}m", seconds / 60)
    } else if seconds < 24 * 60 * 60 {
        format!("{}h", seconds / (60 * 60))
    } else {
        format!("{}d", seconds / (24 * 60 * 60))
    }
}

fn token_quantity(tokens: u32) -> String {
    if tokens >= 1_000 && tokens.is_multiple_of(1_000) {
        format!("{}k", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

fn short_session_id(id: &str) -> String {
    bounded_text(id, 12)
}

fn bounded_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let head = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

async fn choose_resume_session(
    selection: &Selection,
    no_color: bool,
    no_motion: bool,
) -> Result<Option<String>> {
    let prepared = prepare(selection)?;
    let sessions = smith_runtime::host::list(&prepared.resolution.config, &prepared.project)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("listing project sessions")?;
    let entries = session_resource_entries(sessions, None);
    let mut picker = ResourcePicker::new(
        "Resume session",
        entries,
        "Nothing to resume for this project · Esc to start without resuming",
    );
    let mut theme = Theme::from_env();
    if no_color {
        theme = theme.without_color();
    }
    if no_motion {
        theme = theme.without_motion();
    }

    let mut terminal = terminal::enter().context("entering the resume picker")?;
    let mut events = EventStream::new();
    let result = async {
        terminal.draw(|frame| {
            let area = standalone_picker_area(frame.area(), picker.entries.len());
            draw_resource_picker(frame, area, &picker, theme);
        })?;
        loop {
            let Some(event) = events.next().await else {
                return Ok(None);
            };
            match event.context("reading a terminal event")? {
                TermEvent::Key(key) => match picker.on_key(key) {
                    PickerOutcome::Pending => {}
                    PickerOutcome::Cancelled => return Ok(None),
                    PickerOutcome::Selected(session) => return Ok(Some(session)),
                },
                TermEvent::Paste(text) => picker.paste(&text),
                TermEvent::Resize(_, _) => {}
                _ => continue,
            }
            terminal.draw(|frame| {
                let area = standalone_picker_area(frame.area(), picker.entries.len());
                draw_resource_picker(frame, area, &picker, theme);
            })?;
        }
    }
    .await;
    let restore = terminal.restore().context("restoring the terminal");
    restore?;
    result
}

fn standalone_picker_area(area: Rect, entry_count: usize) -> Rect {
    if area.width < 24 || area.height < 8 {
        return area;
    }
    let width = area.width.saturating_sub(4).min(100);
    let height = u16::try_from(entry_count.saturating_add(4))
        .unwrap_or(u16::MAX)
        .clamp(6, area.height.saturating_sub(2));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

async fn list_sessions(selection: &Selection) -> Result<()> {
    let prepared = prepare(selection)?;
    let sessions = smith_runtime::host::list(&prepared.resolution.config, &prepared.project)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    for session in sessions {
        let updated = session.updated.map_or_else(
            || "unknown-version".to_owned(),
            |updated| updated.to_string(),
        );
        let turns = session
            .turn_count
            .map_or_else(|| "?".to_owned(), |turns| turns.to_string());
        let provider = session.provider.as_deref().unwrap_or("?");
        let model = session.model.as_deref().unwrap_or("?");
        let preview = session.user_preview.as_deref().unwrap_or("no user preview");
        println!(
            "{}\t{updated}\t{turns}\t{provider}/{model}\t{}",
            session.id.as_str(),
            bounded_text(preview, 80)
        );
    }
    Ok(())
}

/// Shortens a home-relative path to `~/…` for the header.
fn abbreviate_home(path: &str) -> String {
    match std::env::var_os("HOME") {
        Some(home) => abbreviate(path, &home.to_string_lossy()),
        None => path.to_owned(),
    }
}

/// The pure half of [`abbreviate_home`].
fn abbreviate(path: &str, home: &str) -> String {
    match path.strip_prefix(home) {
        Some("") => "~".to_owned(),
        Some(rest) => format!("~{rest}"),
        None => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use smith_runtime::checkpoint::{
        CheckpointKey, CheckpointKeyProvider, CheckpointProtectionError,
    };
    use smith_tui::{Block, LocalResultState};

    #[derive(Debug)]
    struct TestCheckpointKeys;

    impl CheckpointKeyProvider for TestCheckpointKeys {
        fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
            Ok(CheckpointKey::new([0x52; 32]))
        }
    }

    const LOCAL_COMMAND_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;

    #[test]
    fn tool_display_enrichment_runs_at_request_and_completion_boundaries() {
        let call = agent_runtime_core::ids::ToolCallId::new("call-display");
        let requested = RuntimeEvent::ToolCallRequested {
            call: call.clone(),
            name: "read".to_owned(),
            argument_keys: vec!["path".to_owned()],
            argument_fingerprint: serde_json::from_value(serde_json::json!(
                "0123456789abcdef0123456789abcdef"
            ))
            .expect("fingerprint"),
            arguments: None,
        };
        let completed = RuntimeEvent::ToolCallCompleted {
            call: call.clone(),
            name: "read".to_owned(),
            is_error: false,
        };

        assert_eq!(tool_call_for_display(&requested), Some(call.clone()));
        assert_eq!(tool_call_for_display(&completed), Some(call));
        assert!(tool_call_for_display(&RuntimeEvent::TurnStarted).is_none());
    }

    #[test]
    fn runtime_timeline_uses_stable_ids_terminal_plan_and_redacted_gate_evidence() {
        use agent_runtime_core::clock::Timestamp;
        use agent_runtime_core::event::PlanSensitivity;
        use agent_runtime_core::ids::{EventId, ToolCallId, TurnId};

        let session = SessionId::new("session-1");
        let turn = TurnId::new("turn-7");
        let envelope = |seq, payload| {
            EventEnvelope::new(
                seq,
                EventId::new(format!("event-{seq}")),
                session.clone(),
                Some(turn.clone()),
                Timestamp::ZERO,
                payload,
            )
        };
        let events = vec![
            envelope(
                1,
                RuntimeEvent::PlanUpdated {
                    revision: 3,
                    sensitivity: PlanSensitivity::Public,
                    counts: BTreeMap::from([
                        ("pending".to_owned(), 0),
                        ("in_progress".to_owned(), 0),
                        ("completed".to_owned(), 2),
                        ("cancelled".to_owned(), 1),
                    ]),
                    items: Some(Vec::new()),
                },
            ),
            envelope(
                2,
                RuntimeEvent::ToolCallRequested {
                    call: ToolCallId::new("call-4"),
                    name: "shell".to_owned(),
                    argument_keys: vec!["command".to_owned()],
                    argument_fingerprint: serde_json::from_value(serde_json::json!(
                        "0123456789abcdef0123456789abcdef"
                    ))
                    .expect("fingerprint"),
                    arguments: Some(serde_json::json!({"command": "secret-command"})),
                },
            ),
            envelope(
                3,
                RuntimeEvent::ToolCallCompleted {
                    call: ToolCallId::new("call-4"),
                    name: "shell".to_owned(),
                    is_error: false,
                },
            ),
            envelope(
                4,
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ),
            envelope(
                5,
                RuntimeEvent::ChildSpawned {
                    child: ChildId::new("child-2"),
                    workspace: WorkspacePolicy::ReadOnlyView,
                    max_turns: 1,
                    max_tokens: None,
                    deadline_ms: None,
                },
            ),
            envelope(
                6,
                RuntimeEvent::ChildCompleted {
                    child: ChildId::new("child-2"),
                    result: "secret-child-result".to_owned(),
                },
            ),
        ];

        let rendered = render_runtime_timeline(&events);
        assert_eq!(
            rendered.lines,
            [
                "root turn-7 · completed · plan 0 active/0 pending/2 done/1 cancelled · gates 1 passed/0 failed",
                "child child-2 · started · ReadOnlyView · 1 turn limit",
                "child child-2 · task completed",
            ]
        );
        assert_eq!(rendered.children, BTreeSet::from([ChildId::new("child-2")]));
        let joined = rendered.lines.join("\n");
        assert!(!joined.contains("secret-command"));
        assert!(!joined.contains("secret-child-result"));
    }

    #[tokio::test]
    async fn prepared_file_attachment_reads_exactly_without_provider_spend() {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        std::fs::create_dir_all(project.path().join(".smith")).expect("config directory");
        std::fs::write(
            project.path().join(".smith/config.toml"),
            LOCAL_COMMAND_CONFIG,
        )
        .expect("config");
        std::fs::create_dir_all(project.path().join("src")).expect("source directory");
        std::fs::write(
            project.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        )
        .expect("source file");
        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolution")
            .config;
        let provider = Arc::new(agent_runtime::provider::fake::FakeProvider::new(
            "example-model",
            agent_runtime_core::provider::Capabilities::basic_streaming(),
            Vec::new(),
        ));
        let runtime = RuntimeRequest {
            provider: Some(provider.clone()),
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("workspace"),
            )),
            approval: Some(Arc::new(agent_runtime_core::approval::DenyAll)),
            ..RuntimeRequest::new(config, HostSurface::Terminal)
        };
        let host = smith_runtime::host::start(
            HostSessionRequest::new(runtime, project.path())
                .checkpoint_keys(Arc::new(TestCheckpointKeys)),
        )
        .await
        .expect("host");
        let history_before = host.session().history();

        let input = prepare_attached_input(
            host.session(),
            "explain this".to_owned(),
            vec!["src/lib.rs".to_owned()],
            5_000,
        )
        .await
        .expect("prepared attachment");
        assert_eq!(input.parts.len(), 2);
        let wire = serde_json::to_string(&input).expect("input wire");
        assert!(wire.contains("source=\\\"prepared_read\\\""), "{wire}");
        assert!(wire.contains("pub fn answer()"), "{wire}");
        assert_eq!(host.session().history(), history_before);
        assert!(provider.requests().is_empty());

        let error = prepare_attached_input(
            host.session(),
            "do not send".to_owned(),
            vec!["../outside.txt".to_owned()],
            5_000,
        )
        .await
        .expect_err("workspace escape must fail locally");
        assert!(error.contains("was not sent"), "{error}");
        assert!(provider.requests().is_empty());

        host.shutdown().await.expect("shutdown");
    }

    #[test]
    fn a_home_relative_path_is_abbreviated() {
        let home = "/Users/example";
        assert_eq!(abbreviate("/Users/example/work/api", home), "~/work/api");
        assert_eq!(abbreviate("/Users/example", home), "~");
        assert_eq!(abbreviate("/opt/other", home), "/opt/other");
    }

    #[test]
    fn stdin_prompts_are_non_empty_utf8_and_bounded() {
        assert_eq!(read_prompt(&b"hello"[..]).expect("a prompt"), "hello");
        assert!(read_prompt(&b"   \n"[..]).is_err());
        assert!(read_prompt(&[0xff][..]).is_err());
        assert!(read_prompt(vec![b'x'; MAX_STDIN_PROMPT_BYTES + 1].as_slice()).is_err());
    }

    #[test]
    fn palette_reconfiguration_preserves_or_replaces_the_intended_session() {
        let mut selection = Selection {
            profile: Some("old".into()),
            provider: Some("explicit-provider".into()),
            model: Some("explicit-model".into()),
            ..Selection::default()
        };
        let mut resume = Some("older-session".into());

        apply_palette_command(
            &mut selection,
            &mut resume,
            "current-session".into(),
            PaletteCommand::Profile("work".into()),
        );
        assert_eq!(selection.profile.as_deref(), Some("work"));
        assert_eq!(selection.provider, None);
        assert_eq!(selection.model, None);
        assert_eq!(resume.as_deref(), Some("current-session"));

        apply_palette_command(
            &mut selection,
            &mut resume,
            "current-session".into(),
            PaletteCommand::Agent("review".into()),
        );
        assert_eq!(selection.profile.as_deref(), Some("work"));
        assert_eq!(selection.agent.as_deref(), Some("review"));

        apply_palette_command(
            &mut selection,
            &mut resume,
            "current-session".into(),
            PaletteCommand::NewSession,
        );
        assert_eq!(resume, None);

        apply_palette_command(
            &mut selection,
            &mut resume,
            "ignored".into(),
            PaletteCommand::Resume("selected-session".into()),
        );
        assert_eq!(resume.as_deref(), Some("selected-session"));
    }

    #[test]
    fn reasoning_startup_errors_are_distinguished_for_compatible_switch_cleanup() {
        let reasoning = anyhow::Error::new(FactoryError::Reasoning {
            provider: "example".to_owned(),
            model: ModelId::new("fixed-model"),
            message: "reasoning is not adjustable".to_owned(),
        })
        .context("starting the Smith session");
        assert!(is_reasoning_startup_error(&reasoning));

        let unrelated = anyhow::anyhow!("provider is unavailable");
        assert!(!is_reasoning_startup_error(&unrelated));
    }

    #[test]
    fn catalog_inventory_becomes_searchable_resource_metadata_with_disabled_reasons() {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        std::fs::create_dir_all(project.path().join(".smith")).expect("config directory");
        std::fs::write(
            project.path().join(".smith/config.toml"),
            r#"
default_profile = "router"
[profiles.router]
provider = "openrouter"
model = "~openai/gpt-latest"
[providers.openrouter]
kind = "openai-compatible"
base_url = "https://openrouter.ai/api/v1"
credential = "env:OPENROUTER_API_KEY"
[context]
output_reserve = 4096
"#,
        )
        .expect("config");
        let resolution = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolution");
        let snapshot: smith_config::catalog::CatalogSnapshot =
            serde_json::from_str(smith_runtime::model_catalog::EMBEDDED_MODELS_DEV_SEED)
                .expect("embedded catalog");
        let inventory =
            local_inventory_with_catalog(&resolution, AVAILABLE_ADAPTER_KINDS, Some(&snapshot))
                .expect("catalog inventory");
        let selectable_count = inventory.providers[0].model_count;

        let resources = runtime_resources(
            inventory,
            Vec::new(),
            "session",
            project.path(),
            &resolution.config.agent,
            &smith_runtime::reasoning::ReasoningRuntimePolicy::default(),
        );
        assert_eq!(resources.models.len(), 335);
        assert_eq!(
            resources
                .main_profiles
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["router"]
        );
        assert!(resources.profiles.iter().any(|entry| {
            entry.label == "review" && entry.id == format!("{LEGACY_AGENT_PROFILE_PREFIX}review")
        }));
        assert!(
            resources.providers[0]
                .detail
                .contains(&format!("{selectable_count} models"))
        );
        let current = resources
            .models
            .iter()
            .find(|entry| entry.id == "openrouter/~openai/gpt-latest")
            .expect("nested catalog model");
        assert_eq!(current.label, "OpenAI GPT Latest");
        assert!(current.active);
        assert!(current.detail.contains("tools"), "{}", current.detail);
        assert!(current.detail.contains("advertised"), "{}", current.detail);
        let incompatible = resources
            .models
            .iter()
            .find(|entry| entry.id == "openrouter/mancer/weaver")
            .expect("advertised incompatible model");
        assert!(
            incompatible
                .disabled_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("tool"))
        );
    }

    #[test]
    fn the_production_cli_manifest_uses_the_full_facade_only_as_a_dev_dependency() {
        let manifest = include_str!("../Cargo.toml");
        let dependencies = manifest
            .split("[dev-dependencies]")
            .next()
            .expect("a dependencies section");
        assert!(
            !dependencies
                .lines()
                .any(|line| line.trim_start().starts_with("agent-runtime =")),
            "smith-cli must compose the full facade through smith-runtime"
        );
    }

    #[test]
    fn context_categories_name_tool_results_separately_from_user_input() {
        let tool = context_display_category("tool_result", 4_200);
        let user = context_display_category("user_input", 58);
        assert_eq!(tool.label, "tool results");
        assert_eq!(tool.glyph, glyph::CONTEXT_TOOL);
        assert_eq!(tool.tokens, 4_200);
        assert_eq!(user.label, "user input");
        assert_eq!(user.glyph, glyph::CONTEXT_INPUT);
        assert_eq!(user.tokens, 58);
        assert!(tool.rank < user.rank);

        let unknown = context_display_category("future_context_kind", 1);
        assert_eq!(unknown.label, "future context kind");
        assert_eq!(unknown.glyph, glyph::CONTEXT_OTHER);
    }

    #[test]
    fn context_categories_keep_system_and_tools_visible_and_aggregate_instructions() {
        let totals = std::collections::BTreeMap::from([
            ("system_instruction".to_owned(), 100),
            ("developer_instruction".to_owned(), 40),
            ("ability_instruction".to_owned(), 60),
            ("history".to_owned(), 300),
        ]);

        let categories = context_display_categories(&totals);
        assert_eq!(categories[0].label, "system instructions");
        assert_eq!(categories[0].tokens, 200);
        assert_eq!(categories[1].label, "tool schemas");
        assert_eq!(categories[1].tokens, 0);
        assert_eq!(categories[2].label, "history");
        assert!(
            categories
                .iter()
                .all(|category| category.label != "developer instructions")
        );
    }

    #[test]
    fn harness_status_names_registry_view_activation_and_context_provenance() {
        let mut status = Status::new("example-model", "/project");
        status.record_registry("registry-fingerprint", 6);
        status.record_scoped_view("view-fingerprint", 4);
        status.record_retrieval("resolver-1", vec!["tool:read".into(), "tool:search".into()]);
        status.record_activation(1, vec!["tool:read".into()]);
        let totals = std::collections::BTreeMap::new();
        status.record_context_plan(ContextPlanUpdate {
            fingerprint: "context-fingerprint",
            cache_fingerprint: "cache-fingerprint",
            input_tokens: 100,
            input_budget_tokens: 1_000,
            reserved_tokens: 100,
            segment_count: 1,
            totals: &totals,
            confidence: EstimationConfidence::Exact,
        });
        status.record_compaction(250);

        let rendered = render_harness_status(&status);
        assert!(
            rendered.contains("registry snapshot: registry-fingerprint · 6 entries"),
            "{rendered}"
        );
        assert!(
            rendered.contains("scoped capability view: view-fingerprint · 4 visible"),
            "{rendered}"
        );
        assert!(
            rendered.contains("activation epoch: 1 · tool:read"),
            "{rendered}"
        );
        assert!(
            rendered.contains("context provenance: context-fingerprint · cache cache-fingerprint"),
            "{rendered}"
        );
        assert!(
            rendered.contains("context compaction: 1 run(s) · 250 tokens reclaimed"),
            "{rendered}"
        );
    }

    #[tokio::test]
    async fn informational_commands_append_inline_without_provider_history() {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        std::fs::create_dir_all(project.path().join(".smith")).expect("config directory");
        std::fs::write(
            project.path().join(".smith/config.toml"),
            LOCAL_COMMAND_CONFIG,
        )
        .expect("config");
        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolution")
            .config;
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("workspace"),
            )),
            approval: Some(Arc::new(agent_runtime_core::approval::DenyAll)),
            ..RuntimeRequest::new(config, HostSurface::Terminal)
        };
        let host = smith_runtime::host::start(
            HostSessionRequest::new(runtime, project.path())
                .checkpoint_keys(Arc::new(TestCheckpointKeys)),
        )
        .await
        .expect("host");
        let history_before = host.session().history().len();
        let mut app = App::new("example-model", project.path().display().to_string());

        let before_plan = render_context_status(&app.status, host.runtime().policy());
        assert!(before_plan.contains("not planned yet"), "{before_plan}");
        assert!(
            before_plan.contains("128k total · 4k reserved"),
            "{before_plan}"
        );
        assert!(
            before_plan.contains("compaction: enabled on overflow · 74.3k recovery target"),
            "{before_plan}"
        );
        let before_context = render_context_view(&app.status, host.runtime().policy());
        assert!(
            before_context.contains("usage unavailable until the first turn"),
            "{before_context}"
        );
        assert!(
            before_context.contains("· free input: 123.9k"),
            "{before_context}"
        );
        assert!(
            before_context.contains("□ output/reasoning reserve: 4k"),
            "{before_context}"
        );
        assert!(
            before_context.contains("■ system instructions: ? (not counted yet)"),
            "{before_context}"
        );
        assert!(
            before_context.contains("◆ tool schemas: ? (not counted yet)"),
            "{before_context}"
        );
        assert_eq!(
            before_context
                .lines()
                .filter(|line| {
                    !line.is_empty()
                        && line
                            .chars()
                            .all(|character| matches!(character, '·' | '□' | ' '))
                })
                .count(),
            5,
            "{before_context}"
        );

        let totals = std::collections::BTreeMap::from([
            (
                agent_runtime_core::manifest::SegmentKind::new("history"),
                1_500,
            ),
            (
                agent_runtime_core::manifest::SegmentKind::new("tool_schema"),
                500,
            ),
        ]);
        app.status.record_context_plan(ContextPlanUpdate {
            fingerprint: "context-test",
            cache_fingerprint: "cache-test",
            input_tokens: 2_000,
            input_budget_tokens: 123_904,
            reserved_tokens: 4_096,
            segment_count: 2,
            totals: &totals,
            confidence: EstimationConfidence::Estimated,
        });
        let planned = render_context_status(&app.status, host.runtime().policy());
        assert!(planned.contains("~98% input left"), "{planned}");
        assert!(planned.contains("~2k used / 123.9k budget"), "{planned}");
        assert!(planned.contains("provider input (session): ?"), "{planned}");
        assert!(planned.contains("tool schema: ~500"), "{planned}");
        assert!(
            planned.contains("compaction: enabled on overflow · 74.3k recovery target"),
            "{planned}"
        );
        let context = render_context_view(&app.status, host.runtime().policy());
        assert!(
            context.contains("example-model · ~2k / 123.9k input tokens · ~98% left"),
            "{context}"
        );
        assert!(context.contains("◆ tool schemas: ~500"), "{context}");
        assert!(
            context.contains("■ system instructions: ~0 (0.0%)"),
            "{context}"
        );
        assert!(context.contains("● history: ~1.5k"), "{context}");
        assert!(
            context.contains("counting: estimated · 2 segments"),
            "{context}"
        );

        handle_local_command(&mut app, &host, project.path(), CommandAction::Status).await;
        handle_local_command(&mut app, &host, project.path(), CommandAction::Context).await;
        handle_local_command(&mut app, &host, project.path(), CommandAction::Agent(None)).await;
        handle_local_command(&mut app, &host, project.path(), CommandAction::Diff(None)).await;

        git(project.path(), &["init"]);
        git(
            project.path(),
            &["config", "user.email", "smith@example.invalid"],
        );
        git(project.path(), &["config", "user.name", "Smith Test"]);
        std::fs::write(project.path().join("tracked.txt"), "before\n").expect("tracked");
        git(project.path(), &["add", "tracked.txt"]);
        git(project.path(), &["commit", "-m", "initial"]);
        std::fs::write(project.path().join("tracked.txt"), "after\n").expect("changed");
        handle_local_command(
            &mut app,
            &host,
            project.path(),
            CommandAction::Diff(Some("unstaged".to_owned())),
        )
        .await;

        for character in "/help".chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            None
        );

        let results = app
            .transcript
            .blocks()
            .iter()
            .filter_map(|block| match block {
                Block::LocalResult { title, state, .. } => Some((title.as_str(), *state)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            results,
            [
                ("status", LocalResultState::Info),
                ("context", LocalResultState::Info),
                ("agents", LocalResultState::Empty),
                ("diff", LocalResultState::Error),
                ("diff · unstaged", LocalResultState::Info),
                ("help", LocalResultState::Info),
            ]
        );
        assert!(app.overlay.is_none(), "local output must not open a viewer");
        assert_eq!(
            host.session().history().len(),
            history_before,
            "local output became provider conversation history"
        );
        let status_content = app
            .transcript
            .blocks()
            .iter()
            .find_map(|block| match block {
                Block::LocalResult { title, content, .. } if title == "status" => {
                    Some(content.as_str())
                }
                _ => None,
            })
            .expect("status output");
        assert!(
            status_content.contains("~98% input left"),
            "{status_content}"
        );
        assert!(
            status_content.contains("profile: dev · posture build · use main · rev"),
            "{status_content}"
        );
        assert!(status_content.contains("source"), "{status_content}");
        let context_content = app
            .transcript
            .blocks()
            .iter()
            .find_map(|block| match block {
                Block::LocalResult { title, content, .. } if title == "context" => {
                    Some(content.as_str())
                }
                _ => None,
            })
            .expect("context output");
        assert!(
            context_content.contains("Estimated usage by category"),
            "{context_content}"
        );

        let totals = std::collections::BTreeMap::from([
            (
                agent_runtime_core::manifest::SegmentKind::new("summary"),
                600,
            ),
            (
                agent_runtime_core::manifest::SegmentKind::new("user_input"),
                600,
            ),
        ]);
        app.status.record_context_plan(ContextPlanUpdate {
            fingerprint: "context-summary",
            cache_fingerprint: "cache-summary",
            input_tokens: 1_200,
            input_budget_tokens: 123_904,
            reserved_tokens: 4_096,
            segment_count: 2,
            totals: &totals,
            confidence: EstimationConfidence::Estimated,
        });
        let compacted = render_context_status(&app.status, host.runtime().policy());
        assert!(
            compacted.contains("compaction: applied · ~600 summary · 74.3k recovery target"),
            "{compacted}"
        );
        host.shutdown().await.expect("shutdown");
    }

    fn git(project: &std::path::Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .args(arguments)
            .current_dir(project)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("git");
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
