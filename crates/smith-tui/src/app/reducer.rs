//! Canonical runtime-event reduction and speculative output projection.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::ids::TurnId;
use smith_runtime::client::{
    ChildPhase, ChildRecoveryState, PlanSensitivity, SmithEvent as EventEnvelope,
    SmithEventKind as RuntimeEvent, TurnFinish,
};

use crate::status::{Activity, ContextPlanUpdate, render_elapsed, render_terminal_elapsed};
use crate::transcript::ToolStatus;

use super::conversation::ConversationMut;
use super::state::*;

impl App {
    pub(super) fn finish_work(&mut self) {
        self.work = None;
    }

    /// Moves the provider round-trip stage, restarting its timer only on an
    /// actual stage change so a stream of same-stage deltas reads as one
    /// continuously running phase.
    fn set_provider_phase(&mut self, phase: Option<ProviderPhase>) {
        match phase {
            Some(next) => {
                if self.provider_phase.map(|(current, _)| current) != Some(next) {
                    self.provider_phase = Some((next, Instant::now()));
                }
            }
            None => self.provider_phase = None,
        }
    }

    /// Starts or restarts one child's panel clock for live work.
    fn run_child_clock(&mut self, child: &str) {
        self.child_clocks
            .entry(child.to_owned())
            .and_modify(|clock| clock.resume())
            .or_insert_with(ChildClock::started);
    }

    /// Freezes one child's panel clock at its settled elapsed time.
    fn settle_child_clock(&mut self, child: &str) {
        if let Some(clock) = self.child_clocks.get_mut(child) {
            clock.settle();
        }
    }

