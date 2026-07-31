//! Bridge between Agent Runtime's interaction protocol and Smith's pure TUI
//! questionnaire state.

use std::collections::BTreeMap;

use agent_runtime_core::ids::{ChoiceId, QuestionId};
use agent_runtime_core::interaction::{InteractionRequest, InteractionSensitivity, QuestionAnswer};
use smith_host::{InteractionNotice, InteractionPrompt, InteractionRequests};
use smith_tui::App;
use smith_tui::questionnaire::{
    QuestionnaireAnswerValue, QuestionnaireChoice, QuestionnaireForm, QuestionnaireQuestion,
    QuestionnaireResolution,
};

/// Terminal-owned prompt lifecycle and responder registry.
pub(crate) struct InteractionSurface {
    requests: Option<InteractionRequests>,
    prompts: BTreeMap<String, InteractionPrompt>,
    restored_request: Option<String>,
}

impl InteractionSurface {
    pub(crate) fn new(
        requests: Option<InteractionRequests>,
        restored_request: Option<String>,
    ) -> Self {
        Self {
            requests,
            prompts: BTreeMap::new(),
            restored_request,
        }
    }

    pub(crate) async fn next_notice(&mut self) -> Option<InteractionNotice> {
        match &mut self.requests {
            Some(requests) => requests.recv().await,
            None => std::future::pending().await,
        }
    }

    pub(crate) fn close_receiver(&mut self) {
        self.requests = None;
    }

    pub(crate) fn apply_notice(&mut self, app: &mut App, notice: InteractionNotice) {
        match notice {
            InteractionNotice::Present(prompt) => {
                let request_id = prompt.request().id().as_str().to_owned();
                if self.prompts.contains_key(&request_id) {
                    let _ = prompt
                        .unavailable("duplicate questionnaire identity reached the Smith surface");
                    app.transcript
                        .push_error("duplicate questionnaire identity was rejected");
                    return;
                }
                let restored = self.restored_request.as_deref() == Some(request_id.as_str());
                match form_from_request(prompt.request(), restored) {
                    Ok(form) => {
                        self.prompts.insert(request_id, prompt);
                        app.present_questionnaire(form);
                    }
                    Err(error) => {
                        let _ = prompt.unavailable(
                            "questionnaire cannot be represented by this Smith client",
                        );
                        app.transcript.push_error(error);
                    }
                }
            }
            InteractionNotice::Close { request_id, .. } => {
                let request_id = request_id.as_str();
                self.prompts.remove(request_id);
                app.dismiss_questionnaire(request_id);
            }
        }
    }

    pub(crate) fn drain_answers(&mut self, app: &mut App) {
        while let Some((request_id, resolution)) = app.take_questionnaire_resolution() {
            let Some(prompt) = self.prompts.remove(&request_id) else {
                // A synchronous runtime close won the race. The runtime owns
                // that terminal outcome; a late key cannot answer it again.
                continue;
            };
            let result = match resolution {
                QuestionnaireResolution::Submitted(answers) => prompt.answer(
                    answers
                        .into_iter()
                        .map(|answer| match answer.value {
                            QuestionnaireAnswerValue::Choice(choice) => QuestionAnswer::choice(
                                QuestionId::new(answer.question_id),
                                ChoiceId::new(choice),
                            ),
                            QuestionnaireAnswerValue::FreeForm(value) => QuestionAnswer::free_form(
                                QuestionId::new(answer.question_id),
                                value,
                            ),
                        })
                        .collect(),
                ),
                QuestionnaireResolution::Declined => prompt.decline(),
                QuestionnaireResolution::Cancelled => prompt.cancel(),
                QuestionnaireResolution::TimedOut => prompt.time_out(),
            };
            if let Err(error) = result {
                app.transcript
                    .push_error(format!("questionnaire response was rejected: {error}"));
            }
        }
    }
}

