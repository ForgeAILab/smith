//! Host adapters for authority-free runtime interaction.
//!
//! Interaction is deliberately separate from approval. These adapters move a
//! validated questionnaire to a presentation surface and return task
//! information only; they never hold or manufacture a permission grant.

use std::collections::{BTreeMap, btree_map::Entry};
use std::fmt;
use std::sync::{Arc, Mutex};

use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::InteractionRequestId;
use agent_runtime_core::interaction::{
    InteractionBroker, InteractionOutcomeKind, InteractionReadiness, InteractionRequest,
    InteractionResponse, InteractionSensitivity, QuestionAnswer,
};
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

/// Redaction-safe evidence that an unattended run reached a questionnaire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionRequired {
    /// Stable request identity.
    pub request_id: String,
    /// Number of required questions, without their sensitive content.
    pub question_count: usize,
}

/// Host-owned registry for literals that must be removed before default
/// persistence.
///
/// The interactive broker uses this only for free-form values from a request
/// explicitly marked sensitive. The registry is intentionally value-only: it
/// receives neither the questionnaire nor its request identity.
pub trait SensitiveValueSink: Send + Sync + fmt::Debug {
    /// Registers one exact sensitive literal before it enters the live turn.
    fn register_sensitive_value(&self, value: &str);
}

/// A non-interactive broker that fails immediately and records why the run
/// could not continue.
#[derive(Debug, Default)]
pub struct HeadlessInteraction {
    required: Mutex<Option<InteractionRequired>>,
}

impl HeadlessInteraction {
    /// Creates an empty unattended interaction recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// First questionnaire that reached this host, with prompt/answer content
    /// omitted.
    pub fn required(&self) -> Option<InteractionRequired> {
        self.required
            .lock()
            .expect("headless interaction state poisoned")
            .clone()
    }
}

#[async_trait]
impl InteractionBroker for HeadlessInteraction {
    fn readiness(&self) -> InteractionReadiness {
        InteractionReadiness::Unavailable
    }

    async fn interact(&self, request: &InteractionRequest) -> InteractionResponse {
        let required = InteractionRequired {
            request_id: request.id().as_str().to_owned(),
            question_count: request.questionnaire_payload().questions().len(),
        };
        let mut slot = self
            .required
            .lock()
            .expect("headless interaction state poisoned");
        if slot.is_none() {
            *slot = Some(required);
        }
        InteractionResponse::unavailable(request.id().clone(), "headless interaction required")
    }

    fn close(&self, _request_id: &InteractionRequestId, _outcome: InteractionOutcomeKind) {}
}

/// One lifecycle notification delivered to an interactive surface.
#[derive(Debug)]
pub enum InteractionNotice {
    /// Present this exact request.
    Present(InteractionPrompt),
    /// Remove a queued or visible prompt. The runtime has already selected the
    /// terminal outcome, so the surface must not answer it again.
    Close {
        /// Request to remove.
        request_id: InteractionRequestId,
        /// Metadata-only outcome selected by the runtime.
        outcome: InteractionOutcomeKind,
    },
}

/// One exact questionnaire handed to the user-facing surface.
pub struct InteractionPrompt {
    request: InteractionRequest,
    shared: Arc<InteractiveState>,
}

impl InteractionPrompt {
    /// Exact validated request.
    pub fn request(&self) -> &InteractionRequest {
        &self.request
    }

    /// Returns answers in the runtime protocol.
    pub fn answer(self, answers: Vec<QuestionAnswer>) -> Result<(), RuntimeError> {
        let request_id = self.request.id().clone();
        self.respond(InteractionResponse::answered(request_id, answers))
    }

    /// Explicitly declines the request.
    pub fn decline(self) -> Result<(), RuntimeError> {
        let request_id = self.request.id().clone();
        self.respond(InteractionResponse::declined(request_id))
    }

    /// Cancels the request from the presentation surface.
    pub fn cancel(self) -> Result<(), RuntimeError> {
        let request_id = self.request.id().clone();
        self.respond(InteractionResponse::cancelled(request_id))
    }

    /// Reports that the presentation deadline elapsed.
    pub fn time_out(self) -> Result<(), RuntimeError> {
        let request_id = self.request.id().clone();
        self.respond(InteractionResponse::timed_out(request_id))
    }

