//! Interactive terminal event loop and TUI action routing.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use agent_runtime_core::cancel::CancelReason;
use agent_runtime_core::ids::ChildId;
use agent_runtime_core::usage::{CounterKind, UsageRecord};
use anyhow::{Context, Result};
use crossterm::event::{Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use smith_client::NoticeKind;
use smith_client::agent_report::AgentSnapshot;
use smith_client::commands::{SelectionCommand, SessionControl};
use smith_client::local_result::LocalResult;
use smith_client::recovery_report::{RecoveryAction, RecoveryReport, RestoreReport};
use smith_config::inventory::SelectionInventory;
use smith_config::resolve::ResolvedAgent;
use smith_host::{
    ApprovalPrompt, ApprovalRequests, GitChanges, InteractionRequests, RotationPrompt,
    RotationRequests,
};
use smith_runtime::client::{
    ChildPhase, SmithEvent as EventEnvelope, SmithEventKind as RuntimeEvent,
};
use smith_runtime::factory::RuntimePolicy;
use smith_runtime::host::HostSession;
use smith_runtime::pool_state::ActiveAccounts;
use smith_runtime::rotation::SharedPool;
use smith_runtime::session::SessionListing;
use smith_tui::app::{Action, App, MouseOutcome, RunningTaskSummary, SubmissionTarget};
use smith_tui::theme::Theme;

use crate::local_command::{LocalOutcome, handle_local_command, tool_call_for_display};
use crate::resources::{abbreviate_home, account_entries, account_status, runtime_resources};
use crate::submission::{
    LocalShellApprovals, LocalShellIdentity, attach_from_clipboard, child_summary_projection,
    copy_selection_to_clipboard, dispatch_prepared_with_materialization, follow_up_agent,
    resume_agent, start_agent, start_local_shell, start_review,
};
use crate::{FRAME, SPINNER_TICK, interaction, local_command, terminal};

pub(super) enum InteractiveExit {
    /// A declared MCP server finished connecting, so the composed tool set is
    /// stale. The session is rebuilt around the same identity at the next idle
    /// boundary; the connections themselves survive it.
    CapabilitiesChanged,
    /// The session's usage, plus the price reference it was resolved against
    /// (if any) — carried out here because the exit report needs the identical
    /// reference `/status` read from during the session rather than a fresh
    /// catalog lookup of its own.
    Quit(
        Box<smith_client::status::SessionUsage>,
        Option<smith_client::status::PriceReference>,
        Option<Box<smith_client::cache::CacheTurnSummary>>,
    ),
    Reconfigure(SelectionCommand),
    ModuleSwitch(smith_client::commands::ModuleSwitchRequest),
    Connect(String),
    Disconnect(String),
}

pub(super) struct PresentationOptions {
    pub(super) no_color: bool,
    pub(super) no_motion: bool,
    pub(super) reasoning_notice: Option<String>,
    /// A host-side outcome to confirm once the session is on screen: first-run
    /// setup's summary, or the result of an `/advisor` change.
    pub(super) host_notice: Option<String>,
    pub(super) cache_miss_notices: bool,
}

pub(super) struct InteractiveResources {
    pub(super) inventory: SelectionInventory,
    pub(super) agents: ResolvedAgent,
    pub(super) sessions: Vec<SessionListing>,
    /// Live credential-pool state, when the provider declares a pool.
    pub(super) credential_pool: Option<SharedPool>,
    /// The catalog snapshot backing this session, so the active model's
    /// price can be resolved with the exact binding the runtime factory
    /// used. See `resolve_price`.
    pub(super) catalog: Arc<smith_config::catalog::CatalogSnapshot>,
    /// Declared MCP servers and their connections, when any are declared.
    pub(super) mcp: Option<Arc<crate::mcp::McpContext>>,
    /// The skills this composition indexed, and the files it refused.
    pub(super) skills: Arc<crate::skills::SkillContext>,
    /// File commands and their host-owned discovery roots.
    pub(super) commands: Arc<local_command::file_commands::CommandContext>,
    /// Capability patterns this session denied on top of its profile.
    pub(super) capability_denials: Vec<String>,
}

/// The runtime's out-of-band request streams, plus the accounts they rotate
/// between. Each stream is absent when the surface was started without that
/// capability.
pub(super) struct InteractiveRequests<'a> {
    /// Borrow process input so shutdown and composition cannot replace its reader.
    pub(super) keys: &'a mut crate::screen_runner::TerminalEvents,
    pub(super) approvals: Option<ApprovalRequests>,
    pub(super) interactions: Option<InteractionRequests>,
    pub(super) rotations: Option<RotationRequests>,
    pub(super) accounts: ActiveAccounts,
}

