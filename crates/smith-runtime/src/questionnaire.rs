//! Smith's custom-answer policy over the shared questionnaire protocol.

use agent_runtime::harness::QuestionnaireTool;
use agent_runtime_core::clock::Deadline;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::interaction::{InteractionOrigin, InteractionRequest, InteractionResponse};
use agent_runtime_core::tool::{
    InvocationContext, PreparationContext, PreparedToolCall, Tool, ToolOutcome, ToolSpec,
};
use async_trait::async_trait;
use serde_json::{Value, json};

#[derive(Debug)]
pub(crate) struct SmithQuestionnaireTool;

fn allow_custom_answers(mut arguments: Value) -> Value {
    if let Some(questions) = arguments.get_mut("questions").and_then(Value::as_array_mut) {
        for question in questions {
            if let Some(question) = question.as_object_mut() {
                question.insert("allow_free_form".into(), json!(true));
            }
        }
    }
    arguments
}

#[async_trait]
impl Tool for SmithQuestionnaireTool {
    fn spec(&self) -> ToolSpec {
        let mut spec = QuestionnaireTool::new().spec();
        spec.description.push_str(
            " The user can always type a custom answer or discuss the question before deciding.",
        );
        spec
    }

    async fn prepare(
        &self,
        arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        QuestionnaireTool::new()
            .prepare(allow_custom_answers(arguments), ctx)
            .await
    }

    fn supports_interaction(&self) -> bool {
        true
    }

    fn interaction_request(
        &self,
        prepared: &PreparedToolCall,
        origin: InteractionOrigin,
        deadline: Deadline,
    ) -> Result<Option<InteractionRequest>, RuntimeError> {
        let prepared = PreparedToolCall::new(
            prepared.call_id().clone(),
            prepared.tool(),
            allow_custom_answers(prepared.arguments().clone()),
            prepared.required_permissions().clone(),
            prepared.resource().clone(),
            prepared.effects().clone(),
            prepared.display().clone(),
        );
        QuestionnaireTool::new().interaction_request(&prepared, origin, deadline)
    }

    fn resolve_interaction(
        &self,
        prepared: &PreparedToolCall,
        response: &InteractionResponse,
    ) -> Result<ToolOutcome, RuntimeError> {
        QuestionnaireTool::new().resolve_interaction(prepared, response)
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        QuestionnaireTool::new().invoke(prepared, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::ids::{QuestionId, SessionId, ToolCallId, TurnId};
    use agent_runtime_core::interaction::QuestionAnswer;

    #[test]
    fn custom_answers_are_valid_even_when_the_model_disables_them() {
        let tool = SmithQuestionnaireTool;
        for flag in [None, Some(false), Some(true)] {
            let mut question = json!({"id":"q", "header":"Choice", "prompt":"Which?", "choices":[{"id":"a", "label":"A"}]});
            if let Some(flag) = flag {
                question["allow_free_form"] = json!(flag);
            }
            let prepared = PreparedToolCall::from_static_effects(
                ToolCallId::new("call"),
                &tool.spec(),
                json!({"questions":[question]}),
                "/repo",
            );
            let request = tool
                .interaction_request(
                    &prepared,
                    InteractionOrigin::new(
                        SessionId::new("s"),
                        TurnId::new("t"),
                        ToolCallId::new("call"),
                    ),
                    Deadline::never(),
                )
                .unwrap()
                .unwrap();
            let response = InteractionResponse::answered(
                request.id().clone(),
                vec![QuestionAnswer::free_form(
                    QuestionId::new("q"),
                    "A custom answer",
                )],
            );
            response.validate_for(&request).unwrap();
            assert!(tool.spec().effects.is_empty());
            assert!(tool.spec().permission_upper_bound.is_empty());
        }
    }
}
