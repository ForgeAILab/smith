//! Approval and questionnaire ownership, ordering, and expiry.

use std::collections::VecDeque;

use agent_runtime_core::clock::SystemClock;
use smith_host::approval::{ApprovalPrompt, PromptScope};

use crate::diff::EditReview;
use crate::questionnaire::{QuestionnaireForm, QuestionnaireResolution, QuestionnaireState};
use crate::transcript::ToolStatus;

use super::state::*;

impl App {
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

    pub(super) fn answer_approval(&mut self, allow: Option<PromptScope>) {
        let Some(Overlay::Approval { prompt, .. }) = self.overlay.take() else {
            return;
        };
        let tool = prompt.tool().to_owned();
        match allow {
            Some(scope) => {
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
                self.transcript
                    .push_notice("approval", format!("{tool} denied"));
                self.transcript
                    .complete_tool_call_by_name(&tool, ToolStatus::Denied);
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