#[derive(Default)]
pub(super) struct ShellShortcuts {
    anchors: BTreeMap<u64, usize>,
}

impl ShellShortcuts {
    pub(super) fn dispatched(&mut self, host: &HostSession, echo: u64) {
        let anchor = host.session().with_history(|history| history.len());
        self.anchors.insert(echo, anchor);
    }

    pub(super) fn finish(
        &mut self,
        host: &HostSession,
        app: &mut App,
        echo: u64,
        call: Option<&str>,
        content: &str,
        is_error: bool,
    ) {
        let result = app
            .transcript
            .finish_shell_shortcut(echo, call, is_error, content);
        if let Some(anchor) = self.anchors.remove(&echo)
            && let Some(command) = app.transcript.shell_shortcut_command(echo)
        {
            host.record_shell_shortcut(anchor, call, command, is_error, result.as_deref());
        }
    }
}

pub(super) fn restore_transcript(
    host: &HostSession,
    app: &mut App,
    history: &[agent_runtime_core::content::Message],
) {
    let shortcuts = host
        .saved_shell_shortcuts()
        .into_iter()
        .map(|shortcut| smith_tui::transcript::RestoredShellShortcut {
            anchor: shortcut.anchor,
            call: shortcut.call,
            command: shortcut.command,
            is_error: shortcut.is_error,
            result: shortcut.result,
        })
        .collect::<Vec<_>>();
    app.transcript
        .replace_from_history_with_shell_shortcuts(history, &shortcuts);
    for (call, display) in host.tool_call_displays() {
        app.set_tool_display(call.as_str(), display);
    }
    for (call, text) in host.tool_result_texts() {
        app.set_tool_result_preview(call.as_str(), text);
    }
}

/// The application and the binding it last displayed, retained between hosts.
pub(super) struct InteractiveApp {
    pub(super) app: App,
    session: String,
    binding: AppBinding,
}

struct AppBinding {
    provider: String,
    model: String,
    profile: String,
    reasoning: smith_runtime::reasoning::ReasoningRuntimePolicy,
    context_window: Option<String>,
}

impl AppBinding {
    fn from_host(host: &HostSession) -> Self {
        let policy = host.runtime().policy();
        Self {
            provider: policy.provider_name.clone(),
            model: policy.model.as_str().to_owned(),
            profile: policy.agent_profile.clone(),
            reasoning: policy.reasoning.clone(),
            context_window: policy.context_window.clone(),
        }
    }
}

/// Selects a fresh seed or a same-session rebind without touching the terminal.
pub(super) async fn prepare_interactive_app(
    host: &HostSession,
    project: &std::path::Path,
    resources: &InteractiveResources,
    presentation: &PresentationOptions,
    previous: Option<InteractiveApp>,
) -> InteractiveApp {
    let session = host.session().id().as_str().to_owned();
    let binding = AppBinding::from_host(host);
    let app = match previous {
        Some(mut previous) if previous.session == session => {
            rebind_app(
                host,
                &mut previous.app,
                &previous.binding,
                project,
                resources,
                presentation,
            )
            .await;
            previous.app
        }
        previous => {
            let mut app = seed_app(host, project, resources, presentation).await;
            if let Some(mut previous) = previous {
                app.inherit_composer_history(&mut previous.app);
            }
            app
        }
    };
    InteractiveApp {
        app,
        session,
        binding,
    }
}

pub(super) async fn run_interactive(
    terminal: &mut terminal::Terminal,
    previous: Option<InteractiveApp>,
    host: &HostSession,
    requests: InteractiveRequests<'_>,
    project: &std::path::Path,
    resources: InteractiveResources,
    presentation: PresentationOptions,
) -> Result<(InteractiveExit, InteractiveApp)> {
    let InteractiveApp {
        app,
        session,
        binding,
    } = prepare_interactive_app(host, project, &resources, &presentation, previous).await;
    let InteractiveRequests {
        keys,
        approvals,
        interactions,
        rotations,
        accounts,
    } = requests;
    let InteractiveResources {
        agents,
        catalog,
        inventory,
        credential_pool,
        mcp,
        skills,
        commands,
        ..
    } = resources;
    let theme =
        crate::screen_runner::theme_from_flags(presentation.no_color, presentation.no_motion);
    let mut run_result = run_tui(
        terminal,
        app,
        TuiRunInputs {
            keys,
            host,
            project,
            approvals,
            interactions,
            rotations,
            accounts,
            credential_pool,
            agents: &agents,
            catalog: &catalog,
            inventory: &inventory,
            theme,
            mcp,
            skills,
            commands,
        },
    )
    .await;
    let shutdown_result = host
        .shutdown()
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("shutting the session down");

    if let Ok((InteractiveExit::Quit(usage, ..), _)) = &mut run_result {
        let snapshot = host.snapshot();
        usage.reconcile_synthetic_records(snapshot.usage.records());
        usage.reconcile_advisor_records(snapshot.usage.records());
    }

    shutdown_result?;
    run_result.map(|(exit, app)| {
        (
            exit,
            InteractiveApp {
                app,
                session,
                binding,
            },
        )
    })
}