fn form_from_request(
    request: &InteractionRequest,
    restored: bool,
) -> Result<QuestionnaireForm, String> {
    let sensitive = request.sensitivity() == InteractionSensitivity::Sensitive;
    let questions = request
        .questionnaire_payload()
        .questions()
        .iter()
        .map(|question| {
            let choices = question
                .choices()
                .iter()
                .map(|choice| {
                    let projected = QuestionnaireChoice::new(choice.id().as_str(), choice.label());
                    match choice.description() {
                        Some(description) => projected.with_description(description),
                        None => projected,
                    }
                })
                .collect();
            let projected = QuestionnaireQuestion::new(
                question.id().as_str(),
                question.header(),
                question.prompt(),
                choices,
            )
            .with_sensitivity(sensitive);
            if question.allows_free_form() {
                projected.with_free_form(sensitive)
            } else {
                projected
            }
        })
        .collect();
    QuestionnaireForm::new(request.id().as_str(), questions, request.deadline())
        .map(|form| form.restored(restored))
        .map_err(|error| format!("invalid questionnaire presentation: {error}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_runtime_core::clock::Deadline;
    use agent_runtime_core::ids::{InteractionRequestId, SessionId, ToolCallId, TurnId};
    use agent_runtime_core::interaction::{
        Choice, InteractionBroker, InteractionOrigin, InteractionOutcomeKind, Question,
        Questionnaire,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use smith_host::InteractiveInteraction;
    use smith_tui::Overlay;

    use super::*;

    fn request(id: &str, sensitivity: InteractionSensitivity) -> InteractionRequest {
        InteractionRequest::questionnaire(
            InteractionRequestId::new(id),
            InteractionOrigin::new(
                SessionId::new("session-1"),
                TurnId::new("turn-1"),
                ToolCallId::new("call-1"),
            ),
            Questionnaire::new(vec![
                Question::new(
                    QuestionId::new("direction"),
                    "Direction",
                    "Which direction?",
                )
                .with_choices(vec![
                    Choice::new(ChoiceId::new("small"), "Small")
                        .with_description("Keep the scope narrow"),
                ])
                .allow_free_form(true),
            ])
            .unwrap(),
            Deadline::never(),
            sensitivity,
        )
        .unwrap()
    }

    #[test]
    fn projection_preserves_stable_ids_bounds_and_restored_state() {
        let form = form_from_request(
            &request("interaction-1", InteractionSensitivity::Public),
            true,
        )
        .unwrap();
        assert_eq!(form.request_id, "interaction-1");
        assert!(form.restored);
        assert_eq!(form.questions[0].id, "direction");
        assert_eq!(form.questions[0].choices[0].id, "small");
        assert_eq!(
            form.questions[0].choices[0].description.as_deref(),
            Some("Keep the scope narrow")
        );
    }

    #[test]
    fn sensitive_request_masks_free_form_at_the_tui_boundary() {
        let form = form_from_request(
            &request("interaction-1", InteractionSensitivity::Sensitive),
            false,
        )
        .unwrap();
        assert!(form.questions[0].sensitive);
    }

    #[test]
    fn sensitive_choice_only_payload_is_classified_and_debug_safe() {
        const QUESTION_ID: &str = "private-question-id";
        const PROMPT: &str = "Private acquisition codename?";
        const CHOICE_ID: &str = "private-choice-id";
        const LABEL: &str = "Unreleased Project";
        const DESCRIPTION: &str = "Confidential tradeoff";
        let request = InteractionRequest::questionnaire(
            InteractionRequestId::new("interaction-sensitive-choice"),
            InteractionOrigin::new(
                SessionId::new("session-1"),
                TurnId::new("turn-1"),
                ToolCallId::new("call-1"),
            ),
            Questionnaire::new(vec![
                Question::new(QuestionId::new(QUESTION_ID), "Private", PROMPT).with_choices(vec![
                    Choice::new(ChoiceId::new(CHOICE_ID), LABEL).with_description(DESCRIPTION),
                ]),
            ])
            .unwrap(),
            Deadline::never(),
            InteractionSensitivity::Sensitive,
        )
        .unwrap();
        let form = form_from_request(&request, false).expect("projected form");
        assert!(
            form.questions[0].sensitive,
            "choice-only questions must inherit request sensitivity"
        );
        let resolution = QuestionnaireResolution::Submitted(vec![
            smith_tui::questionnaire::QuestionnaireAnswer {
                question_id: QUESTION_ID.to_owned(),
                value: QuestionnaireAnswerValue::Choice(CHOICE_ID.to_owned()),
                sensitive: form.questions[0].sensitive,
            },
        ]);
        let debug = format!("{form:?}\n{resolution:?}");
        for protected in [QUESTION_ID, PROMPT, CHOICE_ID, LABEL, DESCRIPTION] {
            assert!(
                !debug.contains(protected),
                "debug leaked `{protected}`: {debug}"
            );
        }
    }

    #[tokio::test]
    async fn runtime_close_removes_visible_and_queued_forms_by_identity() {
        let (broker, requests) = InteractiveInteraction::new();
        let broker = Arc::new(broker);
        let mut surface = InteractionSurface::new(Some(requests), None);
        let mut app = App::new("model", "project");
        let first = request("first", InteractionSensitivity::Public);
        let second = request("second", InteractionSensitivity::Public);
        let first_task = {
            let broker = Arc::clone(&broker);
            tokio::spawn(async move { broker.interact(&first).await })
        };
        let second_task = {
            let broker = Arc::clone(&broker);
            tokio::spawn(async move { broker.interact(&second).await })
        };
        for _ in 0..2 {
            let notice = surface.next_notice().await.expect("presentation");
            surface.apply_notice(&mut app, notice);
        }
        assert_eq!(app.pending_questionnaire_count(), 2);

        broker.close(
            &InteractionRequestId::new("first"),
            InteractionOutcomeKind::TimedOut,
        );
        let notice = surface.next_notice().await.expect("first close");
        surface.apply_notice(&mut app, notice);
        assert!(matches!(
            &app.overlay,
            Some(Overlay::Questionnaire { state })
                if state.form().request_id == "second"
        ));

        broker.close(
            &InteractionRequestId::new("second"),
            InteractionOutcomeKind::Cancelled,
        );
        let notice = surface.next_notice().await.expect("second close");
        surface.apply_notice(&mut app, notice);
        assert!(app.overlay.is_none());
        assert_eq!(app.pending_questionnaire_count(), 0);
        assert!(app.take_questionnaire_resolution().is_none());
        assert_eq!(
            first_task.await.unwrap().outcome_kind(),
            InteractionOutcomeKind::TimedOut
        );
        assert_eq!(
            second_task.await.unwrap().outcome_kind(),
            InteractionOutcomeKind::Cancelled
        );
    }

    #[tokio::test]
    async fn staged_tui_answer_returns_typed_information_without_approval() {
        let (broker, requests) = InteractiveInteraction::new();
        let broker = Arc::new(broker);
        let mut surface = InteractionSurface::new(Some(requests), None);
        let mut app = App::new("model", "project");
        let request = request("answer", InteractionSensitivity::Public);
        let task = {
            let broker = Arc::clone(&broker);
            tokio::spawn(async move { broker.interact(&request).await })
        };
        let notice = surface.next_notice().await.expect("presentation");
        surface.apply_notice(&mut app, notice);

        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        surface.drain_answers(&mut app);

        let response = task.await.unwrap();
        assert_eq!(response.outcome_kind(), InteractionOutcomeKind::Answered);
        let answers = response.answers().expect("typed answers");
        assert_eq!(answers[0].question_id().as_str(), "direction");
        assert_eq!(answers[0].choice_id().map(|id| id.as_str()), Some("small"));
    }
}
