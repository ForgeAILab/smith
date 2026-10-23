//! Interactive terminal event loop and TUI action routing.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
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
    ChildPhase, ModuleNotification, SmithEvent as EventEnvelope, SmithEventKind as RuntimeEvent,
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
    /// Persist a validated compaction setting and rebuild the idle session.
    CompactionSetting(smith_client::commands::CompactAction),
    /// Admit a requested skill, then optionally dispatch its prepared task.
    ActivateSkill {
        selected: smith_runtime::skills::RequestedSkill,
        submission: Option<smith_tui::app::PreparedSubmission>,
    },
    /// A validated cache snapshot can join at an idle boundary.
    CatalogChanged(Arc<smith_config::catalog::CatalogSnapshot>),
    /// A classified root rejection requires the same-session search route to
    /// be recomposed before another prompt reaches the provider.
    HostedSearchUnavailable,
    /// A declared MCP server finished connecting, so the composed tool set is
    /// stale. The session is rebuilt around the same identity at the next idle
    /// boundary; the connections themselves survive it.
    CapabilitiesChanged,
    /// The session's usage, plus the price reference it was resolved against
    /// (if any) — carried out here because the exit report needs the identical
    /// reference `/status` read from during the session rather than a fresh
    /// catalog lookup of its own. The stopped work and shutdown diagnostic
    /// also travel out so they can be printed after terminal restoration.
    Quit(
        Box<smith_client::status::SessionUsage>,
        Option<smith_client::status::PriceReference>,
        Option<Box<smith_client::cache::CacheTurnSummary>>,
        Option<String>,
        Option<String>,
    ),
    Reconfigure(SelectionCommand),
    ModuleSwitch(smith_client::commands::ModuleSwitchRequest),
    PluginSwitch(smith_client::commands::PluginSwitchRequest),
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
    pub(super) mcp_trust_notices: Vec<String>,
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
    restore_transcript_with_activity(host, app, history, &[]);
}

pub(super) fn restore_transcript_with_activity(
    host: &HostSession,
    app: &mut App,
    history: &[agent_runtime_core::content::Message],
    hosted: &[smith_tui::transcript::RestoredHostedSearch],
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
        .replace_from_history_with_activity(history, &shortcuts, hosted);
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
    let mut signals = match crate::headless::signals::StopSignals::new() {
        Ok(signals) => signals,
        Err(error) => {
            let _ = crate::exit_deadline::shutdown_host(host).await;
            return Err(error).context("installing terminal signal handlers");
        }
    };
    let stop = crate::headless::signals::StopState::default();
    let run = async {
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
            &stop,
        )
        .await;
        let exiting = matches!(&run_result, Ok((InteractiveExit::Quit(..), _)) | Err(_));
        let shutdown_result = if exiting {
            crate::exit_deadline::shutdown_host(host).await
        } else {
            tokio::select! {
                result = host.shutdown() => result.map(|_| ()).map_err(|error| anyhow::anyhow!("{error}")),
                _ = stop.changed() => crate::exit_deadline::shutdown_host(host).await,
            }
        }
        .context("shutting the session down");

        if stop.interrupted()
            && let Ok((exit, app)) = &mut run_result
            && !matches!(exit, InteractiveExit::Quit(..))
        {
            app.request_exit();
            *exit = InteractiveExit::Quit(
                Box::new(app.session_usage()),
                app.status.price().cloned(),
                app.status.cache_summary().map(Box::new),
                app.stopped_work.clone(),
                None,
            );
        }

        if let Ok((InteractiveExit::Quit(usage, ..), _)) = &mut run_result {
            let snapshot = host.snapshot();
            usage.reconcile_synthetic_records(snapshot.usage.records());
            usage.reconcile_advisor_records(snapshot.usage.records());
            usage.reconcile_module_records(snapshot.usage.records());
            usage.reconcile_vision_records(snapshot.usage.records());
        }

        if let Ok((InteractiveExit::Quit(_, _, _, _, error), _)) = &mut run_result {
            *error = shutdown_result.err().map(|error| format!("{error:#}"));
        } else {
            shutdown_result?;
        }
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
    };
    tokio::select! {
        biased;
        _ = signals.watch(host, &stop) => unreachable!("signal watcher runs until exit"),
        result = run => result,
    }
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
pub(super) async fn subscribe_to_child(
    host: &HostSession,
    child: &ChildId,
    events: tokio::sync::mpsc::UnboundedSender<(ChildId, EventEnvelope)>,
) -> Result<(
    smith_runtime::host::ChildTranscriptSnapshot,
    Option<tokio::task::JoinHandle<()>>,
)> {
    let subscription = host.subscribe_child_transcript(child).await?;
    let task = subscription.events.map(|mut stream| {
        let child = child.clone();
        tokio::spawn(async move {
            while let Some(envelope) = stream.next().await {
                if events.send((child.clone(), envelope)).is_err() {
                    break;
                }
            }
        })
    });
    Ok((subscription.snapshot, task))
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
    module_notifications: tokio::sync::broadcast::Receiver<ModuleNotification>,
    keys: &'a mut crate::screen_runner::TerminalEvents,
    spinner: tokio::time::Interval,
    frame: tokio::time::Interval,
    local_tx: tokio::sync::mpsc::UnboundedSender<LocalOutcome>,
    local_rx: tokio::sync::mpsc::UnboundedReceiver<LocalOutcome>,
    pending_compaction: Option<futures_util::future::BoxFuture<'a, LocalResult>>,
    local_shell_approvals: LocalShellApprovals,
    shell_shortcuts: ShellShortcuts,
    child_tx: tokio::sync::mpsc::UnboundedSender<(ChildId, EventEnvelope)>,
    child_rx: tokio::sync::mpsc::UnboundedReceiver<(ChildId, EventEnvelope)>,
    child_subscriptions: BTreeMap<String, tokio::task::JoinHandle<()>>,
    hydrated_children: BTreeSet<String>,
    mcp_changes: Option<tokio::sync::watch::Receiver<u64>>,
    catalog_changes: tokio::sync::watch::Receiver<crate::model_sync::CatalogUpdate>,
    catalog_update_pending: bool,
    composed_remote_tools: usize,
    remote_tools_pending: bool,
    trusted_skill_pending: bool,
    hosted_search_rebuild_pending: bool,
    pending_module_switch: Option<smith_client::commands::ModuleSwitchRequest>,
    pending_plugin_switch: Option<smith_client::commands::PluginSwitchRequest>,
    last_change_turn: Option<u64>,
    interactions: interaction::InteractionSurface,
    dirty: bool,
    window_title: smith_tui::terminal_title::TerminalTitleState,
}