/// Forwards one live child's own event stream into the client's single event
/// loop, tagged with the child it belongs to.
///
/// A child is a full runtime session, and this is how the client watches one:
/// the same events, the same fold, a different transcript. Presentation only —
/// the child's lifecycle, budget, and result delivery stay with the runtime's
/// coordinator, which is the authority for them.
///
/// A dormant child has no live stream, so this is a no-op for one recovered
/// from a durable record; its canonical history is what the inspector shows
/// instead. The forwarding task ends with the child's stream.
fn subscribe_to_child(
    host: &HostSession,
    child: &ChildId,
    events: tokio::sync::mpsc::UnboundedSender<(ChildId, EventEnvelope)>,
) {
    let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
    else {
        return;
    };
    let Some(mut stream) = coordinator.child_events(child) else {
        return;
    };
    let child = child.clone();
    tokio::spawn(async move {
        while let Some(envelope) = stream.next().await {
            if events
                .send((
                    child.clone(),
                    smith_runtime::client::SmithEvent::project_or_unknown(&envelope),
                ))
                .is_err()
            {
                break;
            }
        }
    });
}

pub(super) struct TuiRunInputs<'a> {
    keys: &'a mut crate::screen_runner::TerminalEvents,
    host: &'a HostSession,
    project: &'a std::path::Path,
    approvals: Option<ApprovalRequests>,
    interactions: Option<InteractionRequests>,
    rotations: Option<RotationRequests>,
    accounts: ActiveAccounts,
    credential_pool: Option<SharedPool>,
    agents: &'a ResolvedAgent,
    catalog: &'a smith_config::catalog::CatalogSnapshot,
    inventory: &'a SelectionInventory,
    theme: Theme,
    mcp: Option<Arc<crate::mcp::McpContext>>,
    skills: Arc<crate::skills::SkillContext>,
    commands: Arc<local_command::file_commands::CommandContext>,
}

struct TuiLoop<'a> {
    app: App,
    host: &'a HostSession,
    project: &'a std::path::Path,
    approvals: Option<ApprovalRequests>,
    rotations: Option<RotationRequests>,
    accounts: ActiveAccounts,
    credential_pool: Option<SharedPool>,
    agents: &'a ResolvedAgent,
    catalog: &'a smith_config::catalog::CatalogSnapshot,
    inventory: &'a SelectionInventory,
    theme: Theme,
    mcp: Option<Arc<crate::mcp::McpContext>>,
    skills: Arc<crate::skills::SkillContext>,
    commands: Arc<local_command::file_commands::CommandContext>,
    session: &'a smith_runtime::SessionHandle,
    events: smith_runtime::client::SmithEventStream,
    keys: &'a mut crate::screen_runner::TerminalEvents,
    spinner: tokio::time::Interval,
    frame: tokio::time::Interval,
    local_tx: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
    local_rx: tokio::sync::mpsc::UnboundedReceiver<LocalOutcome>,
    local_shell_approvals: LocalShellApprovals,
    shell_shortcuts: ShellShortcuts,
    child_tx: tokio::sync::mpsc::UnboundedSender<(ChildId, EventEnvelope)>,
    child_rx: tokio::sync::mpsc::UnboundedReceiver<(ChildId, EventEnvelope)>,
    mcp_changes: Option<tokio::sync::watch::Receiver<u64>>,
    composed_remote_tools: usize,
    remote_tools_pending: bool,
    trusted_skill_pending: bool,
    pending_module_switch: Option<smith_client::commands::ModuleSwitchRequest>,
    last_change_turn: Option<u64>,
    interactions: interaction::InteractionSurface,
    dirty: bool,
    window_title: smith_tui::terminal_title::TerminalTitleState,
}

