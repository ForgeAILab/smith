//! The `task_stop` tool.
//!
//! Terminates a background task's owned process group by task ID — the same
//! `SIGTERM`-then-`SIGKILL` grace period `shell` uses on a foreground
//! timeout, just addressed by ID instead of held implicitly by the
//! invocation. Stopping an already-terminal task is not an error: it reports
//! the state the task already reached, because a model racing its own
//! `task_output` poll against a task's natural exit should not have to treat
//! "it finished first" as a failure.

use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::security::{PermissionSet, SecurityResource};
use agent_runtime_core::tool::{
    InvocationContext, PreparationContext, PreparedToolCall, Tool, ToolCallDisplay, ToolEffects,
    ToolOutcome, ToolSpec,
};
use agent_runtime_registry::Permission;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::background;
use crate::support::{invalid, require_str};

/// Stops a running background shell task.
#[derive(Debug, Default, Clone, Copy)]
pub struct TaskStopTool;

#[async_trait]
impl Tool for TaskStopTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "task_stop",
            "Stop a running background shell task by its task ID. Idempotent: a task \
             already at a terminal state reports that state instead of erroring.",
            json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "The background task ID to stop."
                    }
                },
                "required": ["task_id"],
                "additionalProperties": false
            }),
            ToolEffects::read_only().with_spawn(),
        )
        .with_permission_upper_bound(PermissionSet::single(Permission::ProcessSpawn))
    }

    async fn prepare(
        &self,
        arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        let task_id = require_str(&arguments, "task_id")?.to_owned();
        if task_id.trim().is_empty() {
            return Err(invalid("`task_id` must not be empty"));
        }
        Ok(PreparedToolCall::new(
            ctx.call_id.clone(),
            "task_stop",
            arguments,
            PermissionSet::single(Permission::ProcessSpawn),
            SecurityResource::other("background_task", task_id.clone()),
            ToolEffects::read_only().with_spawn(),
            ToolCallDisplay::new(format!("Stop task {task_id}")),
        ))
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let arguments = prepared.into_arguments();
        let task_id = require_str(&arguments, "task_id")?.to_owned();

        let host = background::installed().ok_or_else(background::host_unavailable)?;
        let status = host.stop(ctx.session.clone(), task_id.clone()).await?;

        let rendered = format!("task {task_id}: {}", status.as_str());

        Ok(ToolOutcome {
            value: json!({
                "task_id": task_id,
                "status": status.as_str(),
                "exit_code": status.exit_code(),
            }),
            content: vec![agent_runtime_core::content::ContentPart::text(rendered)].into(),
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::project;

    #[tokio::test]
    async fn without_an_installed_host_the_error_is_clear_rather_than_a_panic() {
        let (_dir, ctx) = project();
        let err = TaskStopTool
            .invoke(json!({"task_id": "task:1"}), &ctx)
            .await
            .unwrap_err();
        assert!(
            err.message.contains("no background task host"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn an_empty_task_id_is_rejected_before_any_host_lookup() {
        let (_dir, ctx) = project();
        let err = TaskStopTool
            .invoke(json!({"task_id": ""}), &ctx)
            .await
            .unwrap_err();
        assert!(err.message.contains("task_id"), "{err:?}");
    }

    #[tokio::test]
    async fn a_missing_task_id_is_rejected() {
        let (_dir, ctx) = project();
        assert!(TaskStopTool.invoke(json!({}), &ctx).await.is_err());
    }

    #[test]
    fn the_tool_declares_a_process_effect_so_approval_is_reachable() {
        let spec = TaskStopTool.spec();
        assert!(spec.effects.spawns_process());
        assert!(spec.effects.mutates());
    }
}
