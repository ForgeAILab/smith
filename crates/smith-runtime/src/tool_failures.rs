//! Capture invocation errors before canonical history commits the turn.
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::{error::RuntimeError, tool::*};
use async_trait::async_trait;
use smith_module::{RecentFailures, ToolFailure};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};
/// Per-session rings shared by invocation wrappers and module evidence.
#[derive(Debug, Default)]
pub struct FailureRecorder {
    sessions: Mutex<BTreeMap<String, VecDeque<ToolFailure>>>,
}
impl RecentFailures for FailureRecorder {
    fn recent(&self, session: &SessionId) -> Vec<ToolFailure> {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&session.to_string())
            .map(|v| v.iter().cloned().collect())
            .unwrap_or_default()
    }
}
impl FailureRecorder {
    pub(crate) fn wrap(self: &Arc<Self>, inner: Arc<dyn Tool>) -> Arc<dyn Tool> {
        Arc::new(Observed {
            inner,
            recorder: self.clone(),
        })
    }
    fn record(&self, name: String, ctx: &InvocationContext, error: String) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let calls = sessions.entry(ctx.session.to_string()).or_default();
        calls.push_back(ToolFailure {
            name,
            call_id: ctx.call_id.to_string(),
            turn_id: ctx.turn.as_ref().map(ToString::to_string),
            error: if error.chars().count() > 500 {
                error.chars().take(487).collect::<String>() + "… [truncated]"
            } else {
                error
            },
        });
        while calls.len() > 5 {
            calls.pop_front();
        }
    }
}
#[derive(Debug)]
struct Observed {
    inner: Arc<dyn Tool>,
    recorder: Arc<FailureRecorder>,
}
#[async_trait]
impl Tool for Observed {
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }
    fn normalize_arguments(
        &self,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        self.inner.normalize_arguments(arguments)
    }
    async fn prepare(
        &self,
        arguments: serde_json::Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        self.inner.prepare(arguments, ctx).await
    }
    fn supports_interaction(&self) -> bool {
        self.inner.supports_interaction()
    }
    fn interaction_request(
        &self,
        prepared: &PreparedToolCall,
        origin: agent_runtime_core::interaction::InteractionOrigin,
        deadline: agent_runtime_core::clock::Deadline,
    ) -> Result<Option<agent_runtime_core::interaction::InteractionRequest>, RuntimeError> {
        self.inner.interaction_request(prepared, origin, deadline)
    }
    fn resolve_interaction(
        &self,
        prepared: &PreparedToolCall,
        response: &agent_runtime_core::interaction::InteractionResponse,
    ) -> Result<ToolOutcome, RuntimeError> {
        self.inner.resolve_interaction(prepared, response)
    }
    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let result = self.inner.invoke(prepared, ctx).await;
        let error = match &result {
            Err(e) => Some(e.to_string()),
            Ok(o) if o.is_error => {
                let text = o
                    .content
                    .as_inline()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|part| part.as_text())
                    .collect::<Vec<_>>()
                    .join("\n");
                Some(if text.is_empty() {
                    o.value.to_string()
                } else {
                    text
                })
            }
            _ => None,
        };
        if let Some(error) = error {
            self.recorder.record(self.spec().name, ctx, error);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Debug)]
    struct Fails(bool);
    #[async_trait]
    impl Tool for Fails {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(
                "failed_tool",
                "fixture",
                serde_json::json!({"type":"object"}),
                ToolEffects::default(),
            )
        }
        async fn invoke(
            &self,
            _: PreparedToolCall,
            _: &InvocationContext,
        ) -> Result<ToolOutcome, RuntimeError> {
            if self.0 {
                Err(RuntimeError::tool("界".repeat(800)))
            } else {
                Ok(ToolOutcome::error("bad outcome"))
            }
        }
    }
    #[tokio::test]
    async fn records_both_error_forms_bounded_and_isolated_by_session() {
        use agent_runtime_core::{
            cancel::Cancellation,
            clock::{Deadline, SystemClock},
            ids::{RequestId, ToolCallId},
        };
        let dir = tempfile::tempdir().unwrap();
        let recorder = Arc::new(FailureRecorder::default());
        let mut ctx = InvocationContext {
            session: SessionId::new("one"),
            turn: None,
            call_id: ToolCallId::new("call"),
            request: RequestId::new("req"),
            workspace: Arc::new(smith_host::ProjectWorkspace::new(dir.path()).unwrap()),
            clock: Arc::new(SystemClock),
            cancel: Cancellation::new(),
            deadline: Deadline::never(),
            output_limit: 1000,
        };
        for i in 0..7 {
            let tool = recorder.wrap(Arc::new(Fails(i % 2 == 0)));
            ctx.call_id = ToolCallId::new(format!("call-{i}"));
            let prepared = PreparedToolCall::from_static_effects(
                ctx.call_id.clone(),
                &tool.spec(),
                serde_json::json!({}),
                ctx.workspace.root(),
            );
            let _ = tool.invoke(prepared, &ctx).await;
        }
        let recent = recorder.recent(&ctx.session);
        assert_eq!(recent.len(), 5);
        assert_eq!(recent[0].call_id, "call-2");
        assert!(recent.iter().all(|f| f.error.chars().count() <= 500));
        assert!(recent.iter().any(|f| f.error.contains("bad outcome")));
        assert!(recorder.recent(&SessionId::new("other")).is_empty());
    }
}
