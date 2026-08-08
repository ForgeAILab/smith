//! Interactive terminal event loop and TUI action routing.

use std::collections::VecDeque;

use smith_runtime::background_tasks::BackgroundTaskRegistry;
use smith_tui::app::RunningTaskSummary;

use super::*;

pub(super) enum InteractiveExit {
    /// A declared MCP server finished connecting, so the composed tool set is
    /// stale. The session is rebuilt around the same identity at the next idle
    /// boundary; the connections themselves survive it.
    CapabilitiesChanged,
    /// The session's usage, plus the price reference it was resolved against
    /// (if any) — carried out here because `Status` does not survive past
    /// `run_tui`'s return, and the exit report needs the identical
    /// reference `/status` read from during the session rather than a fresh
    /// catalog lookup of its own.
    Quit(
        smith_tui::status::SessionUsage,
        Option<smith_tui::status::PriceReference>,
    ),
    Reconfigure(PaletteCommand),
}

pub(super) struct PresentationOptions {
    pub(super) no_color: bool,
    pub(super) no_motion: bool,
    pub(super) reasoning_notice: Option<String>,
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
}

/// The runtime's out-of-band request streams, plus the accounts they rotate
/// between. Each stream is absent when the surface was started without that
/// capability.
pub(super) struct InteractiveRequests {
    pub(super) approvals: Option<ApprovalRequests>,
    pub(super) interactions: Option<InteractionRequests>,
    pub(super) rotations: Option<RotationRequests>,
    pub(super) accounts: ActiveAccounts,
}

pub(super) async fn run_interactive(
    host: &HostSession,
    requests: InteractiveRequests,
    project: &std::path::Path,
    resources: InteractiveResources,
    presentation: PresentationOptions,
) -> Result<InteractiveExit> {
    let InteractiveRequests {
        approvals,
        interactions,
        rotations,
        accounts,
    } = requests;
    let InteractiveResources {
        inventory,
        agents,
        sessions,
        credential_pool,
        catalog,
        mcp,
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
    app.status.set_price(resolve_price(policy, &catalog));
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
        credential_pool.as_ref(),
    ));
    app.status.account = account_status(credential_pool.as_ref());
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
            interruption.tasks.len(),
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
        // `switch_model` just cleared the price along with the cache
        // evidence, but this branch's target is the same active `policy`
        // already resolved above — the prior *snapshot's* model, not this
        // session's — so the price is re-resolved rather than left cleared.
        app.status.set_price(resolve_price(policy, &catalog));
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
            rotations,
            accounts,
            credential_pool,
            agents: &agents,
            theme,
            mcp,
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

/// Resolves the active model's catalog price, using **exactly** the binding
/// the runtime factory itself resolves models against — this mirrors
/// `crates/smith-runtime/src/factory.rs`'s `prepare_factory_inputs` catalog
/// lookup line for line, rather than inventing a second resolution that
/// could disagree with it and price the wrong model.
///
/// Returns `None` when the catalog carries no price entry for this binding.
/// That is never treated as "assume some other price" anywhere downstream —
/// `usage-accounting`'s "Labelled cost calculation" forbids substituting a
/// price from another model, provider, or a hard-coded default, and a
/// `None` here is exactly how that absence is represented.
fn resolve_price(
    policy: &RuntimePolicy,
    catalog: &smith_config::catalog::CatalogSnapshot,
) -> Option<smith_tui::status::PriceReference> {
    let cost = smith_config::catalog::catalog_provider_for(
        &policy.provider_kind,
        policy.endpoint.as_deref(),
    )
    .and_then(|provider| catalog.provider(provider))
    .and_then(|provider| provider.models.get(policy.model.as_str()))
    .and_then(|model| model.cost.as_ref())?;
    Some(smith_tui::status::PriceReference {
        provider: policy.provider_name.clone(),
        model: policy.model.as_str().to_owned(),
        table: smith_tui::status::PriceTable {
            input: cost.input,
            output: cost.output,
            cache_read: cost.cache_read,
            cache_write: cost.cache_write,
        },
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
            if events.send((child.clone(), envelope)).is_err() {
                break;
            }
        }
    });
}

pub(super) struct TuiRunInputs<'a> {
    host: &'a HostSession,
    project: &'a std::path::Path,
    approvals: Option<ApprovalRequests>,
    interactions: Option<InteractionRequests>,
    rotations: Option<RotationRequests>,
    accounts: ActiveAccounts,
    credential_pool: Option<SharedPool>,
    agents: &'a ResolvedAgent,
    theme: Theme,
    mcp: Option<Arc<crate::mcp::McpContext>>,
}