pub(super) async fn run_tui(
    terminal: &mut terminal::Terminal,
    app: App,
    inputs: TuiRunInputs<'_>,
    stop: &crate::headless::signals::StopState,
) -> Result<(InteractiveExit, App)> {
    run_tui_on(terminal.inner_mut(), TuiLoop::new(app, inputs), stop).await
}

async fn run_tui_on<B>(
    terminal: &mut ratatui::Terminal<B>,
    mut tui: TuiLoop<'_>,
    stop: &crate::headless::signals::StopState,
) -> Result<(InteractiveExit, App)>
where
    B: ratatui::backend::Backend,
    B::Error: Send + Sync + 'static,
{
    tui.restore_child_subscriptions().await;
    let exit = loop {
        if stop.interrupted() {
            break tui.on_quit();
        }
        if let Some(submission) = tui.app.take_ready_submission() {
            tui.on_submit(submission, SubmissionTarget::WholeTurn).await;
        }
        tokio::select! {
            // Stop signals and keyboard input must not be starved by a provider flood.
            biased;

            _ = stop.changed() => break tui.on_quit(),

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

            notice = tui.module_notifications.recv() => {
                match notice {
                    Ok(notice) => {
                        tui.app.transcript.push_notice(NoticeKind::Monitor, notice.display_text());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        tui.app.transcript.push_notice(
                            NoticeKind::Monitor,
                            format!("{count} module notifications were skipped by the display"),
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {}
                }
                tui.dirty = true;
            }

            envelope = tui.events.next() => {
                if let Some(exit) = tui.on_runtime_event(envelope).await {
                    break exit;
                }
            }

            child_event = tui.child_rx.recv() => {
                tui.on_child_event(child_event).await;
            }

            outcome = tui.local_rx.recv() => {
                tui.on_local_result(outcome);
            }

            report = async {
                match tui.pending_compaction.as_mut() {
                    Some(compaction) => compaction.await,
                    None => std::future::pending().await,
                }
            } => {
                tui.pending_compaction = None;
                tui.app.transcript.push_local(report);
                tui.dirty = true;
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
            changed = tui.catalog_changes.changed() => {
                if changed.is_ok() {
                    let update = tui.catalog_changes.borrow_and_update().clone();
                    tui.catalog_update_pending = crate::model_sync::differs(&update, &tui.host.runtime().policy().user_dir, tui.catalog);
                }
            }
            _ = tui.spinner.tick() => {
                tui.on_spinner();
            }

            _ = tui.frame.tick(), if tui.dirty || tui.remote_tools_pending || tui.trusted_skill_pending || tui.catalog_update_pending || tui.pending_module_switch.is_some() || tui.pending_plugin_switch.is_some() => {
                if let Some(exit) = tui.on_frame(terminal).await? {
                    break exit;
                }
            }
        }

        if let Some(exit) = tui.after_event() {
            break exit;
        }
    };
    for task in tui.child_subscriptions.values() {
        task.abort();
    }
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
        let module_notifications = host.client().module_notifications();
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
        // Marked changed so the first loop pass renders the current MCP
        // footer: a rebind starts from a fresh status, and a server that
        // failed before this point would otherwise vanish from it.
        let mcp_changes = mcp.as_ref().map(|context| {
            let mut changes = context.supervisor().subscribe();
            changes.mark_changed();
            changes
        });
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
            module_notifications,
            keys,
            spinner,
            frame,
            local_tx,
            local_rx,
            pending_compaction: None,
            local_shell_approvals,
            shell_shortcuts,
            child_tx,
            child_rx,
            child_subscriptions: BTreeMap::new(),
            hydrated_children: BTreeSet::new(),
            mcp_changes,
            catalog_changes: crate::model_sync::subscribe(),
            catalog_update_pending: false,
            composed_remote_tools,
            remote_tools_pending,
            trusted_skill_pending,
            hosted_search_rebuild_pending: false,
            pending_module_switch: None,
            pending_plugin_switch: None,
            last_change_turn,
            interactions,
            dirty,
            window_title,
        }
    }
}

/// Check a candidate before shutting down a usable session. No picker or turn
/// may cross this boundary, and a removed current model defers adoption.
async fn prepared_catalog_adoption(
    host: &HostSession,
    project: &std::path::Path,
    current: &smith_config::catalog::CatalogSnapshot,
) -> Option<Arc<smith_config::catalog::CatalogSnapshot>> {
    let policy = host.runtime().policy();
    let snapshot = crate::model_sync::cached_snapshot(&policy.user_dir).ok()?;
    let mut selection = crate::cli::Selection {
        project: Some(project.to_path_buf()),
        provider: Some(policy.provider_name.clone()),
        model: Some(policy.model.as_str().to_owned()),
        reasoning_enabled: policy.reasoning.selected_enabled,
        reasoning_effort: policy.reasoning.selected_effort.clone(),
        context_window: policy.context_window.clone(),
        ..Default::default()
    };
    if !policy.agent_profile_legacy {
        selection.profile = Some(policy.agent_profile.clone());
    }
    let prepared =
        crate::config_command::prepare_with_catalog(&selection, Some(snapshot.clone())).ok()?;
    let snapshot = prepared.catalog.clone();
    if !crate::model_sync::snapshots_differ(&snapshot, current) {
        return None;
    }
    let inventory = smith_config::inventory::local_inventory_with_catalog(
        &prepared.resolution,
        smith_runtime::factory::AVAILABLE_ADAPTER_KINDS,
        Some(&snapshot),
    )
    .ok()?;
    inventory.models.iter().find(|model| {
        model.provider == policy.provider_name
            && model.model == policy.model.as_str()
            && model.selectable
    })?;
    let request = crate::runtime_host::preflight_request(
        &prepared.resolution,
        project,
        smith_runtime::factory::HostSurface::Terminal,
        Some(snapshot.clone()),
    )
    .ok()?;
    let preflight = smith_runtime::factory::preflight(&request).await.ok()?;
    if preflight.endpoint != policy.endpoint || preflight.provider_kind != policy.provider_kind {
        return None;
    }
    Some(snapshot)
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

/// Keeps a confirmed plugin edit pending until the serving composition is idle.
pub(super) fn plugin_switch_exit(
    app: &App,
    pending: &mut Option<smith_client::commands::PluginSwitchRequest>,
) -> Option<InteractiveExit> {
    if app.is_busy() || app.has_pending_input() || app.has_pending_prompt() || app.overlay.is_some()
    {
        return None;
    }
    pending.take().map(InteractiveExit::PluginSwitch)
}

/// Bounds a background task's command for compact, single-line display.
///
/// The registry keeps the exact command for its own purposes; the footer and
/// exit summary only need enough to recognize which task is which.
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
        run_tui_on(
            &mut terminal,
            tui,
            &crate::headless::signals::StopState::default(),
        ),
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

/// Runs the real frame boundary with a staged catalog generation, without raw
/// mode or an input reader, so snapshot immutability can be regression-tested.
#[cfg(test)]
pub(super) async fn fold_scripted_catalog_frame(
    host: &HostSession,
    project: &std::path::Path,
    resources: &InteractiveResources,
    app: App,
) -> Result<(Option<InteractiveExit>, App)> {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 32))?;
    let mut keys = futures_util::stream::pending::<std::io::Result<TermEvent>>();
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
    tui.catalog_update_pending = true;
    let exit = tui.on_frame(&mut terminal).await?;
    Ok((exit, tui.app))
}