    /// Reports that the surface could not faithfully present the request.
    pub fn unavailable(self, reason: impl Into<String>) -> Result<(), RuntimeError> {
        let request_id = self.request.id().clone();
        self.respond(InteractionResponse::unavailable(request_id, reason))
    }

    fn respond(self, response: InteractionResponse) -> Result<(), RuntimeError> {
        response.validate_for(&self.request)?;
        if self.request.sensitivity() == InteractionSensitivity::Sensitive
            && let Some(sink) = &self.shared.sensitive_values
            && let Some(answers) = response.answers()
        {
            for answer in answers {
                if let Some(value) = answer.free_form_value() {
                    sink.register_sensitive_value(value);
                }
            }
        }
        self.shared.resolve(response);
        Ok(())
    }
}

impl fmt::Debug for InteractionPrompt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InteractionPrompt")
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

/// Receiving half owned by the interactive presentation loop.
#[derive(Debug)]
pub struct InteractionRequests {
    rx: mpsc::UnboundedReceiver<InteractionNotice>,
    shared: Arc<InteractiveState>,
}

impl InteractionRequests {
    /// Waits for the next presentation or close notification.
    pub async fn recv(&mut self) -> Option<InteractionNotice> {
        self.rx.recv().await
    }

    /// Takes a pending notification without waiting.
    pub fn try_recv(&mut self) -> Option<InteractionNotice> {
        self.rx.try_recv().ok()
    }
}

impl Drop for InteractionRequests {
    fn drop(&mut self) {
        self.shared.cancel_all();
    }
}

/// Interactive broker paired with [`InteractionRequests`].
#[derive(Debug)]
pub struct InteractiveInteraction {
    shared: Arc<InteractiveState>,
}

impl InteractiveInteraction {
    /// Creates a broker and its presentation receiver.
    ///
    /// Runtime interaction is serial within a root turn and child interaction
    /// is disabled by default, so an unbounded lifecycle channel cannot be
    /// driven by arbitrary provider concurrency. The TUI independently bounds
    /// each form and its prompt queue.
    pub fn new() -> (Self, InteractionRequests) {
        Self::with_sensitive_values(None)
    }

    /// Creates a broker whose sensitive free-form values are registered with
    /// the host persistence redactor before delivery to the runtime.
    pub fn with_sensitive_value_sink(
        sink: Arc<dyn SensitiveValueSink>,
    ) -> (Self, InteractionRequests) {
        Self::with_sensitive_values(Some(sink))
    }

    fn with_sensitive_values(
        sensitive_values: Option<Arc<dyn SensitiveValueSink>>,
    ) -> (Self, InteractionRequests) {
        let (tx, rx) = mpsc::unbounded_channel();
        let shared = Arc::new(InteractiveState {
            tx,
            active: Mutex::new(BTreeMap::new()),
            sensitive_values,
        });
        (
            Self {
                shared: Arc::clone(&shared),
            },
            InteractionRequests { rx, shared },
        )
    }
}

#[async_trait]
impl InteractionBroker for InteractiveInteraction {
    fn readiness(&self) -> InteractionReadiness {
        if self.shared.tx.is_closed() {
            InteractionReadiness::Unavailable
        } else {
            InteractionReadiness::Ready
        }
    }

    async fn interact(&self, request: &InteractionRequest) -> InteractionResponse {
        if let Err(error) = request.validate() {
            return InteractionResponse::unavailable(
                request.id().clone(),
                format!("invalid interaction request: {}", error.message),
            );
        }
        if self.shared.tx.is_closed() {
            return InteractionResponse::unavailable(
                request.id().clone(),
                "no interaction surface is available",
            );
        }

        let (responder, answer) = oneshot::channel();
        {
            let mut active = self
                .shared
                .active
                .lock()
                .expect("interactive interaction state poisoned");
            match active.entry(request.id().clone()) {
                Entry::Vacant(slot) => {
                    slot.insert(responder);
                }
                Entry::Occupied(_) => {
                    return InteractionResponse::unavailable(
                        request.id().clone(),
                        "duplicate active interaction request",
                    );
                }
            }
        }
        let notice = InteractionNotice::Present(InteractionPrompt {
            request: request.clone(),
            shared: Arc::clone(&self.shared),
        });
        if self.shared.tx.send(notice).is_err() {
            self.shared.remove(request.id());
            return InteractionResponse::unavailable(
                request.id().clone(),
                "no interaction surface is available",
            );
        }

        answer
            .await
            .unwrap_or_else(|_| InteractionResponse::cancelled(request.id().clone()))
    }