pub(super) async fn run_tui(
    terminal: &mut terminal::Terminal,
    app: App,
    inputs: TuiRunInputs<'_>,
) -> Result<(InteractiveExit, App)> {
    run_tui_on(terminal.inner_mut(), TuiLoop::new(app, inputs)).await
}

async fn run_tui_on<B>(
    terminal: &mut ratatui::Terminal<B>,
    mut tui: TuiLoop<'_>,
) -> Result<(InteractiveExit, App)>
where
    B: ratatui::backend::Backend,
    B::Error: Send + Sync + 'static,
{
    let exit = loop {
        tokio::select! {
            // Keyboard first: a provider flood must not starve cancellation.
            biased;

            Some(key) = tui.keys.next() => {
                if let Some(exit) = tui.on_terminal_event(terminal, key).await? {
                    break exit;
                }
            }

            prompt = next_approval(&mut tui.approvals) => {
                tui.on_approval(prompt);
            }

            offer = next_rotation(&mut tui.rotations) => {
                tui.on_rotation(offer);
            }

            notice = tui.interactions.next_notice() => {
                tui.on_interaction_notice(notice);
            }

            envelope = tui.events.next() => {
                if let Some(exit) = tui.on_runtime_event(envelope).await {
                    break exit;
                }
            }

            child_event = tui.child_rx.recv() => {
                tui.on_child_event(child_event);
            }

            outcome = tui.local_rx.recv() => {
                tui.on_local_result(outcome);
            }

            () = async {
                match &mut tui.mcp_changes {
                    Some(receiver) => {
                        let _ = receiver.changed().await;
                    }
                    // No declared server: this arm never completes, and the
                    // loop behaves exactly as it did before MCP existed.
                    None => std::future::pending().await,
                }
            } => {
                tui.on_mcp_change();
            }
            _ = tui.spinner.tick() => {
                tui.on_spinner();
            }

            _ = tui.frame.tick(), if tui.dirty || tui.remote_tools_pending || tui.trusted_skill_pending || tui.pending_module_switch.is_some() => {
                if let Some(exit) = tui.on_frame(terminal).await? {
                    break exit;
                }
            }
        }

        if let Some(exit) = tui.after_event() {
            break exit;
        }
    };
    // A normal exit clears the title exactly once, ahead of a new host's
    // tracker or the caller's terminal restore. An I/O error through `?` skips
    // this and leaves the last title standing until the shell's own prompt hook
    // reasserts its title on the next prompt -- that same hook is why
    // restoring a remembered pre-session title is deliberately not attempted.
    let _ = tui.window_title.clear();
    Ok((exit, tui.app))
}

impl<'a> TuiLoop<'a> {
    fn new(app: App, inputs: TuiRunInputs<'a>) -> Self {
        let TuiRunInputs {
            keys,
            host,
            project,
            approvals,
            interactions,
            rotations,
            accounts,
            credential_pool,
            agents,
            catalog,
            inventory,
            theme,
            mcp,
            skills,
            commands,
        } = inputs;
        let session = host.session();
        let events = host.client().events();
        let spinner = tokio::time::interval(SPINNER_TICK);
        let frame = tokio::time::interval(FRAME);
        let (local_tx, local_rx) = tokio::sync::mpsc::unbounded_channel();
        let local_shell_approvals = LocalShellApprovals::default();
        let shell_shortcuts = ShellShortcuts::default();
        // One forwarding task per live child funnels every child's own stream into
        // this loop, so a child's events are folded by the same single-threaded
        // reducer the root's are and can never interleave mid-fold.
        let (child_tx, child_rx) =
            tokio::sync::mpsc::unbounded_channel::<(ChildId, EventEnvelope)>();
        let mcp_changes = mcp.as_ref().map(|context| context.supervisor().subscribe());
        // What the runtime was composed with. A rebuild is worth its cost only
        // when a server has actually contributed something new since.
        let composed_remote_tools = mcp
            .as_ref()
            .map_or(0, |context| context.supervisor().tools().len());
        let remote_tools_pending = false;
        // A newly trusted project skill is only in the trust file until the
        // catalog is resolved again, and the catalog is resolved at composition.
        let trusted_skill_pending = false;
        let last_change_turn = host.changes().latest().map(|set| set.turn);
        let interactions = interaction::InteractionSurface::new(
            interactions,
            host.restored_interaction()
                .map(|restored| restored.request_id().as_str().to_owned()),
        );
        let dirty = true;
        // The terminal window title follows the same state the header does, but
        // is written outside the frame: it is one OSC sequence per *change*, not
        // per draw. Guarded and deduped inside the tracker, so a non-terminal
        // stdout (or an unchanged title) costs nothing.
        let mut window_title = smith_tui::terminal_title::TerminalTitleState::new();
        // One failed write costs a stale title, not the session.
        let _ = window_title.refresh(&app.status);
        Self {
            app,
            host,
            project,
            approvals,
            rotations,
            accounts,
            credential_pool,
            agents,
            catalog,
            inventory,
            theme,
            mcp,
            skills,
            commands,
            session,
            events,
            keys,
            spinner,
            frame,
            local_tx,
            local_rx,
            local_shell_approvals,
            shell_shortcuts,
            child_tx,
            child_rx,
            mcp_changes,
            composed_remote_tools,
            remote_tools_pending,
            trusted_skill_pending,
            pending_module_switch: None,
            last_change_turn,
            interactions,
            dirty,
            window_title,
        }
    }
}

