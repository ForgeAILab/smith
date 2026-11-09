use crate::{Evidence, FeedbackStore, ReportInput};
use agent_runtime_core::{
    error::RuntimeError,
    security::SecurityResource,
    tool::{
        InvocationContext, PreparationContext, PreparedToolCall, Tool, ToolCallDisplay,
        ToolEffects, ToolOutcome, ToolSpec,
    },
};
use async_trait::async_trait;
use serde_json::{Value, json};
use smith_module::ModuleContext;
#[derive(Debug)]
pub(crate) struct FeedbackTool {
    pub context: ModuleContext,
    pub store: FeedbackStore,
}
#[async_trait]
impl Tool for FeedbackTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "harness_feedback",
            crate::GUIDANCE,
            json!({"type":"object","properties":{
            "kind":{"type":"string","enum":["bug","missing_capability","idea"]},"area":{"type":"string"},"title":{"type":"string"},"what_happened":{"type":"string"},"user_said":{"type":"string"},"repro":{"type":"string"}},"required":["kind","area","title","what_happened"],"additionalProperties":false}),
            ToolEffects::default(),
        )
    }
    fn normalize_arguments(&self, mut arguments: Value) -> Result<Value, RuntimeError> {
        if let Some(o) = arguments.as_object_mut() {
            o.retain(|k, _| {
                [
                    "kind",
                    "area",
                    "title",
                    "what_happened",
                    "user_said",
                    "repro",
                ]
                .contains(&k.as_str())
            });
        }
        Ok(arguments)
    }
    async fn prepare(
        &self,
        arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        let arguments = self.normalize_arguments(arguments)?;
        let input: ReportInput = serde_json::from_value(arguments.clone())
            .map_err(|e| RuntimeError::tool(e.to_string()))?;
        let effects = ToolEffects::default();
        let (permissions, _) =
            effects.authorization_request("harness_feedback", ctx.workspace.root());
        Ok(PreparedToolCall::new(
            ctx.call_id.clone(),
            "harness_feedback",
            arguments,
            permissions,
            SecurityResource::other("smith", "feedback"),
            effects,
            ToolCallDisplay::new("Harness Feedback")
                .with_detail(crate::store::bounded(&input.title, 120)),
        ))
    }
    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let input = serde_json::from_value(prepared.into_arguments())
            .map_err(|e| RuntimeError::tool(e.to_string()))?;
        let evidence = Evidence {
            identity: self.context.session_identity.clone(),
            os: std::env::consts::OS.into(),
            session: ctx.session.to_string(),
            turn: ctx.turn.as_ref().map(ToString::to_string),
            call: ctx.call_id.to_string(),
            child: self
                .context
                .is_child_session
                .then(|| ctx.session.to_string()),
            workspace: ctx.workspace.root().into(),
            failures: self
                .context
                .recent_failures
                .as_ref()
                .map(|s| s.recent(&ctx.session))
                .unwrap_or_default(),
            coverage: if self.context.session_identity.provider.starts_with("cli/")
                || self.context.session_identity.model.starts_with("cli/")
            {
                "Only bridged calls are covered; the CLI's own tools are not covered."
            } else {
                "Smith tool calls"
            }
            .into(),
        };
        Ok(
            match self
                .store
                .record(input, evidence, self.context.feedback_redactor.as_deref())
            {
                Ok(r) => ToolOutcome::text(format!("Recorded {} (count {}).", r.id, r.count)),
                Err(e) => ToolOutcome::error(e.to_string()),
            },
        )
    }
}