    fn close(&self, request_id: &InteractionRequestId, outcome: InteractionOutcomeKind) {
        self.shared.close(request_id, outcome);
    }
}

#[derive(Debug)]
struct InteractiveState {
    tx: mpsc::UnboundedSender<InteractionNotice>,
    active: Mutex<BTreeMap<InteractionRequestId, oneshot::Sender<InteractionResponse>>>,
    sensitive_values: Option<Arc<dyn SensitiveValueSink>>,
}

impl InteractiveState {
    fn resolve(&self, response: InteractionResponse) -> bool {
        let responder = self
            .active
            .lock()
            .expect("interactive interaction state poisoned")
            .remove(response.request_id());
        if let Some(responder) = responder {
            let _ = responder.send(response);
            true
        } else {
            false
        }
    }

    fn remove(&self, request_id: &InteractionRequestId) {
        self.active
            .lock()
            .expect("interactive interaction state poisoned")
            .remove(request_id);
    }

    fn close(&self, request_id: &InteractionRequestId, outcome: InteractionOutcomeKind) {
        let response = match outcome {
            InteractionOutcomeKind::Answered => InteractionResponse::unavailable(
                request_id.clone(),
                "interaction closed before answer delivery",
            ),
            InteractionOutcomeKind::Declined => InteractionResponse::declined(request_id.clone()),
            InteractionOutcomeKind::TimedOut => InteractionResponse::timed_out(request_id.clone()),
            InteractionOutcomeKind::Cancelled => InteractionResponse::cancelled(request_id.clone()),
            InteractionOutcomeKind::Unavailable => {
                InteractionResponse::unavailable(request_id.clone(), "interaction host unavailable")
            }
        };
        if self.resolve(response) {
            let _ = self.tx.send(InteractionNotice::Close {
                request_id: request_id.clone(),
                outcome,
            });
        }
    }