/// Defence behind command validation: no host-owned work may cross a rebuild.
pub(super) fn reconfigure_exit(app: &mut App, command: SessionControl) -> Option<InteractiveExit> {
    let name = match &command {
        SessionControl::Reconfigure(selection) => match selection {
            SelectionCommand::NewSession => "new",
            SelectionCommand::Resume(_) => "resume",
            SelectionCommand::Profile(_) => "profile",
            SelectionCommand::Model { .. } => "model",
            SelectionCommand::Agent(_) => "agent",
            SelectionCommand::Think(_) => "think",
            SelectionCommand::Effort(_) => "effort",
            SelectionCommand::Advisor(_) => "advisor",
            SelectionCommand::CapabilityDeny(_) | SelectionCommand::CapabilityAllow(_) => {
                "capabilities"
            }
            SelectionCommand::ContextWindow(_) => "context",
        },
        SessionControl::Connect(_) => "connect",
        SessionControl::Disconnect(_) => "disconnect",
        SessionControl::Account(_) => return None,
    };
    if app.is_busy() || app.has_pending_input() || app.has_pending_prompt() {
        app.push_notice(
            NoticeKind::CommandRefused,
            format!("/{name} requires an idle turn; draft preserved"),
        );
        return None;
    }
    match command {
        SessionControl::Reconfigure(selection) => Some(InteractiveExit::Reconfigure(selection)),
        SessionControl::Connect(provider) => Some(InteractiveExit::Connect(provider)),
        SessionControl::Disconnect(provider) => Some(InteractiveExit::Disconnect(provider)),
        SessionControl::Account(_) => None,
    }
}

/// Keeps a confirmed module edit pending until the serving composition is idle.
pub(super) fn module_switch_exit(
    app: &App,
    pending: &mut Option<smith_client::commands::ModuleSwitchRequest>,
) -> Option<InteractiveExit> {
    if app.is_busy() || app.has_pending_input() || app.has_pending_prompt() || app.overlay.is_some()
    {
        return None;
    }
    pending.take().map(InteractiveExit::ModuleSwitch)
}

/// Bounds a background task's command for compact, single-line display.
///
/// The registry keeps the exact command for its own purposes; the footer and
/// exit-confirm modal only need enough to recognize which task is which.
fn compact_command_hint(command: &str) -> String {
    const MAX_CHARS: usize = 60;
    let collapsed = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_CHARS {
        format!("{}…", collapsed.chars().take(MAX_CHARS).collect::<String>())
    } else {
        collapsed
    }
}

pub(super) async fn next_approval(
    approvals: &mut Option<ApprovalRequests>,
) -> Option<ApprovalPrompt> {
    match approvals {
        Some(approvals) => approvals.recv().await,
        None => std::future::pending().await,
    }
}

/// Waits for the next rotation offer, or never when the provider has no pool.
pub(super) async fn next_rotation(
    rotations: &mut Option<RotationRequests>,
) -> Option<RotationPrompt> {
    match rotations {
        Some(rotations) => rotations.recv().await,
        None => std::future::pending().await,
    }
}

/// Persists the active account when it differs from what is remembered.
///
/// Covers rotations the runtime performed on its own; a manual switch already
/// persists at the point of the switch. A failed write costs stickiness, not
/// the session, so it is swallowed rather than escalated.
pub(super) async fn remember_active_account(
    credential_pool: Option<&SharedPool>,
    accounts: &mut ActiveAccounts,
) {
    let Some(pool) = credential_pool else {
        return;
    };
    let Some((provider, active)) = pool.read(|pool| {
        pool.active()
            .map(|member| (pool.provider().to_owned(), member.reference.clone()))
    }) else {
        return;
    };
    if accounts.remember(&provider, &active) {
        let _ = accounts.save().await;
    }
}

