use super::{
    AgentSnapshot, ChildId, ChildPhase, EventEnvelope, InteractiveExit, LocalOutcome, NoticeKind,
    Result, RunningTaskSummary, RuntimeEvent, SubmissionTarget, TuiLoop, VecDeque, account_entries,
    account_status, change_notice, compact_command_hint, dispatch_prepared_with_materialization,
    remember_active_account, resolve_child_usage_binding, subscribe_to_child,
    tool_call_for_display,
};

impl TuiLoop<'_> {
    pub(super) async fn on_runtime_event(
        &mut self,
        envelope: Option<EventEnvelope>,
    ) -> Option<InteractiveExit> {
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
                    let completed_tool =
                        matches!(envelope.payload, RuntimeEvent::ToolCallCompleted { .. });
                    let turn_completed =
                        matches!(envelope.payload, RuntimeEvent::TurnCompleted { .. });
                    if recovered {
                        self.app.apply_recovered(&envelope);
                    } else {
                        self.app.apply(&envelope);
                        if let Some(gap) = self.app.take_stream_gap() {
                            // The envelope was parked, not applied.
                            // Queue the journal's copy of the missing
                            // range first, then retry the parked
                            // envelope on the honest replay path.
                            self.recover_stream_gap(gap, &mut pending).await;
                            continue;
                        }
                    }
                    if turn_completed && self.host.runtime().advisor_route().is_some() {
                        self.app
                            .status
                            .reconcile_advisor_records(self.host.snapshot().usage.records());
                    }
                    if turn_completed
                        && let Some(set) = self.host.changes().latest()
                        && self.last_change_turn != Some(set.turn)
                        && !set.undone
                        && let Some(notice) = change_notice(&set)
                    {
                        self.last_change_turn = Some(set.turn);
                        self.app.transcript.push_notice(NoticeKind::Changes, notice);
                    }
                    if let Some(submission) = self.app.take_ready_submission() {
                        dispatch_prepared_with_materialization(
                            &mut self.app,
                            self.session,
                            self.project,
                            submission,
                            SubmissionTarget::WholeTurn,
                        )
                        .await;
                    }
                    if let Some(call) = tool_call {
                        if let Some(display) = self.host.tool_call_display(&call) {
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
                                self.app.note_pending_spawn(call.as_str(), &display);
                            }
                            self.app.set_tool_display(call.as_str(), display);
                        }
                        if completed_tool && let Some(text) = self.host.tool_result_text(&call) {
                            self.app.set_tool_result_preview(call.as_str(), text);
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
                        RuntimeEvent::ChildSpawned { child, .. } => {
                            let binding =
                                self.app.child_profile(child.as_str()).and_then(|profile| {
                                    resolve_child_usage_binding(
                                        profile,
                                        self.inventory,
                                        self.catalog,
                                    )
                                });
                            self.app.set_child_usage_binding(child.as_str(), binding);
                            subscribe_to_child(self.host, child, self.child_tx.clone());
                        }
                        RuntimeEvent::ChildProgress {
                            child,
                            phase: ChildPhase::ResumeStarted { .. },
                        } => subscribe_to_child(self.host, child, self.child_tx.clone()),
                        _ => {}
                    }
                }
                self.dirty = true;
            }
            None => {
                return Some(InteractiveExit::Quit(
                    Box::new(self.app.session_usage()),
                    self.app.status.price().cloned(),
                    self.app.status.cache_summary().map(Box::new),
                ));
            }
        }
        None
    }

    async fn recover_stream_gap(
        &mut self,
        gap: smith_tui::app::StreamGap,
        pending: &mut VecDeque<(EventEnvelope, bool)>,
    ) {
        match self
            .host
            .client_events_between(gap.first_missing, gap.last_missing)
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
                    self.app.note_recovered_events(events.len());
                }
                pending.push_front((gap.deferred, true));
                for event in events.into_iter().rev() {
                    pending.push_front((event, true));
                }
            }
            Err(error) => {
                self.app.transcript.push_error(format!(
                    "replaying skipped events {}–{} from the \
                                                 session journal failed: {error}",
                    gap.first_missing, gap.last_missing
                ));
                pending.push_front((gap.deferred, true));
            }
        }
    }

    pub(super) fn on_child_event(&mut self, child_event: Option<(ChildId, EventEnvelope)>) {
        if let Some((child, envelope)) = child_event {
            self.app.apply_child(child.as_str(), &envelope);
            // The child's events withhold argument values and result
            // text exactly as the root's do. Both are resolved the
            // same way: by call id, against that agent's canonical
            // history, redacted by the host.
            if let Some(call) = tool_call_for_display(&envelope.payload) {
                if let Some(display) = self.host.child_tool_call_display(&child, &call) {
                    self.app
                        .set_child_tool_display(child.as_str(), call.as_str(), display);
                }
                if matches!(envelope.payload, RuntimeEvent::ToolCallCompleted { .. })
                    && let Some(text) = self.host.child_tool_result_text(&child, &call)
                {
                    self.app
                        .set_child_tool_result_preview(child.as_str(), call.as_str(), text);
                }
            }
            self.dirty = true;
        }
    }

    pub(super) fn on_local_result(&mut self, outcome: Option<LocalOutcome>) {
        if let Some(outcome) = outcome {
            match outcome {
                LocalOutcome::Agent(report) => {
                    self.app
                        .transcript
                        .push_local(smith_client::local_result::LocalResult::Agent(report));
                }
                LocalOutcome::Review(report) => {
                    self.app
                        .transcript
                        .push_local(smith_client::local_result::LocalResult::Review(report));
                }
                LocalOutcome::Notice { kind, text } => {
                    self.app.transcript.push_notice(kind, text);
                }
                LocalOutcome::Error(text) => self.app.transcript.push_error(text),
                LocalOutcome::Shell {
                    echo,
                    call,
                    content,
                    is_error,
                } => {
                    self.shell_shortcuts.finish(
                        self.host,
                        &mut self.app,
                        echo,
                        call.as_ref().map(|call| call.as_str()),
                        &content,
                        is_error,
                    );
                }
            }
            self.dirty = true;
        }
    }

    pub(super) fn on_mcp_change(&mut self) {
        if let Some(context) = &self.mcp {
            let supervisor = context.supervisor();
            let reports = supervisor.reports();
            self.app.status.mcp = smith_tui::McpStatus {
                connecting: reports
                    .iter()
                    .filter(|report| !report.state.is_settled())
                    .count(),
                failed: reports
                    .iter()
                    .filter(|report| {
                        matches!(report.state, smith_runtime::mcp::McpState::Failed { .. })
                    })
                    .count(),
            };
            self.remote_tools_pending = supervisor.tools().len() != self.composed_remote_tools;
            self.dirty = true;
        }
    }

    pub(super) fn on_spinner(&mut self) {
        let exit_hint_expired = self.app.expire_ctrl_c_exit_hint();
        // Rows retire while the session is idle — that is the whole
        // point of them retiring — so this cannot ride on `tick`,
        // which only advances while there is work to animate.
        let rows_retired = self.app.expire_child_rows();
        let busy = self.app.is_busy();
        if busy {
            self.app.tick();
        }
        if exit_hint_expired
            || rows_retired
            || (busy && (self.theme.uses_motion() || self.app.tick.is_multiple_of(10)))
        {
            self.dirty = true;
        }
    }

    pub(super) async fn on_frame<B>(
        &mut self,
        terminal: &mut ratatui::Terminal<B>,
    ) -> Result<Option<InteractiveExit>>
    where
        B: ratatui::backend::Backend,
        B::Error: Send + Sync + 'static,
    {
        // A newly connected server's tools and a newly trusted skill
        // both join at the next idle boundary, never mid-turn:
        // swapping the ability set underneath a running turn is what
        // the epoch rules exist to prevent.
        if (self.remote_tools_pending || self.trusted_skill_pending)
            && !self.app.is_busy()
            && !self.app.has_pending_input()
            && !self.app.has_pending_prompt()
            && self.app.overlay.is_none()
        {
            return Ok(Some(InteractiveExit::CapabilitiesChanged));
        }
        // Re-read on the way to the screen rather than at each site
        // that could change it: the pool also moves on its own — a
        // rotation the runtime performed, a snapshot that arrived
        // mid-turn — and a footer refreshed only on manual switches
        // would keep naming an account the session had already left.
        if self.credential_pool.is_some() {
            self.app.status.account = account_status(self.credential_pool.as_ref());
            self.app
                .set_accounts(account_entries(self.credential_pool.as_ref()));
            // Rotation happens inside the runtime, which cannot reach
            // user-scope state, so the account it moved to is
            // remembered here. `remember` reports whether anything
            // changed, so this writes on a switch and not on a frame.
            remember_active_account(self.credential_pool.as_ref(), &mut self.accounts).await;
        }
        // Same cadence as the account refresh above: the TUI never
        // reaches the registry itself, so this poll-on-redraw is the
        // only path by which a task's start or terminal state
        // reaches operational status and the exit-confirm gate.
        self.app.set_running_tasks(
            self.host
                .background_tasks()
                .running_tasks(self.session.id())
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
        if let Some(coordinator) = self
            .host
            .runtime()
            .delegation()
            .and_then(|delegation| delegation.coordinator())
        {
            let statuses = coordinator.list();
            if let Some(inspected) = self.app.inspected_child.clone() {
                let card = statuses
                    .iter()
                    .find(|status| status.child.as_str() == inspected)
                    .map(AgentSnapshot::from);
                self.app.set_inspected_detail(&inspected, card);
            }
            self.app.set_child_counts(
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
        // The title rides the redraw cadence: every input that can
        // change it (model switch, project label, activity
        // transition) marks the frame dirty on its way in, and the
        // tracker turns that into at most one OSC write per change.
        let _ = self.window_title.refresh(&self.app.status);
        terminal.draw(|frame| {
            smith_tui::render::layout(frame.area(), &self.app, self.theme).apply(&mut self.app);
            smith_tui::render::draw(frame, &self.app, self.theme);
        })?;
        self.dirty = false;
        Ok(None)
    }

    pub(super) fn after_event(&mut self) -> Option<InteractiveExit> {
        self.interactions.drain_answers(&mut self.app);
        self.host
            .set_goal_continuation_enabled(!self.app.should_defer_goal_continuation());
        if self.app.should_quit {
            return Some(InteractiveExit::Quit(
                Box::new(self.app.session_usage()),
                self.app.status.price().cloned(),
                self.app.status.cache_summary().map(Box::new),
            ));
        }
        None
    }
}
