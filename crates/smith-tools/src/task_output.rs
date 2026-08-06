//! The `task_output` tool.
//!
//! Reads a background task's spooled output incrementally by byte offset —
//! the poll half of `run_in_background`. A model that started a build in the
//! background has no other way to learn how it's going short of guessing when
//! it might be done, so this stays cheap to call repeatedly: pass the
//! previous response's `next_offset` back in as `offset` and only the output
//! that arrived since comes back.

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
use crate::support::{invalid, optional_usize, require_str};

/// Output slice size when the caller does not say.
const DEFAULT_LIMIT: usize = 65_536;

/// Reads a background task's status and spooled output.
#[derive(Debug, Default, Clone, Copy)]
pub struct TaskOutputTool;

#[async_trait]
impl Tool for TaskOutputTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "task_output",
            "Read a background shell task's status and spooled output: running or \
             terminal, exit code once terminal, and output between `offset` and \
             `next_offset`. Pass the previous `next_offset` back as `offset` to poll \
             only new output.",
            json!({
                "type": "object",
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "The background task ID, from `shell`'s `run_in_background` response or a manual-backgrounding notice."
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Byte offset to start reading from. Defaults to 0."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum bytes to return. Defaults to 65536."
                    }
                },
                "required": ["task_id"],
                "additionalProperties": false
            }),
            ToolEffects::read_only(),
        )
        .with_permission_upper_bound(PermissionSet::single(Permission::FsRead))
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
            "task_output",
            arguments,
            PermissionSet::single(Permission::FsRead),
            SecurityResource::other("background_task", task_id.clone()),
            ToolEffects::read_only(),
            ToolCallDisplay::new(format!("Read output of task {task_id}")),
        ))
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let arguments = prepared.into_arguments();
        let task_id = require_str(&arguments, "task_id")?.to_owned();
        let offset = optional_usize(&arguments, "offset").unwrap_or(0);
        let limit = optional_usize(&arguments, "limit")
            .unwrap_or(DEFAULT_LIMIT)
            .max(1);

        let host = background::installed().ok_or_else(background::host_unavailable)?;
        let result = host
            .output(ctx.session.clone(), task_id.clone(), offset, limit)
            .await?;

        let mut rendered = format!("task {task_id}: {}", result.status.as_str());
        if let Some(code) = result.exit_code {
            rendered.push_str(&format!(" (exit {code})"));
        }
        rendered.push('\n');
        if result.output.is_empty() {
            rendered.push_str("(no new output)");
        } else {
            rendered.push_str(result.output.trim_end());
        }
        if result.truncated {
            rendered.push_str("\n[the task's spool was truncated at its byte cap]");
        }

        Ok(ToolOutcome {
            value: json!({
                "task_id": task_id,
                "status": result.status.as_str(),
                "exit_code": result.exit_code,
                "offset": result.offset,
                "next_offset": result.next_offset,
                "truncated": result.truncated,
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
        let err = TaskOutputTool
            .invoke(json!({"task_id": "task:1"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.message.contains("no background task host"), "{err:?}");
    }

    #[tokio::test]
    async fn an_empty_task_id_is_rejected_before_any_host_lookup() {
        let (_dir, ctx) = project();
        let err = TaskOutputTool
            .invoke(json!({"task_id": "   "}), &ctx)
            .await
            .unwrap_err();
        assert!(err.message.contains("task_id"), "{err:?}");
    }

    #[tokio::test]
    async fn a_missing_task_id_is_rejected() {
        let (_dir, ctx) = project();
        assert!(TaskOutputTool.invoke(json!({}), &ctx).await.is_err());
    }

    #[test]
    fn the_tool_declares_read_only_effects() {
        let spec = TaskOutputTool.spec();
        assert!(spec.effects.is_read_only());
    }
}