/// Applies an account switch to live pool state and remembers it.
///
/// Returns the transcript notice, or `None` when nothing changed. No runtime
/// is rebuilt: the credential source reads the active member on the next
/// acquisition, so the switch takes effect on the very next attempt.
pub(super) async fn switch_account(
    credential_pool: Option<&SharedPool>,
    accounts: &mut ActiveAccounts,
    position: usize,
) -> Option<String> {
    let pool = credential_pool?;
    let outgoing = pool.read(|pool| pool.active().map(|member| member.reference.clone()))?;
    if !pool.write(|pool| pool.set_active(position)) {
        return None;
    }
    let (provider, incoming) = pool.read(|pool| {
        (
            pool.provider().to_owned(),
            pool.active().map(|member| member.reference.clone()),
        )
    });
    let incoming = incoming?;
    if accounts.remember(&provider, &incoming) {
        // A failed write costs stickiness, not the switch: the session is
        // already using the new account either way, so this is reported rather
        // than escalated.
        if let Err(error) = accounts.save().await {
            let _ = error;
        }
    }
    Some(smith_tui::accounts::switch_notice(
        &outgoing, &incoming, true,
    ))
}

/// Exercises the production select loop without raw mode; an idle recomposition
/// ends the run after queued input so tests can inspect an unsubmitted draft.
#[cfg(test)]
pub(super) async fn run_scripted_tui(
    host: &HostSession,
    project: &std::path::Path,
    resources: &InteractiveResources,
    app: App,
    keys: &mut crate::screen_runner::TerminalEvents,
    recompose: bool,
) -> Result<(InteractiveExit, App)> {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 32))?;
    let mut tui = TuiLoop::new(
        app,
        TuiRunInputs {
            keys,
            host,
            project,
            approvals: None,
            interactions: None,
            rotations: None,
            accounts: ActiveAccounts::ephemeral(),
            credential_pool: None,
            agents: &resources.agents,
            catalog: &resources.catalog,
            inventory: &resources.inventory,
            theme: Theme::new().without_color().without_motion(),
            mcp: None,
            skills: resources.skills.clone(),
            commands: resources.commands.clone(),
        },
    );
    tui.remote_tools_pending = recompose;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        run_tui_on(&mut terminal, tui),
    )
    .await
    .context("scripted loop did not reach an exit")?
}

/// Folds a host event batch through the production handler without terminal I/O.
#[cfg(all(test, feature = "module-budget-notice"))]
pub(super) async fn fold_scripted_runtime_event(
    host: &HostSession,
    project: &std::path::Path,
    resources: &InteractiveResources,
    app: App,
    event: EventEnvelope,
) -> App {
    let mut keys = futures_util::stream::pending();
    let mut tui = TuiLoop::new(
        app,
        TuiRunInputs {
            keys: &mut keys,
            host,
            project,
            approvals: None,
            interactions: None,
            rotations: None,
            accounts: ActiveAccounts::ephemeral(),
            credential_pool: None,
            agents: &resources.agents,
            catalog: &resources.catalog,
            inventory: &resources.inventory,
            theme: Theme::new().without_color().without_motion(),
            mcp: None,
            skills: resources.skills.clone(),
            commands: resources.commands.clone(),
        },
    );
    assert!(tui.on_runtime_event(Some(event)).await.is_none());
    tui.app
}

fn change_notice(set: &smith_tools::TurnChangeSet) -> Option<String> {
    // A mutating capability alone does not prove that the turn changed files.
    if set.undone || !set.exact_mutations().any(|edit| edit.before != edit.after) {
        return None;
    }
    let attribution = if set.is_fully_attributable() {
        "undo available"
    } else {
        "contains ambiguous changes; /undo covers Smith's own edits, /diff shows the rest"
    };
    Some(format!("Smith turn {} · {attribution}", set.turn))
}

mod actions;
mod app_state;
mod events;
mod usage;

use app_state::{rebind_app, seed_app};
use usage::{last_session_usage, resolve_child_usage_binding, restore_usage_with_bindings};

#[cfg(test)]
use usage::restore_usage_records;

pub(super) use usage::resolve_price;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod transcript_notice_tests;