pub(super) async fn run_tui(
    terminal: &mut terminal::Terminal,
    mut app: App,
    inputs: TuiRunInputs<'_>,
) -> Result<InteractiveExit> {
    let TuiRunInputs {
        host,
        project,
        mut approvals,
        interactions,
        mut rotations,
        mut accounts,
        credential_pool,
        agents,
        theme,
        mcp,
    } = inputs;
    let session = host.session();
    let mut events = session.subscribe();
    let mut keys = EventStream::new();
    let mut spinner = tokio::time::interval(SPINNER_TICK);
    let mut frame = tokio::time::interval(FRAME);
    let (local_tx, mut local_rx) = tokio::sync::mpsc::unbounded_channel();
    // One forwarding task per live child funnels every child's own stream into
    // this loop, so a child's events are folded by the same single-threaded
    // reducer the root's are and can never interleave mid-fold.
    let (child_tx, mut child_rx) =
        tokio::sync::mpsc::unbounded_channel::<(ChildId, EventEnvelope)>();
    let mut mcp_changes = mcp.as_ref().map(|context| context.supervisor().subscribe());
    // What the runtime was composed with. A rebuild is worth its cost only
    // when a server has actually contributed something new since.
    let composed_remote_tools = mcp
        .as_ref()
        .map_or(0, |context| context.supervisor().tools().len());
    let mut remote_tools_pending = false;
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
                            Some(Action::Submit { submission, target }) => {
                                host.set_goal_continuation_enabled(false);
                                dispatch_prepared_with_materialization(
                                    &mut app,
                                    session,
                                    project,
                                    submission,
                                    target,
                                )
                                .await;
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
                            Some(Action::BackgroundShell) => {
                                // Kept distinct from `Action::Interrupt`: this
                                // never kills the group, it only asks the
                                // registry to adopt whatever foreground call
                                // is currently running, if any.
                                if BackgroundTaskRegistry::global()
                                    .trigger_manual_backgrounding(session.id())
                                {
                                    app.transcript.push_notice(
                                        "background",
                                        "command moved to the background",
                                    );
                                } else {
                                    app.transcript.push_notice(
                                        "background",
                                        "no foreground shell command is running",
                                    );
                                }
                            }
                            Some(Action::Quit) => break InteractiveExit::Quit(app.session_usage(), app.status.price().cloned()),
                            // An account switch is live pool state, so it is
                            // applied here rather than by tearing the session
                            // down and rebuilding it around a new selection.
                            Some(Action::Reconfigure(PaletteCommand::Account(position))) => {
                                match switch_account(
                                    credential_pool.as_ref(),
                                    &mut accounts,
                                    position,
                                )
                                .await
                                {
                                    Some(notice) => {
                                        app.transcript.push_notice("account", notice);
                                        app.set_accounts(account_entries(
                                            credential_pool.as_ref(),
                                        ));
                                        app.status.account =
                                            account_status(credential_pool.as_ref());
                                    }
                                    None => app
                                        .transcript
                                        .push_notice("account", "already using that account"),
                                }
                            }
                            Some(Action::Reconfigure(command)) => {
                                break InteractiveExit::Reconfigure(command);
                            }
                            Some(Action::Command(command)) => {
                                handle_local_command(
                                    &mut app,
                                    host,
                                    project,
                                    mcp.as_deref(),
                                    command,
                                )
                                .await;
                            }
                            Some(Action::TrustMcpServer { server }) => {
                                match mcp.as_ref().map(|context| context.trust(&server)) {
                                    Some(Ok(notice)) => {
                                        app.show_local_result("mcp", notice);
                                    }
                                    Some(Err(error)) => app.show_local_error("mcp", error),
                                    None => app.show_local_error(
                                        "mcp",
                                        "no MCP servers are declared",
                                    ),
                                }
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
                    TermEvent::Mouse(mouse) => match app.on_mouse(mouse) {
                        MouseOutcome::Ignored => {}
                        MouseOutcome::Redraw => dirty = true,
                        MouseOutcome::CopySelection => {
                            // Drawn here rather than deferred to the frame
                            // tick: the selected text exists only in the frame
                            // buffer, and a runtime event arriving in between
                            // would clear the selection before it could be
                            // read — a release that silently copied nothing.
                            let mut selected = None;
                            terminal.draw(|frame| {
                                smith_tui::draw_synced(frame, &mut app, theme);
                                selected = smith_tui::selected_text(frame, &app);
                            })?;
                            dirty = false;
                            // A drag across blank space yields nothing, and
                            // clobbering the clipboard with an empty string
                            // would lose whatever the user had there.
                            if let Some(text) = selected {
                                copy_selection_to_clipboard(&mut app, &text);
                            }
                        }
                    },
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

            offer = next_rotation(&mut rotations) => {
                match offer {
                    Some(prompt) => {
                        app.present_rotation(prompt);
                        dirty = true;
                    }
                    None => rotations = None,
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
                        // The live queue is normally this one envelope. When
                        // applying it reveals a broadcast lag gap, the missing
                        // range is replayed out of the canonical journal ahead
                        // of it, so control events (turn terminals, queued-
                        // input releases) still fold in order instead of
                        // wedging the UI on a state change it never saw.
                        let mut pending = VecDeque::from([(envelope, false)]);
                        while let Some((envelope, recovered)) = pending.pop_front() {
                            let tool_call = tool_call_for_display(&envelope.payload);
                            let completed_tool = matches!(
                                envelope.payload,
                                RuntimeEvent::ToolCallCompleted { .. }
                            );
                            let turn_completed = matches!(
                                envelope.payload,
                                RuntimeEvent::TurnCompleted { .. }
                            );
                            if recovered {
                                app.apply_recovered(&envelope);
                            } else {
                                app.apply(&envelope);
                                if let Some(gap) = app.take_stream_gap() {
                                    // The envelope was parked, not applied.
                                    // Queue the journal's copy of the missing
                                    // range first, then retry the parked
                                    // envelope on the honest replay path.
                                    match host
                                        .journal_events_between(
                                            gap.first_missing,
                                            gap.last_missing,
                                        )
                                        .await
                                    {
                                        Ok(events) => {
                                            if !events.is_empty() {
                                                // Accumulated, not shown yet: a
                                                // broadcast overrun produces a
                                                // run of these gaps back to
                                                // back, and `App` collapses the
                                                // whole run into one line once
                                                // it sees a contiguous event
                                                // again.
                                                app.note_recovered_events(events.len());
                                            }
                                            pending.push_front((gap.deferred, true));
                                            for event in events.into_iter().rev() {
                                                pending.push_front((event, true));
                                            }
                                        }
                                        Err(error) => {
                                            app.transcript.push_error(format!(
                                                "replaying skipped events {}–{} from the \
                                                 session journal failed: {error}",
                                                gap.first_missing, gap.last_missing
                                            ));
                                            pending.push_front((gap.deferred, true));
                                        }
                                    }
                                    continue;
                                }
                            }
                            if turn_completed
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
                            if let Some(submission) = app.take_ready_submission() {
                                dispatch_prepared_with_materialization(
                                    &mut app,
                                    session,
                                    project,
                                    submission,
                                    SubmissionTarget::WholeTurn,
                                )
                                .await;
                            }
                            if let Some(call) = tool_call {
                                if let Some(display) = host.tool_call_display(&call) {
                                    // Only at request time: the same call id
                                    // is resolved again at completion, and
                                    // this queue must see a spawn exactly
                                    // once or `ChildSpawned` would enrich the
                                    // wrong row. This is the root's own
                                    // event stream — the child-events branch
                                    // below never reaches this call, which is
                                    // how the pending-spawn queue stays
                                    // root-only.
                                    if !completed_tool {
                                        app.note_pending_spawn(call.as_str(), &display);
                                    }
                                    app.set_tool_display(call.as_str(), display);
                                }
                                if completed_tool
                                    && let Some(text) = host.tool_result_text(&call)
                                {
                                    app.set_tool_result_preview(call.as_str(), text);
                                }
                            }
                            // A child is a full runtime session. The parent
                            // stream says one started; its own stream says
                            // what it is doing, and that is what the
                            // inspector draws.
                            //
                            // A resume is a second start: the durable record
                            // is bound to a new execution with a new stream,
                            // and the task watching the old one ended with it.
                            match &envelope.payload {
                                RuntimeEvent::ChildSpawned { child, .. }
                                | RuntimeEvent::ChildProgress {
                                    child,
                                    phase: ChildPhase::ResumeStarted { .. },
                                } => subscribe_to_child(host, child, child_tx.clone()),
                                _ => {}
                            }
                        }
                        dirty = true;
                    }
                    None => break InteractiveExit::Quit(app.session_usage(), app.status.price().cloned()),
                }
            }

            child_event = child_rx.recv() => {
                if let Some((child, envelope)) = child_event {
                    app.apply_child(child.as_str(), &envelope);
                    // The child's events withhold argument values and result
                    // text exactly as the root's do. Both are resolved the
                    // same way: by call id, against that agent's canonical
                    // history, redacted by the host.
                    if let Some(call) = tool_call_for_display(&envelope.payload) {
                        if let Some(display) = host.child_tool_call_display(&child, &call) {
                            app.set_child_tool_display(child.as_str(), call.as_str(), display);
                        }
                        if matches!(envelope.payload, RuntimeEvent::ToolCallCompleted { .. })
                            && let Some(text) = host.child_tool_result_text(&child, &call)
                        {
                            app.set_child_tool_result_preview(
                                child.as_str(),
                                call.as_str(),
                                text,
                            );
                        }
                    }
                    dirty = true;
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
                    }
                    dirty = true;
                }
            }

            () = async {
                match &mut mcp_changes {
                    Some(receiver) => {
                        let _ = receiver.changed().await;
                    }
                    // No declared server: this arm never completes, and the
                    // loop behaves exactly as it did before MCP existed.
                    None => std::future::pending().await,
                }
            } => {
                if let Some(context) = &mcp {
                    let supervisor = context.supervisor();
                    let reports = supervisor.reports();
                    app.status.mcp = smith_tui::McpStatus {
                        connecting: reports
                            .iter()
                            .filter(|report| !report.state.is_settled())
                            .count(),
                        failed: reports
                            .iter()
                            .filter(|report| matches!(
                                report.state,
                                smith_runtime::mcp::McpState::Failed { .. }
                            ))
                            .count(),
                    };
                    remote_tools_pending = supervisor.tools().len() != composed_remote_tools;
                    dirty = true;
                }
            }
            _ = spinner.tick() => {
                let exit_hint_expired = app.expire_ctrl_c_exit_hint();
                // Rows retire while the session is idle — that is the whole
                // point of them retiring — so this cannot ride on `tick`,
                // which only advances while there is work to animate.
                let rows_retired = app.expire_child_rows();
                let busy = app.is_busy();
                if busy {
                    app.tick();
                }
                if exit_hint_expired
                    || rows_retired
                    || (busy && (theme.uses_motion() || app.tick.is_multiple_of(10)))
                {
                    dirty = true;
                }
            }

            _ = frame.tick(), if dirty || remote_tools_pending => {
                // A newly connected server's tools join at the next idle
                // boundary, never mid-turn: swapping the tool set underneath a
                // running turn is what the epoch rules exist to prevent.
                if remote_tools_pending
                    && !app.is_busy()
                    && !app.has_pending_input()
                    && app.overlay.is_none()
                {
                    break InteractiveExit::CapabilitiesChanged;
                }
                // Re-read on the way to the screen rather than at each site
                // that could change it: the pool also moves on its own — a
                // rotation the runtime performed, a snapshot that arrived
                // mid-turn — and a footer refreshed only on manual switches
                // would keep naming an account the session had already left.
                if credential_pool.is_some() {
                    app.status.account = account_status(credential_pool.as_ref());
                    app.set_accounts(account_entries(credential_pool.as_ref()));
                    // Rotation happens inside the runtime, which cannot reach
                    // user-scope state, so the account it moved to is
                    // remembered here. `remember` reports whether anything
                    // changed, so this writes on a switch and not on a frame.
                    remember_active_account(credential_pool.as_ref(), &mut accounts).await;
                }
                // Same cadence as the account refresh above: the TUI never
                // reaches the registry itself, so this poll-on-redraw is the
                // only path by which a task's start or terminal state
                // reaches operational status and the exit-confirm gate.
                app.set_running_tasks(
                    BackgroundTaskRegistry::global()
                        .running_tasks(session.id())
                        .into_iter()
                        .map(|task| RunningTaskSummary {
                            task_id: task.task_id,
                            command_hint: compact_command_hint(&task.command),
                        })
                        .collect(),
                );
                // Same reason, for the open child inspector and the
                // delegated-work panel: turns, tokens, and lifecycle live in
                // the coordinator, which the TUI cannot reach. A child
                // selected by arrow key gets the same card as one opened by
                // `/agent <id>`, and it stays current while the child works
                // — and every visible child's panel row gets the
                // coordinator's own turn/token counts on the same cadence,
                // per `usage-accounting`'s "Counts come from the
                // coordinator": Smith computes none of this itself.
                if let Some(coordinator) = host
                    .runtime()
                    .delegation()
                    .and_then(|delegation| delegation.coordinator())
                {
                    let statuses = coordinator.list();
                    if let Some(inspected) = app.inspected_child.clone() {
                        let card = statuses
                            .iter()
                            .find(|status| status.child.as_str() == inspected)
                            .map(crate::local_command::child_status_card);
                        app.set_inspected_detail(&inspected, card);
                    }
                    app.set_child_counts(
                        statuses
                            .iter()
                            .map(|status| {
                                (
                                    status.child.to_string(),
                                    smith_tui::app::ChildCounts {
                                        turns_used: status.turns_used,
                                        max_turns: status.max_turns,
                                        tokens_used: status.tokens_used,
                                    },
                                )
                            })
                            .collect(),
                    );
                }
                terminal.draw(|frame| smith_tui::draw_synced(frame, &mut app, theme))?;
                dirty = false;
            }
        }

        interactions.drain_answers(&mut app);
        host.set_goal_continuation_enabled(!app.should_defer_goal_continuation());
        if app.should_quit {
            break InteractiveExit::Quit(app.session_usage(), app.status.price().cloned());
        }
    };
    Ok(exit)
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
