//! Canonical runtime-event reduction and speculative output projection.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::event::{
    ChildPhase, ChildRecoveryState, EventEnvelope, PlanSensitivity, RuntimeEvent, TurnFinish,
};
use agent_runtime_core::ids::{AttemptId, RequestId, TurnId};

use crate::status::{Activity, ContextPlanUpdate, render_elapsed, render_terminal_elapsed};
use crate::transcript::ToolStatus;

use super::state::*;

impl App {
    pub(super) fn finish_work(&mut self) {
        self.work = None;
    }

    pub(super) fn begin_speculative_attempt(&mut self, request: &RequestId, attempt: &AttemptId) {
        let key = AttemptOutputKey::new(request, attempt);
        if self.finalized_attempts.contains(&key) {
            self.transcript.push_error(format!(
                "provider attempt {} for request {} restarted after its output terminal",
                attempt, request
            ));
            return;
        }
        if !self.speculative_attempts.contains_key(&key) {
            self.speculative_order.push(key.clone());
            self.speculative_attempts
                .insert(key, SpeculativeAttempt::default());
        }
    }

    pub(super) fn buffer_speculative_text(
        &mut self,
        request: &RequestId,
        attempt: &AttemptId,
        text: &str,
    ) {
        self.begin_speculative_attempt(request, attempt);
        let key = AttemptOutputKey::new(request, attempt);
        if let Some(output) = self.speculative_attempts.get_mut(&key) {
            output.push_text(text);
        }
    }

    pub(super) fn buffer_speculative_reasoning(
        &mut self,
        request: &RequestId,
        attempt: &AttemptId,
        text: &str,
        redacted: bool,
    ) {
        self.begin_speculative_attempt(request, attempt);
        let key = AttemptOutputKey::new(request, attempt);
        if let Some(output) = self.speculative_attempts.get_mut(&key) {
            output.push_reasoning(text, redacted);
        }
    }

    pub(super) fn finish_speculative_attempt(
        &mut self,
        request: &RequestId,
        attempt: &AttemptId,
        commit: bool,
    ) {
        let key = AttemptOutputKey::new(request, attempt);
        let Some(output) = self.speculative_attempts.remove(&key) else {
            if !self.finalized_attempts.contains(&key) {
                self.transcript.push_error(format!(
                    "provider attempt {} for request {} ended without a start",
                    attempt, request
                ));
                self.finalized_attempts.insert(key);
            }
            return;
        };
        self.speculative_order.retain(|candidate| candidate != &key);
        if !self.finalized_attempts.insert(key) {
            return;
        }

        if commit {
            for chunk in output.chunks {
                match chunk {
                    SpeculativeChunk::Text(text) => self.transcript.push_text_delta(&text),
                    SpeculativeChunk::Reasoning { text, redacted } => {
                        self.transcript.push_reasoning_delta(&text, redacted);
                    }
                }
            }
        } else if !output.chunks.is_empty() {
            self.transcript.push_notice(
                "retry",
                format!("discarded speculative output from provider attempt {attempt}"),
            );
        }
    }

