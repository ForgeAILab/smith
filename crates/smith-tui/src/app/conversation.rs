//! One agent's conversation, and the fold that builds it.
//!
//! A delegated child is an agent that reports back, not a different kind of
//! thing. It runs as a full runtime session and emits the same events the root
//! session does, so the client folds both through the code in this module: the
//! same speculative-output rules, the same tool rows, the same transcript.
//!
//! What stays out of here is everything that belongs to *a session the user is
//! driving* rather than to a conversation — header status, the plan, steering,
//! pending input, the turn clock. A child has no business setting the root's
//! spinner, and the type system is where that is enforced: this fold cannot
//! reach those fields because it never borrows them.

use std::collections::{BTreeMap, BTreeSet};

use agent_runtime_core::ids::{AttemptId, RequestId};
use smith_runtime::client::SmithEventKind as RuntimeEvent;

use crate::transcript::{ToolStatus, Transcript};

use super::state::{AttemptOutputKey, SpeculativeAttempt, SpeculativeChunk};

/// Provider output held back until an attempt explicitly commits it.
///
/// A retried attempt's text must never reach the transcript, so deltas
/// accumulate here and are promoted — or discarded — only on the attempt's
/// terminal event.
#[derive(Debug, Default)]
pub(super) struct SpeculativeState {
    pub(super) attempts: BTreeMap<AttemptOutputKey, SpeculativeAttempt>,
    pub(super) order: Vec<AttemptOutputKey>,
    pub(super) finalized: BTreeSet<AttemptOutputKey>,
}

impl SpeculativeState {
    /// The newest in-flight attempt's visible text, for the live streaming row.
    pub(super) fn visible_text(&self) -> Option<&str> {
        self.order.iter().rev().find_map(|key| {
            self.attempts
                .get(key)
                .map(|output| output.visible_text.as_str())
                .filter(|text| !text.is_empty())
        })
    }

    pub(super) fn in_flight(&self) -> usize {
        self.attempts.len()
    }

    pub(super) fn clear(&mut self) {
        self.attempts.clear();
        self.order.clear();
        self.finalized.clear();
    }
}

/// One delegated child's conversation, kept by the client on its behalf.
///
/// `live` records whether this came from the child's own event stream. A child
/// recovered from a durable record has a panel row and no stream, and the
/// difference decides who narrates its answer — the stream that produced it,
/// or the parent's completion event standing in for it.
#[derive(Debug, Default)]
pub(super) struct Conversation {
    pub(super) transcript: Transcript,
    pub(super) speculative: SpeculativeState,
    pub(super) live: bool,
}

impl Conversation {
    pub(super) fn as_mut(&mut self) -> ConversationMut<'_> {
        ConversationMut {
            transcript: &mut self.transcript,
            speculative: &mut self.speculative,
        }
    }
}

/// A borrowed conversation: whichever transcript and speculative buffer the
/// event being folded belongs to.
pub(super) struct ConversationMut<'a> {
    pub(super) transcript: &'a mut Transcript,
    pub(super) speculative: &'a mut SpeculativeState,
}

