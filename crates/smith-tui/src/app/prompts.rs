//! Prompt ownership, ordering, and expiry.

use std::collections::VecDeque;
use std::sync::Arc;

use agent_runtime_core::clock::{Clock, SystemClock, Timestamp};
use smith_client::NoticeKind;
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
    /// Opens through the shared prompt queue and transient-overlay policy.
    /// Returns false when a transient request cannot take focus.
    pub fn open_overlay(&mut self, overlay: Overlay) -> bool {
        let prompt = match overlay {
            Overlay::Approval { prompt, review } => PendingPrompt::Approval(prompt, review),
            Overlay::Questionnaire { state } => PendingPrompt::Questionnaire(state),
            Overlay::Confirm(dialog) => PendingPrompt::Confirm(dialog),
            transient => {
                if self.overlay.as_ref().is_some_and(Overlay::is_prompt)
                    || !self.pending_prompts.is_empty()
                {
                    self.push_notice(
                        NoticeKind::OverlayBlocked,
                        "answer the pending prompt before opening another panel",
                    );
                    return false;
                }
                self.overlay = Some(transient);
                return true;
            }
        };
        self.pending_prompts.push_back(prompt);
        if self.overlay.as_ref().is_some_and(Overlay::is_prompt) {
            return true;
        }
        let Some(prompt) = self.pending_prompts.pop_front() else {
            return true;
        };
        let overlay = match prompt {
            PendingPrompt::Approval(prompt, review) => {
                self.prompt_input_guard.start();
                self.approval_scroll = 0;
                self.approval_scroll_limit = 0;
                Overlay::Approval { prompt, review }
            }
            PendingPrompt::Questionnaire(state) => Overlay::Questionnaire { state },
            PendingPrompt::Confirm(dialog) => {
                self.prompt_input_guard.start();
                Overlay::Confirm(dialog)
            }
        };
        self.overlay = Some(overlay);
        true
    }

    /// Shows a rotation offer without replacing an earlier prompt.
    pub fn present_rotation(&mut self, prompt: smith_host::rotation::RotationPrompt) {
        let content =
            crate::accounts::rotation_prompt_body(prompt.request(), crate::accounts::now_ms());
        let body = content.lines().map(str::to_owned).collect();
        let mut dialog = ConfirmDialog::new(
            "switch provider account",
            crate::theme::Tone::Warning,
            body,
            "switch account and resend",
            ConfirmOutcome::SwitchAccount,
            ConfirmOutcome::StayAccount,
        );
        dialog.accept_tone = crate::theme::Tone::Accent;
        dialog.cancel_label = "stay".to_owned();
        dialog.hint = if prompt.request().eligible.len() > 1 {
            "y switch and resend · 1-9 choose account · n/esc stay"
        } else {
            "y switch and resend · n/esc stay"
        }
        .to_owned();
        dialog.rotation = Some(Box::new(prompt));
        self.open_overlay(Overlay::Confirm(dialog));
    }

    /// Presents an approval request.
    ///
    /// The diff is derived once here instead of on every redraw.
    pub fn present_approval(&mut self, prompt: ApprovalPrompt) {
        if prompt.deadline().is_expired(&SystemClock) {
            prompt.time_out();
            self.transcript.push_notice(
                NoticeKind::Approval,
                "approval timed out before it could be presented",
            );
            return;
        }
        self.set_tool_approval_status(
            prompt.prepared().call_id().as_str(),
            ToolStatus::WaitingForApproval,
        );
        let review = EditReview::from_call(prompt.tool(), prompt.prepared().arguments());
        self.open_overlay(Overlay::Approval {
            prompt: Box::new(prompt),
            review,
        });
    }

    /// Presents one authority-free questionnaire.
    pub fn present_questionnaire(&mut self, form: QuestionnaireForm) {
        let request_id = form.request_id.clone();
        if form.deadline.is_expired(&SystemClock) {
            self.questionnaire_resolutions
                .push_back((request_id, QuestionnaireResolution::TimedOut));
            self.transcript.push_notice(
                NoticeKind::Questionnaire,
                "question timed out before it could be presented",
            );
            return;
        }
        self.open_overlay(Overlay::Questionnaire {
            state: QuestionnaireState::new(form),
        });
    }

    /// Removes a runtime-closed questionnaire without manufacturing a second
    /// host response.
    ///
    /// The runtime calls its interaction broker's synchronous close hook when
    /// cancellation or its deadline wins, including when the broker future
    /// was dropped. The host adapter projects that close here so a visible or
    /// queued overlay cannot outlive the owning turn.
    pub fn dismiss_questionnaire(&mut self, request_id: &str) {
        if matches!(&self.overlay, Some(Overlay::Questionnaire { state })
            if state.form().request_id == request_id)
        {
            self.overlay = None;
        }
        self.pending_prompts.retain(|prompt| {
            !matches!(
                prompt,
                PendingPrompt::Questionnaire(state)
                    if state.form().request_id == request_id
            )
        });
        self.present_next_prompt();
    }

    pub(super) fn present_next_prompt(&mut self) {
        if self.overlay.as_ref().is_some_and(Overlay::is_prompt) {
            return;
        }
        if let Some(prompt) = self.pending_prompts.pop_front() {
            let overlay = match prompt {
                PendingPrompt::Approval(prompt, review) => Overlay::Approval { prompt, review },
                PendingPrompt::Questionnaire(state) => Overlay::Questionnaire { state },
                PendingPrompt::Confirm(dialog) => Overlay::Confirm(dialog),
            };
            // Keep the tail queued while the already-oldest prompt opens.
            let tail = std::mem::take(&mut self.pending_prompts);
            self.open_overlay(overlay);
            self.pending_prompts = tail;
        }
    }

    pub(super) fn expire_prompts(&mut self) {
        let mut expired_approvals = 0_usize;
        let mut expired_questions = 0_usize;
        let expired = match &self.overlay {
            Some(Overlay::Approval { prompt, .. }) => prompt.deadline().is_expired(&SystemClock),
            Some(Overlay::Questionnaire { state }) => {
                state.form().deadline.is_expired(&SystemClock)
            }
            _ => false,
        };
        if expired {
            match self.overlay.take() {
                Some(Overlay::Approval { prompt, .. }) => {
                    prompt.time_out();
                    expired_approvals += 1;
                }
                Some(Overlay::Questionnaire { state }) => {
                    self.resolve_questionnaire(state, QuestionnaireResolution::TimedOut);
                    expired_questions += 1;
                }
                _ => unreachable!("only runtime prompts expire"),
            }
        }

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
                PendingPrompt::Confirm(dialog) => waiting.push_back(PendingPrompt::Confirm(dialog)),
            }
        }
        self.pending_prompts = waiting;

        if expired_approvals > 0 {
            self.transcript.push_notice(
                NoticeKind::Approval,
                format!(
                    "timed out {}",
                    smith_client::plural(
                        expired_approvals,
                        "pending approval request",
                        "pending approval requests"
                    )
                ),
            );
        }
        if expired_questions > 0 {
            self.transcript.push_notice(
                NoticeKind::Questionnaire,
                format!(
                    "timed out {}",
                    smith_client::plural(
                        expired_questions,
                        "pending question request",
                        "pending question requests"
                    )
                ),
            );
        }
        self.present_next_prompt();
    }

    pub(super) fn tool_waiting_for_approval(&self, call_id: &str) -> bool {
        let matches = |prompt: &ApprovalPrompt| prompt.prepared().call_id().as_str() == call_id;
        let visible = match &self.overlay {
            Some(Overlay::Approval { prompt, .. }) => matches(prompt),
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
                        NoticeKind::Approval,
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
                        .push_notice(NoticeKind::Approval, format!("{tool} denied"));
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
        self.transcript
            .push_notice(NoticeKind::Questionnaire, notice);
    }
}
