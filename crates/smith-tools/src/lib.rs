//! Smith's built-in coding tools.
//!
//! The shared runtime supplies the [`Tool`](agent_runtime_core::tool::Tool)
//! trait, the approval gate, and the executor, but no concrete tools — what an
//! agent may do to a machine is product policy. These are Smith's:
//!
//! | Tool | Effects |
//! | --- | --- |
//! | [`ReadTool`] | read-only |
//! | [`ListTool`] | read-only |
//! | [`SearchTool`] | read-only |
//! | [`EditTool`] | writes |
//! | [`ShellTool`] | writes, spawns processes, network |
//! | [`TaskOutputTool`] | read-only |
//! | [`TaskStopTool`] | spawns processes (process control) |
//!
//! Built-in filesystem tools resolve paths through the session's
//! [`Workspace`](agent_runtime_core::workspace::Workspace), so their project
//! boundary is enforced in one place. [`ShellTool`] is the explicit exception:
//! until Smith supplies an OS sandbox, it declares same-user host authority
//! rather than pretending its working directory contains the process. Mutating
//! tools declare their effects, which is what makes the runtime route them
//! through approval before they run.
//!
//! Reads, searches, listings, and command output are all bounded. A tool that
//! can return an unbounded amount of text is a tool that can exhaust a context
//! window on one call.
//!
//! [`ShellTool`] can also hand a command to a session-owned background task
//! registry instead of waiting for it — see [`background`] for the seam
//! `TaskOutputTool` and `TaskStopTool` poll and control through. That
//! registry lives one crate up, in `smith-runtime`; a host that never
//! installs one still gets a coherent tool surface, just with those three
//! paths returning a clear error instead of running.

pub mod background;
pub mod change;
pub mod display;
pub mod edit;
pub mod list;
pub mod read;
pub mod read_state;
pub mod search;
pub mod shell;
pub mod support;
pub mod task_output;
pub mod task_stop;

#[cfg(test)]
mod testing;

pub use change::{
    ChangeRecorder, EditMutation, ToolMutation, TurnChangeSet, observed_tools,
    observed_tools_with_background,
};
pub use display::{ToolCallDisplay, has_tool_call_display_schema, project_tool_call_display};
pub use edit::EditTool;
pub use list::ListTool;
pub use read::ReadTool;
pub use read_state::{ReadDefect, ReadObservation, ReadRecorder};
pub use search::SearchTool;
pub use shell::{HOST_SHELL_RESOURCE_KIND, ShellTool};
pub use task_output::TaskOutputTool;
pub use task_stop::TaskStopTool;

use std::sync::Arc;

use agent_runtime_core::tool::Tool;

/// The bare tools, before any session observation is attached.
///
/// Ordering is stable so the model sees the same tool list across runs — a
/// changing order needlessly invalidates a provider's prompt cache.
pub(crate) fn built_in(background: Arc<dyn background::BackgroundTaskHost>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ReadTool),
        Arc::new(ListTool),
        Arc::new(SearchTool),
        Arc::new(EditTool),
        Arc::new(ShellTool::new(background.clone())),
        Arc::new(TaskOutputTool::new(background.clone())),
        Arc::new(TaskStopTool::new(background)),
    ]
}

/// Every built-in tool, ready to register with a runtime builder.
///
/// The tools are always wrapped, even without a [`ChangeRecorder`], because
/// `edit`'s destructive operations are gated on what the session has read and
/// that state has to live somewhere. Tools themselves stay pure functions of
/// their arguments and the workspace.
pub fn all() -> Vec<Arc<dyn Tool>> {
    all_with_background(background::unavailable())
}

/// Every built-in tool, wired to the background-task owner for this runtime.
pub fn all_with_background(
    background: Arc<dyn background::BackgroundTaskHost>,
) -> Vec<Arc<dyn Tool>> {
    change::observe_with_background(None, ReadRecorder::new(), background)
}

/// Only the tools that cannot change anything.
///
/// Useful for a read-only child agent or an exploratory session, where the
/// approval gate should have nothing to be asked about.
pub fn read_only() -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(ReadTool), Arc::new(ListTool), Arc::new(SearchTool)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_set_has_unique_stable_names() {
        let tools = all();
        let names: Vec<String> = tools.iter().map(|tool| tool.spec().name).collect();
        assert_eq!(
            names,
            [
                "read",
                "list",
                "search",
                "edit",
                "shell",
                "task_output",
                "task_stop"
            ]
        );

        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "tool names must be unique");
    }

    #[test]
    fn the_read_only_set_declares_no_mutating_effects() {
        for tool in read_only() {
            let spec = tool.spec();
            assert!(
                spec.effects.is_read_only(),
                "`{}` is in the read-only set but declares effects",
                spec.name
            );
        }
    }

    #[test]
    fn mutating_tools_declare_effects_so_approval_is_reachable() {
        // A mutating tool that forgot to declare its effects would bypass the
        // approval gate entirely, so this is a safety check, not a style one.
        for tool in all() {
            let spec = tool.spec();
            let mutating = matches!(spec.name.as_str(), "edit" | "shell" | "task_stop");
            assert_eq!(
                spec.effects.mutates(),
                mutating,
                "`{}` declares the wrong effects",
                spec.name
            );
        }
    }

    #[test]
    fn every_tool_advertises_a_usable_object_schema() {
        for tool in all() {
            let spec = tool.spec();
            let schema = spec.input_schema;
            assert_eq!(schema["type"], "object", "`{}`", spec.name);
            assert!(
                schema["properties"].is_object(),
                "`{}` has no properties",
                spec.name
            );
            assert!(
                !spec.description.is_empty(),
                "`{}` has no description",
                spec.name
            );
        }
    }
}