impl ConversationMut<'_> {
    /// Folds one event into this conversation, reporting whether it was the
    /// conversation's business at all.
    ///
    /// The caller keeps whatever else the event means to it. `TurnCompleted`
    /// closes the open block here *and* stops the root's spinner there; both
    /// are true, and neither belongs in the other's code.
    pub(super) fn apply(&mut self, event: &RuntimeEvent) -> bool {
        match event {
            RuntimeEvent::ProviderAttemptStarted {
                request, attempt, ..
            } => self.begin_attempt(request, attempt),
            RuntimeEvent::TextDelta {
                request,
                attempt,
                text,
            } => {
                self.begin_attempt(request, attempt);
                if let Some(output) = self.attempt_mut(request, attempt) {
                    output.push_text(text);
                }
            }
            RuntimeEvent::ReasoningDelta {
                request,
                attempt,
                text,
                redacted,
            } => {
                self.begin_attempt(request, attempt);
                if let Some(output) = self.attempt_mut(request, attempt) {
                    output.push_reasoning(text, *redacted);
                }
            }
            RuntimeEvent::ProviderAttemptOutputCommitted { request, attempt } => {
                self.finish_attempt(request, attempt, true);
            }
            RuntimeEvent::ProviderAttemptOutputDiscarded { request, attempt } => {
                self.finish_attempt(request, attempt, false);
            }
            // A harness turn runs on an installed coding agent, so its
            // output arrives already committed: there is no provider attempt
            // to speculate over and nothing to retry away.
            RuntimeEvent::ExternalText { text } => {
                self.transcript.push_text_delta(text);
            }
            RuntimeEvent::ExternalReasoning { text } => {
                // No provider signed it, so it is never marked redacted.
                self.transcript.push_reasoning_delta(text, false);
            }
            RuntimeEvent::ExternalToolInvoked { id, name } => {
                self.transcript.push_external_tool_call(id.as_str(), name);
            }
            RuntimeEvent::ExternalToolCompleted { id, ok } => {
                self.transcript.complete_tool_call(
                    id.as_str(),
                    if *ok {
                        ToolStatus::Ok
                    } else {
                        ToolStatus::Failed
                    },
                );
            }
            RuntimeEvent::ToolCallRequested {
                call,
                name,
                argument_keys,
                arguments,
                ..
            } => {
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
                self.transcript.complete_tool_call(call.as_str(), status);
            }
            RuntimeEvent::TurnStarted | RuntimeEvent::InternalTurnStarted { .. } => {
                self.discard_orphaned("next turn start");
            }
            RuntimeEvent::TurnCompleted { .. } => {
                // A valid stream has already committed or discarded every
                // attempt. A gap, corrupt journal, or incompatible producer
                // must fail closed: orphan text cannot become visible while
                // idle or leak into the next turn.
                self.discard_orphaned("turn completion");
                self.transcript.close_open();
            }
            RuntimeEvent::SessionShutdown => {
                self.discard_orphaned("session shutdown");
                self.transcript.close_open();
            }
            RuntimeEvent::Error { error } => self.transcript.push_error(error.to_string()),
            RuntimeEvent::Downgrade { capability, detail } => self
                .transcript
                .push_notice("downgrade", format!("{capability}: {detail}")),
            RuntimeEvent::LimitReached { limit } => self
                .transcript
                .push_notice("limit", format!("{limit:?} reached")),
            _ => return false,
        }
        true
    }

    /// Resolves every still-running tool row to `status`.
    ///
    /// A conversation whose stream ended mid-call has rows nobody will ever
    /// report an outcome for. Leaving them `running` would claim work that
    /// stopped is still in flight.
    pub(super) fn settle(&mut self, status: ToolStatus) {
        self.transcript.settle_running_tool_calls(status);
        self.transcript.close_open();
    }

    fn attempt_mut(
        &mut self,
        request: &RequestId,
        attempt: &AttemptId,
    ) -> Option<&mut SpeculativeAttempt> {
        self.speculative
            .attempts
            .get_mut(&AttemptOutputKey::new(request, attempt))
    }

    fn begin_attempt(&mut self, request: &RequestId, attempt: &AttemptId) {
        let key = AttemptOutputKey::new(request, attempt);
        if self.speculative.finalized.contains(&key) {
            self.transcript.push_error(format!(
                "provider attempt {attempt} for request {request} restarted after its output terminal"
            ));
            return;
        }
        if !self.speculative.attempts.contains_key(&key) {
            self.speculative.order.push(key.clone());
            self.speculative
                .attempts
                .insert(key, SpeculativeAttempt::default());
        }
    }

    fn finish_attempt(&mut self, request: &RequestId, attempt: &AttemptId, commit: bool) {
        let key = AttemptOutputKey::new(request, attempt);
        let Some(output) = self.speculative.attempts.remove(&key) else {
            if !self.speculative.finalized.contains(&key) {
                self.transcript.push_error(format!(
                    "provider attempt {attempt} for request {request} ended without a start"
                ));
                self.speculative.finalized.insert(key);
            }
            return;
        };
        self.speculative.order.retain(|candidate| candidate != &key);
        if !self.speculative.finalized.insert(key) {
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

    fn discard_orphaned(&mut self, boundary: &str) {
        let orphaned = self.speculative.attempts.len();
        if orphaned == 0 {
            return;
        }
        self.speculative.attempts.clear();
        self.speculative.order.clear();
        self.transcript.push_notice(
            "integrity",
            format!(
                "discarded {orphaned} unterminated speculative provider attempt(s) at {boundary}"
            ),
        );
    }
}
