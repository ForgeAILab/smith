//! Request compatibility Smith owns for the native Gemini adapter.
//!
//! Gemini's Interactions API refuses a request whose conversation ends with a
//! model turn: `Requests ending with a model turn are not supported.` The
//! shared runtime's *attributed internal turns* end exactly that way. A child
//! completing is one: its outcome rides in the turn's context as an
//! attributed fragment rather than as a message, so the last thing in the
//! conversation is still whatever the assistant last said. Every other
//! adapter Smith ships accepts that and simply continues from it.
//!
//! Left alone, the delivery of every child result on a Gemini binding failed
//! with a 400 the user saw as "Gemini provider rejected the request" — the
//! result itself lost with it — and the same held for any other internal turn.
//! So on this adapter, and only on it, a request that would end on a model
//! turn is given one explicit continuation turn. It says only that the
//! instruction is in context, because it is: the attributed fragment is
//! already in the request Smith is about to send, and restating its content
//! here would put an unbounded copy of protected child text into a message
//! Smith invented.

use std::fmt;
use std::sync::Arc;

use agent_runtime_core::content::{Message, Role};
use agent_runtime_core::provider::{
    Capabilities, ModelDescriptor, ModelId, Provider, ProviderCallContext, ProviderError,
    ProviderRequest, ProviderStream,
};
use async_trait::async_trait;

/// The continuation turn appended to a conversation that would otherwise end
/// on a model turn.
const CONTINUATION: &str =
    "Continue this turn from the attributed instruction at the end of your context.";

/// Wraps a native Gemini provider so no request ends on a model turn.
pub fn accept_internal_turns(provider: Arc<dyn Provider>) -> Arc<dyn Provider> {
    Arc::new(GeminiContinuationProvider { inner: provider })
}

struct GeminiContinuationProvider {
    inner: Arc<dyn Provider>,
}

impl fmt::Debug for GeminiContinuationProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeminiContinuationProvider")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Provider for GeminiContinuationProvider {
    fn describe(&self) -> Vec<ModelDescriptor> {
        self.inner.describe()
    }

    fn capabilities(&self, model: &ModelId) -> Option<Capabilities> {
        self.inner.capabilities(model)
    }

    async fn stream(
        &self,
        mut request: ProviderRequest,
        ctx: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        close_model_turn(&mut request);
        self.inner.stream(request, ctx).await
    }
}

/// Appends the continuation turn when the conversation ends on a model turn.
///
/// System messages are skipped rather than counted as the end of the
/// conversation: the adapter hoists every one of them into the request's
/// system instruction wherever it sits in the list, and the attributed
/// instruction of an internal turn arrives as exactly such a trailing system
/// message. What Gemini sees as the last turn is the last message that is not
/// one.
///
/// Appending rather than rewriting keeps the planned cache prefix intact: the
/// added turn is a suffix, so nothing before it moves.
fn close_model_turn(request: &mut ProviderRequest) {
    let last_turn = request
        .messages
        .iter()
        .rev()
        .find(|message| message.role != Role::System);
    if matches!(last_turn.map(|message| message.role), Some(Role::Assistant)) {
        request.messages.push(Message::user(CONTINUATION));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use agent_runtime_core::content::{ContentPart, ToolCall, ToolResultBlock};
    use agent_runtime_core::ids::ToolCallId;

    fn request(messages: Vec<Message>) -> ProviderRequest {
        ProviderRequest {
            messages,
            ..ProviderRequest::new(ModelId::new("gemini-3.6-flash"), Vec::new())
        }
    }

    #[test]
    fn a_conversation_ending_on_a_model_turn_is_closed_with_one_continuation() {
        // The shape an attributed internal turn produces: the instruction is
        // in context, and the conversation still ends where the assistant
        // left off.
        let mut planned = request(vec![
            Message::user("spawn a child"),
            Message::assistant(vec![ContentPart::Text {
                text: "spawned".to_owned(),
            }]),
        ]);
        close_model_turn(&mut planned);

        assert_eq!(planned.messages.len(), 3);
        assert_eq!(planned.messages[2].role, Role::User);
        assert_eq!(planned.messages[2].joined_text(), CONTINUATION);

        // Idempotent in effect: the request it produced no longer ends on a
        // model turn, so a second pass adds nothing.
        close_model_turn(&mut planned);
        assert_eq!(planned.messages.len(), 3);
    }

    /// The shape an internal turn actually plans: its attributed instruction
    /// is a trailing system message, which the adapter hoists into the
    /// request's system instruction rather than sending as a turn.
    #[test]
    fn a_trailing_system_message_does_not_hide_the_model_turn_behind_it() {
        let mut planned = request(vec![
            Message::user("spawn a child"),
            Message::assistant(vec![ContentPart::Text {
                text: "spawned".to_owned(),
            }]),
            Message::system("Protected delegated child outcomes: child-1 completed"),
        ]);
        close_model_turn(&mut planned);

        assert_eq!(planned.messages.len(), 4);
        assert_eq!(planned.messages[3].role, Role::User);
        assert_eq!(planned.messages[3].joined_text(), CONTINUATION);
    }

    #[test]
    fn an_ordinary_turn_is_sent_exactly_as_planned() {
        for last in [
            Message::user("read the file"),
            Message::tool_result(ToolResultBlock {
                call_id: ToolCallId::new("call-1"),
                name: "read".to_owned(),
                content: vec![ContentPart::text("contents")],
                is_error: false,
            }),
        ] {
            let mut planned = request(vec![
                Message::user("read the file"),
                Message::assistant(vec![ContentPart::ToolCall(ToolCall {
                    id: ToolCallId::new("call-1"),
                    name: "read".to_owned(),
                    arguments: serde_json::json!({"path": "README.md"}),
                })]),
                last,
            ]);
            let before = planned.messages.clone();
            close_model_turn(&mut planned);
            assert_eq!(planned.messages, before);
        }
    }
}
