//! Approval and questionnaire ownership, ordering, and expiry.

use std::collections::VecDeque;
use std::sync::Arc;

use agent_runtime_core::clock::{Clock, SystemClock, Timestamp};
use smith_host::approval::{ApprovalPrompt, PromptScope};

use crate::diff::EditReview;
use crate::questionnaire::{QuestionnaireForm, QuestionnaireResolution, QuestionnaireState};
use crate::transcript::ToolStatus;

use super::state::*;

/// Input must settle after a consequential prompt takes focus.
#[derive(Debug)]
pub(super) struct PromptInputGuard {
    pub(super) clock: Arc<dyn Clock>,
    pub(super) quiet_until: Option<Timestamp>,
}

impl Default for PromptInputGuard {
    fn default() -> Self {
        Self {
            clock: Arc::new(SystemClock),
            quiet_until: None,
        }
    }
}

impl PromptInputGuard {
    pub(super) fn start(&mut self) {
        self.quiet_until = Some(self.clock.now().plus_millis(500));
    }

    pub(super) fn ignore_key(&mut self) -> bool {
        let Some(quiet_until) = self.quiet_until else {
            return false;
        };
        let now = self.clock.now();
        if now < quiet_until {
            self.quiet_until = Some(now.plus_millis(500));
            true
        } else {
            self.quiet_until = None;
            false
        }
    }
}

impl App {
    /// Shows a rotation offer as a modal the user answers.
    ///
    /// Rendered eagerly rather than queued behind other prompts: the runtime
    /// is blocked on the answer, and the offer's whole value is that the user
    /// decides before the turn is resent uncached.
    pub fn present_rotation(&mut self, prompt: smith_host::rotation::RotationPrompt) {
        let content =
            crate::accounts::rotation_prompt_body(prompt.request(), crate::accounts::now_ms());
        self.overlay = Some(Overlay::RotationConfirm {
            prompt: Box::new(prompt),
            content,
        });
        self.prompt_input_guard.start();
    }

    /// Presents an approval request.
    ///
    /// The diff is derived once here instead of on every redraw.
    pub fn present_approval(&mut self, prompt: ApprovalPrompt) {
        if prompt.deadline().is_expired(&SystemClock) {
            prompt.time_out();
            self.transcript.push_notice(
                "approval",
                "approval timed out before it could be presented",
            );
            return;
        }
        self.set_tool_approval_status(
            prompt.prepared().call_id().as_str(),
            ToolStatus::WaitingForApproval,
        );
        let review = EditReview::from_call(prompt.tool(), prompt.prepared().arguments());
        let approval = PendingPrompt::Approval(Box::new(prompt), review);
        if self.overlay.is_none() {
            self.show_prompt(approval);
        } else {
            self.pending_prompts.push_back(approval);
        }
    }

    /// Presents one authority-free questionnaire.
    pub fn present_questionnaire(&mut self, form: QuestionnaireForm) {
        let request_id = form.request_id.clone();
        if form.deadline.is_expired(&SystemClock) {
            self.questionnaire_resolutions
                .push_back((request_id, QuestionnaireResolution::TimedOut));
            self.transcript.push_notice(
                "questionnaire",
                "question timed out before it could be presented",
            );
            return;
        }
        let prompt = PendingPrompt::Questionnaire(QuestionnaireState::new(form));
        if self.overlay.is_none() {
            self.show_prompt(prompt);
        } else {
            self.pending_prompts.push_back(prompt);
        }
    }

    /// Removes a runtime-closed questionnaire without manufacturing a second
    /// host response.
    ///
    /// The runtime calls its interaction broker's synchronous close hook when
    /// cancellation or its deadline wins, including when the broker future
    /// was dropped. The host adapter projects that close here so a visible or
    /// queued overlay cannot outlive the owning turn.
    pub fn dismiss_questionnaire(&mut self, request_id: &str) {
        self.overlay = match self.overlay.take() {
            Some(Overlay::Questionnaire { state }) if state.form().request_id == request_id => None,
            Some(Overlay::ExitConfirm {
                approval,
                questionnaire: Some(state),
            }) if state.form().request_id == request_id => Some(Overlay::ExitConfirm {
                approval,
                questionnaire: None,
            }),
            other => other,
        };
        self.pending_prompts.retain(|prompt| {
            !matches!(
                prompt,
                PendingPrompt::Questionnaire(state)
                    if state.form().request_id == request_id
            )
        });
        self.present_next_prompt();
    }

    pub(super) fn show_prompt(&mut self, prompt: PendingPrompt) {
        if matches!(prompt, PendingPrompt::Approval(..)) {
            self.prompt_input_guard.start();
            self.approval_scroll = 0;
            self.approval_scroll_limit = 0;
        }
        self.overlay = Some(match prompt {
            PendingPrompt::Approval(prompt, review) => Overlay::Approval { prompt, review },
            PendingPrompt::Questionnaire(state) => Overlay::Questionnaire { state },
        });
    }

    pub(super) fn present_next_prompt(&mut self) {
        if self.overlay.is_some() {
            return;
        }
        if let Some(prompt) = self.pending_prompts.pop_front() {
            self.show_prompt(prompt);
        }
    }