    fn cancel_all(&self) {
        let active = std::mem::take(
            &mut *self
                .active
                .lock()
                .expect("interactive interaction state poisoned"),
        );
        for (request_id, responder) in active {
            let _ = responder.send(InteractionResponse::cancelled(request_id));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use agent_runtime_core::clock::Deadline;
    use agent_runtime_core::ids::{ChoiceId, QuestionId, SessionId, ToolCallId, TurnId};
    use agent_runtime_core::interaction::{
        Choice, InteractionOrigin, InteractionSensitivity, Question, Questionnaire,
    };

    use super::*;

    #[derive(Debug, Default)]
    struct RecordingSensitiveValues(Mutex<Vec<String>>);

    impl SensitiveValueSink for RecordingSensitiveValues {
        fn register_sensitive_value(&self, value: &str) {
            self.0.lock().unwrap().push(value.to_owned());
        }
    }

    fn request(id: &str) -> InteractionRequest {
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
                    Choice::new(ChoiceId::new("small"), "Small"),
                    Choice::new(ChoiceId::new("large"), "Large"),
                ]),
            ])
            .unwrap(),
            Deadline::never(),
            InteractionSensitivity::Public,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn answer_is_typed_and_close_after_delivery_is_idempotent() {
        let (broker, mut requests) = InteractiveInteraction::new();
        let expected = request("interaction-1");
        let task = tokio::spawn({
            let expected = expected.clone();
            async move { broker.interact(&expected).await }
        });

        let InteractionNotice::Present(prompt) = requests.recv().await.expect("presented request")
        else {
            panic!("expected a presentation");
        };
        prompt
            .answer(vec![QuestionAnswer::choice(
                QuestionId::new("direction"),
                ChoiceId::new("small"),
            )])
            .unwrap();
        let response = task.await.unwrap();
        assert_eq!(response.outcome_kind(), InteractionOutcomeKind::Answered);
    }

    #[tokio::test]
    async fn sensitive_free_form_is_registered_before_runtime_delivery() {
        const SECRET: &str = "private-task-answer";
        let sink = Arc::new(RecordingSensitiveValues::default());
        let (broker, mut requests) =
            InteractiveInteraction::with_sensitive_value_sink(sink.clone());
        let request = InteractionRequest::questionnaire(
            InteractionRequestId::new("interaction-sensitive"),
            InteractionOrigin::new(
                SessionId::new("session-1"),
                TurnId::new("turn-1"),
                ToolCallId::new("call-1"),
            ),
            Questionnaire::new(vec![
                Question::new(
                    QuestionId::new("secret"),
                    "Secret",
                    "Supply the private value",
                )
                .allow_free_form(true),
            ])
            .unwrap(),
            Deadline::never(),
            InteractionSensitivity::Sensitive,
        )
        .unwrap();
        let task = tokio::spawn(async move { broker.interact(&request).await });
        let InteractionNotice::Present(prompt) = requests.recv().await.expect("presentation")
        else {
            panic!("expected a presentation");
        };
        prompt
            .answer(vec![QuestionAnswer::free_form(
                QuestionId::new("secret"),
                SECRET,
            )])
            .expect("valid sensitive answer");
        let response = task.await.expect("broker task");

        assert_eq!(
            sink.0.lock().unwrap().as_slice(),
            &[SECRET.to_owned()],
            "the persistence sink must see the literal before the broker completes"
        );
        assert_eq!(
            response.answers().unwrap()[0].free_form_value(),
            Some(SECRET),
            "the live turn still receives the exact task information"
        );
    }

    #[tokio::test]
    async fn runtime_close_resolves_and_notifies_a_misbehaving_surface() {
        let (broker, mut requests) = InteractiveInteraction::new();
        let request = request("interaction-timeout");
        let request_id = request.id().clone();
        let task = tokio::spawn(async move { broker.interact(&request).await });
        let InteractionNotice::Present(prompt) = requests.recv().await.expect("presented request")
        else {
            panic!("expected a presentation");
        };
        // Dropping the local handle does not strand the runtime; its close
        // hook still removes the lifecycle and notifies the surface.
        drop(prompt);

        let closer = InteractiveInteraction {
            shared: Arc::clone(&requests.shared),
        };
        closer.close(&request_id, InteractionOutcomeKind::TimedOut);
        assert_eq!(
            task.await.unwrap().outcome_kind(),
            InteractionOutcomeKind::TimedOut
        );
        assert!(matches!(
            requests.recv().await,
            Some(InteractionNotice::Close {
                request_id: closed,
                outcome: InteractionOutcomeKind::TimedOut,
            }) if closed == request_id
        ));
        closer.close(&request_id, InteractionOutcomeKind::TimedOut);
    }

    #[tokio::test]
    async fn dropping_the_surface_cancels_every_active_responder_once() {
        let (broker, mut requests) = InteractiveInteraction::new();
        let first = request("first");
        let second = request("second");
        let first_task = {
            let broker = InteractiveInteraction {
                shared: Arc::clone(&broker.shared),
            };
            tokio::spawn(async move { broker.interact(&first).await })
        };
        let second_task = tokio::spawn(async move { broker.interact(&second).await });
        requests.recv().await.expect("first presentation");
        requests.recv().await.expect("second presentation");
        drop(requests);

        assert_eq!(
            first_task.await.unwrap().outcome_kind(),
            InteractionOutcomeKind::Cancelled
        );
        assert_eq!(
            second_task.await.unwrap().outcome_kind(),
            InteractionOutcomeKind::Cancelled
        );
    }

    #[tokio::test]
    async fn headless_fails_fast_and_records_no_prompt_content() {
        let broker = HeadlessInteraction::new();
        let request = request("headless");
        let response = broker.interact(&request).await;

        assert_eq!(response.outcome_kind(), InteractionOutcomeKind::Unavailable);
        assert_eq!(
            broker.required(),
            Some(InteractionRequired {
                request_id: "headless".to_owned(),
                question_count: 1,
            })
        );
        let debug = format!("{broker:?}");
        assert!(!debug.contains("Which direction?"));
    }
}
