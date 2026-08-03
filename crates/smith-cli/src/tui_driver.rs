//! Interactive terminal event loop and TUI action routing.

use super::*;

pub(super) enum InteractiveExit {
    Quit(smith_tui::status::SessionUsage),
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
}

pub(super) async fn run_interactive(
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

pub(super) struct TuiRunInputs<'a> {
    host: &'a HostSession,
    project: &'a std::path::Path,
    approvals: Option<ApprovalRequests>,
    interactions: Option<InteractionRequests>,
    agents: &'a ResolvedAgent,
    theme: Theme,
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
                            Some(Action::Quit) => break InteractiveExit::Quit(app.status.session_usage()),
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
                    None => break InteractiveExit::Quit(app.status.session_usage()),
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
        host.set_goal_continuation_enabled(!app.should_defer_goal_continuation());
        if app.should_quit {
            break InteractiveExit::Quit(app.status.session_usage());
        }
    };
    Ok(exit)
}

pub(super) async fn next_approval(
    approvals: &mut Option<ApprovalRequests>,
) -> Option<ApprovalPrompt> {
    match approvals {
        Some(approvals) => approvals.recv().await,
        None => std::future::pending().await,
    }
}
