//! Test scaffolding: a temporary project and an invocation context for it.

use std::sync::Arc;

use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Deadline, SystemClock};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId, TurnId};
use agent_runtime_core::tool::{
    InvocationContext, PreparationContext, Tool, ToolContent, ToolOutcome,
};
use serde_json::Value;
use smith_host::workspace::ProjectWorkspace;

/// A temporary project directory and a context bounded to it.
///
/// The directory handle is returned alongside the context because dropping it
/// deletes the project; a test that discards it would see its files vanish
/// mid-run.
pub(crate) fn project() -> (tempfile::TempDir, InvocationContext) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let ctx = context(dir.path());
    (dir, ctx)
}

/// An invocation context rooted at `root`.
pub(crate) fn context(root: &std::path::Path) -> InvocationContext {
    InvocationContext {
        session: SessionId::new("session-1"),
        turn: Some(TurnId::new("turn-1")),
        call_id: ToolCallId::new("call-1"),
        request: RequestId::new("request-1"),
        workspace: Arc::new(ProjectWorkspace::new(root).expect("a workspace")),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 32_000,
    }
}

/// The preparation-phase counterpart of an invocation context.
///
/// `PreparationContext` and `InvocationContext` share almost every field by
/// construction (an invocation is prepared and then run against the same
/// session, turn, and workspace), so every caller that needs both derives one
/// from the other rather than building it by hand.
pub(crate) fn preparation_context(ctx: &InvocationContext) -> PreparationContext {
    PreparationContext {
        session: ctx.session.clone(),
        turn: ctx.turn.clone(),
        call_id: ctx.call_id.clone(),
        request: ctx.request.clone(),
        workspace: ctx.workspace.clone(),
        clock: ctx.clock.clone(),
        cancel: ctx.cancel.clone(),
        deadline: ctx.deadline,
    }
}

/// Prepares and invokes a named tool from a composed set.
///
/// Unit tests that call `EditTool.invoke` reach the tool directly, which is the
/// right scope for its own logic but skips the observing wrapper that holds
/// session read state. Anything asserting a read precondition has to go through
/// here instead, because that is the path a real session takes.
pub(crate) async fn call(
    tools: &[Arc<dyn Tool>],
    name: &str,
    arguments: Value,
    ctx: &InvocationContext,
) -> Result<ToolOutcome, RuntimeError> {
    let tool = tools
        .iter()
        .find(|tool| tool.spec().name == name)
        .unwrap_or_else(|| panic!("no `{name}` tool in the composed set"));
    let preparation = preparation_context(ctx);
    let prepared = tool.prepare(arguments, &preparation).await?;
    tool.invoke(prepared, ctx).await
}

/// The concatenated text content of an outcome.
pub(crate) fn text_of(outcome: &ToolOutcome) -> String {
    let parts = match &outcome.content {
        ToolContent::Inline(parts) => parts,
        ToolContent::Artifact { preview, .. } => preview,
    };
    parts
        .iter()
        .filter_map(|part| match part {
            agent_runtime_core::content::ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn invoke<T: Tool>(
    tool: &T,
    arguments: Value,
    ctx: &InvocationContext,
) -> Result<ToolOutcome, RuntimeError> {
    let preparation = preparation_context(ctx);
    let prepared = tool.prepare(arguments, &preparation).await?;
    tool.invoke(prepared, ctx).await
}

macro_rules! test_invoke {
    ($tool:ty) => {
        impl $tool {
            pub(crate) async fn invoke(
                &self,
                arguments: Value,
                ctx: &InvocationContext,
            ) -> Result<ToolOutcome, RuntimeError> {
                invoke(self, arguments, ctx).await
            }
        }
    };
}

test_invoke!(crate::ReadTool);
test_invoke!(crate::ListTool);
test_invoke!(crate::SearchTool);
test_invoke!(crate::EditTool);
test_invoke!(crate::ShellTool);
test_invoke!(crate::TaskOutputTool);
test_invoke!(crate::TaskStopTool);
