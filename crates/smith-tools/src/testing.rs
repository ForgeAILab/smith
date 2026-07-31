//! Test scaffolding: a temporary project and an invocation context for it.

use std::sync::Arc;

use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Deadline, SystemClock};
use agent_runtime_core::ids::{RequestId, ToolCallId};
use agent_runtime_core::tool::{InvocationContext, ToolOutcome};
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
        call_id: ToolCallId::new("call-1"),
        request: RequestId::new("request-1"),
        workspace: Arc::new(ProjectWorkspace::new(root).expect("a workspace")),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 32_000,
    }
}

/// The concatenated text content of an outcome.
pub(crate) fn text_of(outcome: &ToolOutcome) -> String {
    outcome
        .content
        .iter()
        .filter_map(|part| match part {
            agent_runtime_core::content::ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