    pub(super) fn expire_prompts(&mut self) {
        let mut expired_approvals = 0_usize;
        let mut expired_questions = 0_usize;
        self.overlay = match self.overlay.take() {
            Some(Overlay::Approval { prompt, review }) => {
                if prompt.deadline().is_expired(&SystemClock) {
                    prompt.time_out();
                    expired_approvals += 1;
                    None
                } else {
                    Some(Overlay::Approval { prompt, review })
                }
            }
            Some(Overlay::Questionnaire { state }) => {
                if state.form().deadline.is_expired(&SystemClock) {
                    self.resolve_questionnaire(state, QuestionnaireResolution::TimedOut);
                    expired_questions += 1;
                    None
                } else {
                    Some(Overlay::Questionnaire { state })
                }
            }
            Some(Overlay::ExitConfirm {
                approval: Some((prompt, review)),
                questionnaire,
            }) => {
                if prompt.deadline().is_expired(&SystemClock) {
                    prompt.time_out();
                    expired_approvals += 1;
                    Some(Overlay::ExitConfirm {
                        approval: None,
                        questionnaire,
                    })
                } else {
                    Some(Overlay::ExitConfirm {
                        approval: Some((prompt, review)),
                        questionnaire,
                    })
                }
            }
            Some(Overlay::ExitConfirm {
                approval,
                questionnaire: Some(state),
            }) => {
                if state.form().deadline.is_expired(&SystemClock) {
                    self.resolve_questionnaire(state, QuestionnaireResolution::TimedOut);
                    expired_questions += 1;
                    Some(Overlay::ExitConfirm {
                        approval,
                        questionnaire: None,
                    })
                } else {
                    Some(Overlay::ExitConfirm {
                        approval,
                        questionnaire: Some(state),
                    })
                }
            }
            other => other,
        };

        let mut waiting = VecDeque::with_capacity(self.pending_prompts.len());
        while let Some(prompt) = self.pending_prompts.pop_front() {
            match prompt {
                PendingPrompt::Approval(prompt, review) => {
                    if prompt.deadline().is_expired(&SystemClock) {
                        prompt.time_out();
                        expired_approvals += 1;
                    } else {
                        waiting.push_back(PendingPrompt::Approval(prompt, review));
                    }
                }
                PendingPrompt::Questionnaire(state) => {
                    if state.form().deadline.is_expired(&SystemClock) {
                        self.resolve_questionnaire(state, QuestionnaireResolution::TimedOut);
                        expired_questions += 1;
                    } else {
                        waiting.push_back(PendingPrompt::Questionnaire(state));
                    }
                }
            }
        }
        self.pending_prompts = waiting;

        if expired_approvals > 0 {
            self.transcript.push_notice(
                "approval",
                format!("timed out {expired_approvals} pending approval request(s)"),
            );
        }
        if expired_questions > 0 {
            self.transcript.push_notice(
                "questionnaire",
                format!("timed out {expired_questions} pending question request(s)"),
            );
        }
        self.present_next_prompt();
    }

    pub(super) fn tool_waiting_for_approval(&self, call_id: &str) -> bool {
        let matches = |prompt: &ApprovalPrompt| prompt.prepared().call_id().as_str() == call_id;
        let visible = match &self.overlay {
            Some(Overlay::Approval { prompt, .. }) => matches(prompt),
            Some(Overlay::ExitConfirm {
                approval: Some((prompt, _)),
                ..
            }) => matches(prompt),
            _ => false,
        };
        visible
            || self.pending_prompts.iter().any(
                |prompt| matches!(prompt, PendingPrompt::Approval(prompt, _) if matches(prompt)),
            )
    }

    fn set_tool_approval_status(&mut self, call_id: &str, status: ToolStatus) -> bool {
        let represented = self.transcript.complete_tool_call(call_id, status);
        if let Some(work) = &mut self.live_turn.work
            && let Some((_, work_status, started_at)) = work.tools.get_mut(call_id)
        {
            *work_status = status;
            *started_at = (status == ToolStatus::Running).then(std::time::Instant::now);
        }
        represented
    }

    pub(super) fn answer_approval(&mut self, allow: Option<PromptScope>) {
        let Some(Overlay::Approval { prompt, .. }) = self.overlay.take() else {
            return;
        };
        let tool = prompt.tool().to_owned();
        let call_id = prompt.prepared().call_id().as_str().to_owned();
        match allow {
            Some(scope) => {
                self.set_tool_approval_status(&call_id, ToolStatus::Running);
                prompt.allow(scope);
                if scope == PromptScope::Session {
                    self.transcript.push_notice(
                        "approval",
                        format!("{tool} allowed for this target for the session"),
                    );
                }
            }
            None => {
                prompt.deny("the user declined");
                if self.set_tool_approval_status(&call_id, ToolStatus::Denied) {
                    self.transcript
                        .set_tool_result_preview(&call_id, "approval declined: the user declined");
                } else {
                    self.transcript
                        .push_notice("approval", format!("{tool} denied"));
                }
            }
        }
        self.present_next_prompt();
    }

    pub(super) fn resolve_questionnaire(
        &mut self,
        state: QuestionnaireState,
        resolution: QuestionnaireResolution,
    ) {
        let request_id = state.form().request_id.clone();
        let notice = match &resolution {
            QuestionnaireResolution::Submitted(_) => "submitted",
            QuestionnaireResolution::Declined => "declined",
            QuestionnaireResolution::Cancelled => "cancelled",
            QuestionnaireResolution::TimedOut => "timed out",
        };
        self.questionnaire_resolutions
            .push_back((request_id, resolution));
        self.transcript.push_notice("questionnaire", notice);
    }
}