    pub(super) fn discard_orphaned_speculative_output(&mut self, boundary: &str) {
        let orphaned = self.speculative_attempts.len();
        if orphaned == 0 {
            return;
        }
        self.speculative_attempts.clear();
        self.speculative_order.clear();
        self.transcript.push_notice(
            "integrity",
            format!(
                "discarded {orphaned} unterminated speculative provider attempt(s) at {boundary}"
            ),
        );
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

    /// Folds one runtime event into state.
    pub fn apply(&mut self, envelope: &EventEnvelope) {
        if let Some(previous) = self.last_event_seq
            && envelope.seq > previous.saturating_add(1)
        {
            self.transcript.push_error(format!(
                "live event stream skipped sequence {} through {}; \
                 the persisted session journal remains canonical",
                previous.saturating_add(1),
                envelope.seq.saturating_sub(1)
            ));
        }
        self.last_event_seq = Some(envelope.seq);

        match &envelope.payload {
            RuntimeEvent::SessionStarted => {
                self.status.activity = Activity::Idle;
                self.status.goal = None;
                self.status.capabilities = Default::default();
                self.status.context_plan = None;
                self.plan = None;
                self.work = None;
                self.turn_started_at = None;
                self.turn_started_timestamp = None;
                self.speculative_attempts.clear();
                self.speculative_order.clear();
                self.finalized_attempts.clear();
                self.active_turn = None;
                self.pending_input = PendingInputState::default();
            }
            RuntimeEvent::TurnStarted | RuntimeEvent::InternalTurnStarted { .. } => {
                self.discard_orphaned_speculative_output("next turn start");
                self.status.activity = Activity::Working;
                self.turn_summary = None;
                self.plan = None;
                self.work = Some(WorkSummary::default());
                self.turn_started_at = Some(Instant::now());
                self.turn_started_timestamp =
                    (envelope.timestamp != Timestamp::ZERO).then_some(envelope.timestamp);
                self.finalized_attempts.clear();
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
            RuntimeEvent::ProviderAttemptStarted {
                request, attempt, ..
            } => {
                self.begin_speculative_attempt(request, attempt);
            }
            RuntimeEvent::TextDelta {
                request,
                attempt,
                text,
            } => {
                self.buffer_speculative_text(request, attempt, text);
            }
            RuntimeEvent::ReasoningDelta {
                request,
                attempt,
                text,
                redacted,
            } => {
                self.buffer_speculative_reasoning(request, attempt, text, *redacted);
            }
            RuntimeEvent::ProviderAttemptOutputCommitted { request, attempt } => {
                self.finish_speculative_attempt(request, attempt, true);
            }
            RuntimeEvent::ProviderAttemptOutputDiscarded { request, attempt } => {
                self.finish_speculative_attempt(request, attempt, false);
            }
            RuntimeEvent::ToolCallRequested {
                call,
                name,
                argument_keys,
                arguments,
                ..
            } => {
                if let Some(work) = &mut self.work {
                    work.tools.insert(
                        call.as_str().to_owned(),
                        (name.clone(), ToolStatus::Running),
                    );
                }
                self.transcript.push_tool_call(
                    call.as_str(),
                    name,
                    arguments.as_ref(),
                    argument_keys,
                );
            }
            RuntimeEvent::ToolCallCompleted { call, is_error, .. } => {
                let status = if *is_error {
                    ToolStatus::Failed
                } else {
                    ToolStatus::Ok
                };
                if let Some(work) = &mut self.work
                    && let Some((_, work_status)) = work.tools.get_mut(call.as_str())
                {
                    *work_status = status;
                }
                self.transcript.complete_tool_call(call.as_str(), status);
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
                self.status.record_usage(&record.delta);
            }
            RuntimeEvent::CacheObservation { read_tokens, .. } => {
                self.status.record_cache(*read_tokens);
            }
            RuntimeEvent::Downgrade { capability, detail } => {
                self.transcript
                    .push_notice("downgrade", format!("{capability}: {detail}"));
            }
            RuntimeEvent::LimitReached { limit } => {
                self.transcript
                    .push_notice("limit", format!("{limit:?} reached"));
            }
            RuntimeEvent::Error { error } => {
                self.transcript.push_error(error.to_string());
            }
            RuntimeEvent::TurnCompleted { finish, .. } => {
                self.reconcile_pending_terminal(envelope.turn.as_ref(), finish);
                self.cancel_pending_prompts();
                // A valid v5 stream has already committed or discarded every
                // attempt. A gap, corrupt journal, or incompatible producer
                // must fail closed: orphan text cannot become visible while
                // idle or leak into the next turn.
                self.discard_orphaned_speculative_output("turn completion");
                let elapsed = self.turn_started_at.take().map(|started| started.elapsed());
                let terminal_elapsed = self.turn_started_timestamp.take().and_then(|started| {
                    envelope
                        .timestamp
                        .as_millis()
                        .checked_sub(started.as_millis())
                        .map(Duration::from_millis)
                });
                self.transcript.close_open();
                self.status.activity = Activity::Idle;
                if self.active_turn.as_ref() == envelope.turn.as_ref() {
                    self.active_turn = None;
                }
                self.finish_work();
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
            // Child lifecycle appears immediately, before the parent model is
            // told anything: results reach the model only at a safe boundary,
            // but the user watches the child work in real time.
            RuntimeEvent::ChildSpawned {
                child,
                workspace,
                max_turns,
                ..
            } => {
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "running".to_owned(),
                        detail: Some(describe_workspace(workspace)),
                    },
                );
                self.transcript.push_notice(
                    "sub-agent",
                    format!(
                        "{child} started · {} · up to {max_turns} turns",
                        describe_workspace(workspace)
                    ),
                );
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
                    self.children.insert(
                        child.to_string(),
                        ChildSummary {
                            state: state.to_owned(),
                            detail: Some(detail.clone()),
                        },
                    );
                    self.transcript
                        .push_notice("sub-agent", format!("{child} recovered {state} · {detail}"));
                }
                ChildPhase::TurnStarted => {
                    if let Some(summary) = self.children.get_mut(&child.to_string()) {
                        summary.state = "working".to_owned();
                    }
                    self.transcript
                        .push_notice("sub-agent", format!("{child} is working"));
                }
                ChildPhase::ResumeStarted { child_session } => {
                    self.children.insert(
                        child.to_string(),
                        ChildSummary {
                            state: "resuming".to_owned(),
                            detail: Some(format!("exact checkpoint · session {child_session}")),
                        },
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
                    self.children.insert(
                        child.to_string(),
                        ChildSummary {
                            state: "interrupted".to_owned(),
                            detail: Some(detail.clone()),
                        },
                    );
                    self.transcript
                        .push_notice("sub-agent", format!("{child} interrupted · {detail}"));
                }
                ChildPhase::ToolCall { name } => {
                    if let Some(summary) = self.children.get_mut(&child.to_string()) {
                        summary.detail = Some(format!("ran {name}"));
                    }
                    self.transcript
                        .push_notice("sub-agent", format!("{child} ran {name}"));
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
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "needs input".to_owned(),
                        detail: Some(detail.clone()),
                    },
                );
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
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "completed".to_owned(),
                        detail: Some(summary.clone()),
                    },
                );
                self.transcript
                    .push_notice("sub-agent", format!("{child} completed: {summary}"));
            }
            RuntimeEvent::ChildStopped { child, reason } => {
                let detail = describe_cancel_reason(reason);
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "stopped".to_owned(),
                        detail: Some(detail.clone()),
                    },
                );
                self.transcript
                    .push_notice("sub-agent", format!("{child} stopped · {detail}"));
            }
            RuntimeEvent::ChildFailed { child, error } => {
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "failed".to_owned(),
                        detail: Some(error.message.clone()),
                    },
                );
                self.transcript
                    .push_error(format!("sub-agent {child} failed: {}", error.message));
            }
            RuntimeEvent::SessionShutdown => {
                self.cancel_pending_prompts();
                self.discard_orphaned_speculative_output("session shutdown");
                self.transcript.close_open();
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
    }
}