    /// The root conversation: this session's transcript and the provider
    /// output still deciding whether to join it.
    pub(super) fn conversation(&mut self) -> ConversationMut<'_> {
        ConversationMut {
            transcript: &mut self.transcript,
            speculative: &mut self.speculative,
        }
    }

    pub(super) fn reconcile_pending_terminal(
        &mut self,
        turn: Option<&TurnId>,
        finish: &TurnFinish,
    ) {
        // Disposition events are ordered before the terminal. This drain is a
        // fail-closed fallback for a live-stream gap: uncommitted text is kept,
        // never promoted to transcript history.
        let mut retained = VecDeque::new();
        while let Some(entry) = self.pending_input.accepted_steers.pop_front() {
            if turn == Some(&entry.receipt.turn) {
                self.push_rejected_followup(RejectedFollowup {
                    turn: Some(entry.receipt.turn),
                    interrupt_eligible: self.pending_input.interrupt_for_steer,
                    submission: entry.submission,
                });
            } else {
                retained.push_back(entry);
            }
        }
        self.pending_input.accepted_steers = retained;

        match finish {
            TurnFinish::Completed => {
                if self.pending_input.ready_submission.is_none() {
                    let rejected = self
                        .pending_input
                        .rejected_followups
                        .drain(..)
                        .map(|entry| entry.submission)
                        .collect::<Vec<_>>();
                    self.pending_input.ready_submission = PreparedSubmission::merge_fifo(rejected)
                        .or_else(|| self.pending_input.queued_turns.pop_front());
                }
            }
            TurnFinish::Cancelled { .. } if self.pending_input.interrupt_for_steer => {
                let mut resend = Vec::new();
                let mut keep = VecDeque::new();
                while let Some(entry) = self.pending_input.rejected_followups.pop_front() {
                    if entry.interrupt_eligible && entry.turn.as_ref() == turn {
                        resend.push(entry.submission);
                    } else {
                        keep.push_back(entry);
                    }
                }
                self.pending_input.rejected_followups = keep;
                self.pending_input.ready_submission = PreparedSubmission::merge_fifo(resend);
            }
            TurnFinish::Cancelled { .. }
            | TurnFinish::LimitReached { .. }
            | TurnFinish::NeedsInput { .. }
            | TurnFinish::Failed => {
                let mut restore = Vec::new();
                restore.extend(
                    self.pending_input
                        .accepted_steers
                        .drain(..)
                        .map(|entry| entry.submission),
                );
                restore.extend(
                    self.pending_input
                        .rejected_followups
                        .drain(..)
                        .map(|entry| entry.submission),
                );
                restore.extend(self.pending_input.queued_turns.drain(..));
                if let Some(ready) = self.pending_input.ready_submission.take() {
                    restore.push(ready);
                }
                if let Some(submission) = PreparedSubmission::merge_fifo(restore) {
                    self.restore_prepared_to_composer(submission);
                }
            }
        }
        self.pending_input.interrupt_for_steer = false;
    }

    /// Folds one live runtime event into state.
    ///
    /// An envelope that arrives past a sequence gap is **parked**, not
    /// applied: folding it ahead of the missing events would process control
    /// transitions out of order — a lost `TurnCompleted` would leave the UI
    /// working forever and strand queued input. The host drains the gap with
    /// [`App::take_stream_gap`], replays the missing range from the canonical
    /// journal through [`App::apply_recovered`], then applies the parked
    /// envelope the same way.
    pub fn apply(&mut self, envelope: &EventEnvelope) {
        // Only the host's replay flow should ever leave a gap parked across
        // two `apply` calls. If it happens anyway, keep ordering: fold the
        // parked envelope honestly before considering the newer one.
        if let Some(parked) = self.stream_gap.take() {
            self.apply_recovered(&parked.deferred);
        }
        if let Some(previous) = self.last_event_seq
            && envelope.seq > previous.saturating_add(1)
        {
            self.stream_gap = Some(StreamGap {
                first_missing: previous.saturating_add(1),
                last_missing: envelope.seq.saturating_sub(1),
                deferred: envelope.clone(),
            });
            return;
        }
        // Reaching here with no gap means the stream is caught up: any run
        // of gaps that was in progress just ended, so this is where it gets
        // reported.
        self.flush_gap_notices();
        self.apply_now(envelope);
    }

    /// Folds one journal-replayed (or replay-fallback) event into state.
    ///
    /// A sequence still missing here has already been asked of the journal
    /// (or the ring behind it) and it did not have the range either, so this
    /// is the honest "permanently gone" case, not a new gap to park. A
    /// broadcast-channel overrun does not produce one clean gap — it produces
    /// a burst of them in a row — so this does not report immediately: it
    /// merges the missing span into [`Self::pending_lost_range`] and returns,
    /// letting [`Self::flush_gap_notices`] emit exactly one line once the
    /// stream is caught up, however many gaps that took.
    pub fn apply_recovered(&mut self, envelope: &EventEnvelope) {
        if let Some(previous) = self.last_event_seq
            && envelope.seq > previous.saturating_add(1)
        {
            let first = previous.saturating_add(1);
            let last = envelope.seq.saturating_sub(1);
            self.pending_lost_range = Some(match self.pending_lost_range {
                Some((lowest, highest)) => (lowest.min(first), highest.max(last)),
                None => (first, last),
            });
        } else {
            self.flush_gap_notices();
        }
        self.apply_now(envelope);
    }

    /// Accumulates events recovered from the journal for the run of gaps
    /// currently being replayed, without emitting a transcript line yet.
    ///
    /// The terminal event loop calls this once per gap, right after a
    /// successful host-side journal read and before folding the recovered
    /// events in through [`Self::apply_recovered`]. Before this method
    /// existed that call site pushed its own notice immediately, so a burst
    /// of several gaps in a row — the exact pattern a broadcast-channel
    /// overrun produces — read as a wall of identical-looking lines.
    /// [`Self::flush_gap_notices`] reports the accumulated total as one line
    /// instead, once the run ends.
    pub fn note_recovered_events(&mut self, count: usize) {
        self.pending_recovered_events += count;
    }

    /// Emits at most one transcript line for the run of gaps that just ended.
    ///
    /// `push_notice`, never `push_error`: the user cannot act on a stream gap
    /// either way, and `Transcript::push_error` closes the open assistant
    /// block. Firing one of those per gap — instead of one per *run* of
    /// gaps, which is what this collapses to — was what fragmented a single
    /// streamed reply into several pieces.
    fn flush_gap_notices(&mut self) {
        if self.pending_recovered_events > 0 {
            self.transcript.push_notice(
                "stream",
                format!(
                    "live stream lagged; recovered {} skipped event(s) from the session journal",
                    self.pending_recovered_events
                ),
            );
            self.pending_recovered_events = 0;
        }
        if let Some((first, last)) = self.pending_lost_range.take() {
            self.transcript.push_notice(
                "stream",
                format!(
                    "live event stream sequence {first} through {last} is permanently gone — \
                     the session journal does not have it either, so the displayed history is \
                     now incomplete"
                ),
            );
        }
    }

    fn apply_now(&mut self, envelope: &EventEnvelope) {
        self.last_event_seq = Some(envelope.seq);
        // A pointer selection addresses rendered cells, so any event that can
        // append or reflow content moves the text out from under the
        // highlight. Dropping it is honest; repainting it over whatever landed
        // there instead is not.
        self.selection = None;

        // Cache evidence is a presentation projection of canonical runtime
        // events. Fold it before the semantic match so newly added runtime
        // cache variants remain replayable without entering conversation
        // history or provider context.
        self.status.record_cache_event(envelope);

        // Everything that becomes a transcript block goes through the shared
        // conversation fold — the same code a delegated child's stream goes
        // through. What remains below is this session's own business: header
        // status, the plan, the turn clock, steering, pending input, and the
        // panel of children. A child cannot reach any of it.
        self.conversation().apply(&envelope.payload);

        match &envelope.payload {
            RuntimeEvent::SessionStarted => {
                // A new session's children are new children; leaving the
                // inspector pointed at a name from the last one would show an
                // empty log under a stale identity.
                self.inspected_child = None;
                self.status.activity = Activity::Idle;
                self.status.goal = None;
                self.status.capabilities = Default::default();
                self.status.context_plan = None;
                self.plan = None;
                self.work = None;
                self.turn_started_at = None;
                self.turn_started_timestamp = None;
                self.speculative.clear();
                self.active_turn = None;
                self.pending_input = PendingInputState::default();
                self.provider_phase = None;
            }
            RuntimeEvent::TurnStarted | RuntimeEvent::InternalTurnStarted { .. } => {
                self.provider_phase = None;
                self.status.activity = Activity::Working;
                self.turn_summary = None;
                self.plan = None;
                self.work = Some(WorkSummary::default());
                self.turn_started_at = Some(Instant::now());
                self.turn_started_timestamp =
                    (envelope.timestamp != Timestamp::ZERO).then_some(envelope.timestamp);
                self.speculative.finalized.clear();
                self.active_turn.clone_from(&envelope.turn);
            }
            RuntimeEvent::TurnSteerCommitted { steer, .. } => {
                if let Some(index) = self
                    .pending_input
                    .accepted_steers
                    .iter()
                    .position(|entry| &entry.receipt.id == steer)
                    && let Some(entry) = self.pending_input.accepted_steers.remove(index)
                {
                    self.transcript.push_user(entry.submission.committed_text);
                    self.follow_newest();
                }
            }
            RuntimeEvent::TurnSteerDiscarded { steer, .. } => {
                if let Some(index) = self
                    .pending_input
                    .accepted_steers
                    .iter()
                    .position(|entry| &entry.receipt.id == steer)
                    && let Some(entry) = self.pending_input.accepted_steers.remove(index)
                {
                    let interrupt_eligible = self.pending_input.interrupt_for_steer
                        && envelope.turn.as_ref() == Some(&entry.receipt.turn);
                    self.push_rejected_followup(RejectedFollowup {
                        turn: Some(entry.receipt.turn),
                        interrupt_eligible,
                        submission: entry.submission,
                    });
                }
            }
            RuntimeEvent::ModelProfileResolved {
                provider, model, ..
            } => {
                let changed = self.status.provider.as_deref() != Some(provider.as_str())
                    || self.status.model != model.as_str();
                let had_provider = self.status.provider.is_some();
                if changed {
                    if had_provider {
                        self.transcript.push_notice(
                            "provider",
                            format!("changed to {provider}/{model} · prior cache not transferable"),
                        );
                    }
                    self.status
                        .switch_model(Some(provider.clone()), model.as_str());
                }
            }
            RuntimeEvent::RegistrySnapshotSealed { snapshot, entries } => {
                self.status.record_registry(snapshot.as_str(), *entries);
            }
            RuntimeEvent::ScopedViewDerived {
                view,
                visible_entries,
                ..
            } => {
                self.status
                    .record_scoped_view(view.as_str(), *visible_entries);
            }
            RuntimeEvent::CapabilityRetrievalPerformed {
                resolver_revision,
                candidates,
                ..
            } => {
                self.status.record_retrieval(
                    resolver_revision.as_str(),
                    candidates
                        .iter()
                        .take(16)
                        .map(ToString::to_string)
                        .collect(),
                );
            }
            RuntimeEvent::CapabilitiesActivated { epoch, activation } => {
                let capabilities = activation
                    .iter()
                    .take(16)
                    .map(|capability| capability.id.to_string())
                    .collect::<Vec<_>>();
                self.status.record_activation(*epoch, capabilities.clone());
                self.transcript.push_notice(
                    "capabilities",
                    if capabilities.is_empty() {
                        format!("activation epoch {epoch} has no optional capabilities")
                    } else {
                        format!("activation epoch {epoch}: {}", capabilities.join(", "))
                    },
                );
            }
            RuntimeEvent::ProviderAttemptStarted { .. } => {
                self.set_provider_phase(Some(ProviderPhase::Sending));
            }
            RuntimeEvent::TextDelta { .. } => {
                self.set_provider_phase(Some(ProviderPhase::Responding));
            }
            RuntimeEvent::ReasoningDelta { .. } => {
                self.set_provider_phase(Some(ProviderPhase::Thinking));
            }
            RuntimeEvent::ProviderAttemptFinished { .. } => {
                self.set_provider_phase(None);
            }
            // A harness turn has no provider round trip of its own, so the
            // phase line reports what the installed agent is doing instead of
            // sitting on "sending" for the whole turn.
            RuntimeEvent::ExternalText { .. } => {
                self.set_provider_phase(Some(ProviderPhase::Responding));
            }
            RuntimeEvent::ExternalReasoning { .. } => {
                self.set_provider_phase(Some(ProviderPhase::Thinking));
            }
            RuntimeEvent::ExternalToolInvoked { id, name } => {
                if let Some(work) = &mut self.work {
                    work.tools.insert(
                        id.clone(),
                        (name.clone(), ToolStatus::Running, Some(Instant::now())),
                    );
                }
            }
            RuntimeEvent::ExternalToolCompleted { id, ok } => {
                let status = if *ok {
                    ToolStatus::Ok
                } else {
                    ToolStatus::Failed
                };
                if let Some(work) = &mut self.work
                    && let Some((_, work_status, _)) = work.tools.get_mut(id.as_str())
                {
                    *work_status = status;
                }
            }
            RuntimeEvent::ToolCallRequested { call, name, .. } => {
                if let Some(work) = &mut self.work {
                    work.tools.insert(
                        call.as_str().to_owned(),
                        (name.clone(), ToolStatus::Running, Some(Instant::now())),
                    );
                }
            }
            RuntimeEvent::ToolCallCompleted { call, is_error, .. } => {
                let status = if *is_error {
                    ToolStatus::Failed
                } else {
                    ToolStatus::Ok
                };
                if let Some(work) = &mut self.work
                    && let Some((_, work_status, _)) = work.tools.get_mut(call.as_str())
                {
                    *work_status = status;
                }
            }
            RuntimeEvent::ContextPlanned {
                context,
                cache_plan,
                segment_count,
                totals,
                input_tokens,
                input_budget_tokens,
                reserved_tokens,
                confidence,
                ..
            } => {
                self.status.record_context_plan(ContextPlanUpdate {
                    fingerprint: context.as_str(),
                    cache_fingerprint: cache_plan.as_str(),
                    input_tokens: *input_tokens,
                    input_budget_tokens: *input_budget_tokens,
                    reserved_tokens: *reserved_tokens,
                    segment_count: *segment_count,
                    totals,
                    confidence: *confidence,
                });
            }
            RuntimeEvent::ContextCompacted {
                reclaimed_tokens,
                evicted,
                summaries,
                ..
            } => {
                self.status.record_compaction(*reclaimed_tokens);
                self.transcript.push_notice(
                    "context",
                    format!(
                        "compacted context · reclaimed {reclaimed_tokens} tokens · \
                         {} evicted · {} summaries",
                        evicted.len(),
                        summaries.len()
                    ),
                );
            }
            RuntimeEvent::PlanUpdated {
                revision,
                sensitivity,
                counts,
                items,
            } => {
                let plan = PlanSummary {
                    revision: *revision,
                    sensitivity: *sensitivity,
                    counts: counts.clone(),
                    items: if *sensitivity == PlanSensitivity::Public {
                        items.clone()
                    } else {
                        None
                    },
                };
                self.plan = Some(plan);
            }
            RuntimeEvent::GoalUpdated { goal, .. } => {
                self.status.set_goal(goal.clone());
            }
            RuntimeEvent::Usage { record } => {
                self.status.record_usage_record(record);
            }
            RuntimeEvent::CacheObservation { .. } | RuntimeEvent::CacheStateChanged { .. } => {}
            RuntimeEvent::TurnCompleted { finish, .. } => {
                self.provider_phase = None;
                self.reconcile_pending_terminal(envelope.turn.as_ref(), finish);
                self.cancel_pending_prompts();
                let elapsed = self.turn_started_at.take().map(|started| started.elapsed());
                let terminal_elapsed = self.turn_started_timestamp.take().and_then(|started| {
                    envelope
                        .timestamp
                        .as_millis()
                        .checked_sub(started.as_millis())
                        .map(Duration::from_millis)
                });
                self.status.activity = Activity::Idle;
                if self.active_turn.as_ref() == envelope.turn.as_ref() {
                    self.active_turn = None;
                }
                self.finish_work();
                if self.cache_miss_notices
                    && let Some(turn) = envelope.turn.as_ref().map(ToString::to_string)
                    && self.last_cache_notice_turn.as_deref() != Some(turn.as_str())
                    && let Some(notice) = self.status.cache_notice()
                {
                    self.transcript.push_notice("cache", notice);
                    self.last_cache_notice_turn = Some(turn);
                }
                match finish {
                    TurnFinish::Cancelled { reason } => {
                        self.transcript.push_notice(
                            "turn",
                            elapsed.map_or_else(
                                || format!("Interrupted ({reason:?})"),
                                |elapsed| {
                                    format!(
                                        "Interrupted after {} ({reason:?})",
                                        render_elapsed(elapsed)
                                    )
                                },
                            ),
                        );
                    }
                    TurnFinish::LimitReached { limit } => {
                        self.transcript.push_notice(
                            "turn",
                            elapsed.map_or_else(
                                || format!("Stopped at the {limit:?} limit"),
                                |elapsed| {
                                    format!(
                                        "Stopped after {} at the {limit:?} limit",
                                        render_elapsed(elapsed)
                                    )
                                },
                            ),
                        );
                    }
                    TurnFinish::NeedsInput { request } => {
                        self.transcript.push_notice(
                            "turn",
                            elapsed.map_or_else(
                                || format!("Waiting for parent input · request {request}"),
                                |elapsed| {
                                    format!(
                                        "Waiting for parent input after {} · request {request}",
                                        render_elapsed(elapsed)
                                    )
                                },
                            ),
                        );
                    }
                    TurnFinish::Completed => {
                        // Routine completions never enter the transcript: one
                        // row per historical turn is log detail, not UI. The
                        // newest summary renders beneath the transcript until
                        // the next turn starts.
                        self.turn_summary = Some(terminal_elapsed.map_or_else(
                            || "Worked".to_owned(),
                            |elapsed| format!("Worked for {}", render_terminal_elapsed(elapsed)),
                        ));
                    }
                    TurnFinish::Failed => {
                        self.transcript.push_notice(
                            "turn",
                            elapsed.map_or_else(
                                || "Failed".to_owned(),
                                |elapsed| format!("Failed after {}", render_elapsed(elapsed)),
                            ),
                        );
                    }
                }
            }
            // Child lifecycle reaches the panel and the child's own log
            // immediately, before the parent model is told anything: results
            // reach the model only at a safe boundary, but the user watches
            // the child work in real time.
            //
            // Only delegation's boundaries — a child starting, and a child
            // ending — are the root conversation's business. Mid-flight
            // progress is panel state and inspector history: a running child
            // narrating every tool call buries the transcript it was spawned
            // to serve.
            RuntimeEvent::ChildSpawned {
                child,
                workspace,
                max_turns,
                ..
            } => {
                // Correlates back to the spawn row by tool-call id — see
                // `App::note_pending_spawn` — popped once and reused for
                // both the panel's profile and the transcript row's
                // enrichment, so the two never disagree about which spawn
                // this is.
                let spawn = self.pending_spawns.pop_front();
                let profile = spawn
                    .as_ref()
                    .and_then(|spawn| spawn.profile.clone())
                    .unwrap_or_else(|| self.status.agent.clone());

                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "running".to_owned(),
                        detail: Some(describe_workspace(workspace)),
                        profile: Some(profile),
                    },
                );
                self.child_clocks
                    .insert(child.to_string(), ChildClock::started());
                // An unbounded child names no turn budget rather than the
                // sentinel's absurd number.
                let terms = if *max_turns == u32::MAX {
                    describe_workspace(workspace).to_owned()
                } else {
                    format!(
                        "{} · up to {max_turns} turns",
                        describe_workspace(workspace)
                    )
                };
                self.push_child_notice(child.as_str(), "started", terms.clone());
                // The reviewed spawn row is now the one place a spawn is
                // announced in the transcript — see `child-agents`'s "Safe
                // parent reporting": "A spawn SHALL announce itself exactly
                // once". The per-child log entry above and the panel row
                // still carry the same facts.
                if let Some(spawn) = spawn {
                    // Only what the row does not already say. The reviewed
                    // projection has already named the workspace from the
                    // call's own argument — and named it more precisely, since
                    // an explicit directory shows its path there — so
                    // re-appending the resolved posture here would print one
                    // fact twice in two spellings (`workspace read only` from
                    // the projector, `read-only` from `describe_workspace`).
                    // Printing delegation twice is the thing this change
                    // exists to stop.
                    let mut enrichment = vec![child.to_string()];
                    if *max_turns != u32::MAX {
                        enrichment.push(format!("up to {max_turns} turns"));
                    }
                    // The projector already emitted a `profile <name>`
                    // qualifier when the spawn selected one; only an absent
                    // selection gets a qualifier here, explicitly labelled
                    // inherited, so the row never claims the call chose a
                    // profile it did not.
                    if spawn.profile.is_none() {
                        enrichment.push(format!("profile {} (inherited)", self.status.agent));
                    }
                    self.transcript.enrich_tool_call(&spawn.call_id, enrichment);
                }
            }
            RuntimeEvent::ChildProgress { child, phase } => match phase {
                ChildPhase::Recovered {
                    child_session,
                    state,
                    resumable,
                } => {
                    let state = match state {
                        ChildRecoveryState::Idle => "idle",
                        ChildRecoveryState::Interrupted => "interrupted",
                        ChildRecoveryState::Blocked => "blocked",
                        ChildRecoveryState::Expired => "expired",
                        ChildRecoveryState::Terminal => "terminal",
                    };
                    let detail = format!(
                        "durable · session {child_session}{}",
                        if *resumable { " · resumable" } else { "" }
                    );
                    let profile = self.carried_child_profile(child.as_str());
                    self.children.insert(
                        child.to_string(),
                        ChildSummary {
                            state: state.to_owned(),
                            detail: Some(detail.clone()),
                            profile,
                        },
                    );
                    // A recovered record ran in another process; no honest
                    // wall-clock exists for it.
                    self.child_clocks.remove(child.as_str());
                    // Recovery is bookkeeping about a child that already
                    // exists, and a resumed session recovers all of them at
                    // once. The panel lists them; the transcript stays quiet.
                    self.push_child_notice(
                        child.as_str(),
                        "recovered",
                        format!("{state} · {detail}"),
                    );
                }
                ChildPhase::TurnStarted => {
                    if let Some(summary) = self.children.get_mut(&child.to_string()) {
                        summary.state = "working".to_owned();
                    }
                    self.run_child_clock(child.as_str());
                    // A turn boundary, drawn as the root timeline draws its
                    // own: quiet punctuation, not a sourced notice row.
                    self.push_child_notice(child.as_str(), "turn", "working");
                }
                ChildPhase::ResumeStarted { child_session } => {
                    let profile = self.carried_child_profile(child.as_str());
                    self.children.insert(
                        child.to_string(),
                        ChildSummary {
                            state: "resuming".to_owned(),
                            detail: Some(format!("exact checkpoint · session {child_session}")),
                            profile,
                        },
                    );
                    self.run_child_clock(child.as_str());
                    self.push_child_notice(
                        child.as_str(),
                        "resuming",
                        format!("exact checkpoint · session {child_session}"),
                    );
                    self.transcript.push_notice(
                        "sub-agent",
                        format!("{child} is resuming its exact checkpoint"),
                    );
                }
                ChildPhase::Interrupted {
                    child_session,
                    resumable,
                } => {
                    let detail = format!(
                        "durable · session {child_session}{}",
                        if *resumable {
                            " · exact resume available"
                        } else {
                            " · no compatible checkpoint"
                        }
                    );
                    let profile = self.carried_child_profile(child.as_str());
                    self.children.insert(
                        child.to_string(),
                        ChildSummary {
                            state: "interrupted".to_owned(),
                            detail: Some(detail.clone()),
                            profile,
                        },
                    );
                    self.settle_child_clock(child.as_str());
                    self.settle_child_tool_calls(child.as_str());
                    self.push_child_notice(child.as_str(), "interrupted", detail.clone());
                    self.transcript
                        .push_notice("sub-agent", format!("{child} interrupted · {detail}"));
                }
                // The completed/stopped notice that follows says everything a
                // bare "finished a turn" would.
                ChildPhase::TurnFinished => {}
            },
            RuntimeEvent::ChildNeedsInput {
                child,
                request,
                question_ids,
                sensitivity,
                ..
            } => {
                let question_count = question_ids.len();
                let detail = format!(
                    "{question_count} {} · {} · request {request}",
                    if question_count == 1 {
                        "question"
                    } else {
                        "questions"
                    },
                    describe_interaction_sensitivity(sensitivity),
                );
                let profile = self.carried_child_profile(child.as_str());
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "needs input".to_owned(),
                        detail: Some(detail.clone()),
                        profile,
                    },
                );
                self.push_child_notice(child.as_str(), "needs input", detail.clone());
                // A blocked child is waiting on the user, not working: this
                // one is an ask, not progress, so it stays in the transcript.
                self.transcript
                    .push_notice("sub-agent", format!("{child} needs input · {detail}"));
            }
            RuntimeEvent::ChildCompleted { child, result } => {
                let mut summary: String = result.chars().take(200).collect();
                if summary.chars().count() < result.chars().count() {
                    summary.push('…');
                }
                if summary.is_empty() {
                    summary = "(no visible answer)".to_owned();
                }
                let profile = self.carried_child_profile(child.as_str());
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "completed".to_owned(),
                        detail: Some(summary.clone()),
                        profile,
                    },
                );
                self.settle_child_clock(child.as_str());
                self.settle_child_tool_calls(child.as_str());
                // The panel row and the root notice are one-liners and stay
                // clipped. The inspector is where the delegated work is
                // actually read, so it keeps the answer whole, as the prose
                // it is.
                self.push_child_answer(child.as_str(), result);
                self.transcript
                    .push_notice("sub-agent", format!("{child} completed: {summary}"));
                self.arm_child_dismissal(child.as_str());
            }
            RuntimeEvent::ChildStopped { child, reason } => {
                let detail = describe_cancel_reason(reason);
                let profile = self.carried_child_profile(child.as_str());
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "stopped".to_owned(),
                        detail: Some(detail.clone()),
                        profile,
                    },
                );
                self.settle_child_clock(child.as_str());
                self.settle_child_tool_calls(child.as_str());
                self.push_child_notice(child.as_str(), "stopped", detail.clone());
                self.transcript
                    .push_notice("sub-agent", format!("{child} stopped · {detail}"));
            }
            RuntimeEvent::ChildFailed { child, error } => {
                let profile = self.carried_child_profile(child.as_str());
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "failed".to_owned(),
                        detail: Some(error.message.clone()),
                        profile,
                    },
                );
                self.settle_child_clock(child.as_str());
                self.settle_child_tool_calls(child.as_str());
                self.push_child_error(child.as_str(), error.message.clone());
                self.transcript
                    .push_error(format!("sub-agent {child} failed: {}", error.message));
            }
            RuntimeEvent::SessionShutdown => {
                self.provider_phase = None;
                // The session is over; a still-ticking child clock would lie.
                for clock in self.child_clocks.values_mut() {
                    clock.settle();
                }
                self.cancel_pending_prompts();
                self.status.activity = Activity::Ended;
                self.turn_started_at = None;
                self.turn_started_timestamp = None;
                self.active_turn = None;
                self.pending_input = PendingInputState::default();
            }
            // Planning-lifecycle events carry diagnostics the basic TUI does not
            // surface yet; they are recorded by the session log regardless.
            _ => {}
        }
        self.refresh_parked_activity();
    }

    fn refresh_parked_activity(&mut self) {
        if matches!(
            self.status.activity,
            Activity::Working | Activity::Interrupting | Activity::Ended
        ) {
            return;
        }
        let pending_child = self
            .children
            .values()
            .any(|child| matches!(child.state.as_str(), "running" | "working" | "resuming"));
        self.status.activity = if pending_child {
            Activity::ParkedAwaitingChild
        } else {
            Activity::Idle
        };
    }
}
