//! Codex stream normalization.

use agent_runtime::agent::external::ExternalAgentEvent;
use agent_runtime_core::usage::{CounterKind, UsageDelta};

use super::{TurnState, counter, session_event, string_field};

/// A Codex item's identity, which correlates its started and completed
/// reports. An item without one cannot be correlated, so every such item
/// shares one identity rather than opening rows nothing ever closes.
fn codex_item_id(item: &serde_json::Value) -> String {
    string_field(item, "id").unwrap_or_else(|| "item".to_owned())
}

/// The invocation event for one Codex tool item, or `None` when the item is
/// not a tool.
fn codex_tool_invocation(item: &serde_json::Value, id: &str) -> Option<ExternalAgentEvent> {
    let name = match item.get("type").and_then(serde_json::Value::as_str)? {
        name @ ("command_execution" | "file_change" | "mcp_tool_call") => name,
        _ => return None,
    };
    Some(ExternalAgentEvent::ToolInvoked {
        id: id.to_owned(),
        name: name.to_owned(),
        detail: item.clone(),
    })
}

fn is_bridge_call(item: &serde_json::Value, state: &TurnState) -> bool {
    item.get("type").and_then(serde_json::Value::as_str) == Some("mcp_tool_call")
        && state.bridge_server.as_deref().is_some_and(|server| {
            item.get("server").and_then(serde_json::Value::as_str) == Some(server)
        })
}

/// Codex reports `input_tokens` *inclusive* of the cached prefix, so the
/// cached count is subtracted back out to keep the counters disjoint.
pub(super) fn codex_usage(usage: &serde_json::Value) -> UsageDelta {
    let cached = counter(usage, "cached_input_tokens");
    let input = counter(usage, "input_tokens").saturating_sub(cached);
    UsageDelta::new()
        .with(CounterKind::InputUncached, input)
        .with(CounterKind::InputCached, cached)
        .with(
            CounterKind::CacheWrite,
            counter(usage, "cache_write_input_tokens"),
        )
        .with(CounterKind::Output, counter(usage, "output_tokens"))
        .with(
            CounterKind::Reasoning,
            counter(usage, "reasoning_output_tokens"),
        )
}

pub(super) fn normalize_codex(
    value: &serde_json::Value,
    state: &mut TurnState,
) -> Vec<ExternalAgentEvent> {
    let mut events = Vec::new();
    match value.get("type").and_then(serde_json::Value::as_str) {
        Some("thread.started") => {
            events.extend(session_event(string_field(value, "thread_id")));
        }
        // A command Codex is still running is reported as a started item
        // first. Opening the row here is what makes a slow command visible
        // while it runs rather than appearing, already finished, at the end.
        Some("item.started") => {
            let Some(item) = value.get("item") else {
                return events;
            };
            if is_bridge_call(item, state) {
                return events;
            }
            let id = codex_item_id(item);
            if let Some(invoked) = codex_tool_invocation(item, &id) {
                state.invoked.insert(id);
                events.push(invoked);
            }
        }
        Some("item.completed") => {
            let Some(item) = value.get("item") else {
                return events;
            };
            if is_bridge_call(item, state) {
                return events;
            }
            let id = codex_item_id(item);
            match item.get("type").and_then(serde_json::Value::as_str) {
                Some("agent_message") => {
                    if let Some(text) = string_field(item, "text") {
                        events.push(ExternalAgentEvent::Text { text });
                    }
                }
                Some("reasoning") => {
                    if let Some(text) = string_field(item, "text") {
                        events.push(ExternalAgentEvent::Reasoning { text });
                    }
                }
                // The completed item carries the invocation as well as the
                // outcome, so a tool whose start was never reported still
                // gets its row -- just once, which is what the started set
                // above is for.
                Some("command_execution") | Some("file_change") | Some("mcp_tool_call") => {
                    if !state.invoked.remove(&id)
                        && let Some(invoked) = codex_tool_invocation(item, &id)
                    {
                        events.push(invoked);
                    }
                    let exit = item.get("exit_code").and_then(serde_json::Value::as_i64);
                    events.push(ExternalAgentEvent::ToolCompleted {
                        id,
                        // Only a command reports an exit status; a file change
                        // or an MCP call that completed at all succeeded.
                        ok: exit.is_none_or(|exit| exit == 0),
                        detail: item
                            .get("aggregated_output")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                    });
                }
                // `error` items also carry benign local warnings — a malformed
                // agent-role file, a clamped hook timeout — so one must not
                // fail the turn on its own. Only `turn.failed` is terminal.
                _ => {}
            }
        }
        Some("turn.completed") => {
            if let Some(usage) = value.get("usage") {
                events.push(ExternalAgentEvent::Usage {
                    usage: codex_usage(usage),
                });
            }
            events.push(ExternalAgentEvent::Completed);
        }
        Some("turn.failed") => {
            let message = value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("CLI agent reported a failed turn")
                .to_owned();
            events.push(ExternalAgentEvent::Failed { message });
        }
        _ => {}
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_codex_error_item_is_a_warning_not_a_failed_turn() {
        let value = serde_json::json!({
            "type": "item.completed",
            "item": {"id": "item_0", "type": "error", "message": "clamping hook timeout"}
        });
        assert!(normalize_codex(&value, &mut TurnState::default()).is_empty());
    }

    #[test]
    fn codex_command_execution_reports_both_invocation_and_outcome() {
        let value = serde_json::json!({
            "type": "item.completed",
            "item": {
                "id": "item_5",
                "type": "command_execution",
                "command": "/bin/zsh -lc 'cat config.txt'",
                "aggregated_output": "version = 3\n",
                "exit_code": 0,
                "status": "completed"
            }
        });
        let events = normalize_codex(&value, &mut TurnState::default());
        assert!(matches!(events[0], ExternalAgentEvent::ToolInvoked { .. }));
        assert!(matches!(
            events[1],
            ExternalAgentEvent::ToolCompleted { ok: true, .. }
        ));
    }

    #[test]
    fn a_codex_command_reported_as_started_opens_exactly_one_row() {
        let mut state = TurnState::default();
        let item = serde_json::json!({
            "id": "item_7",
            "type": "command_execution",
            "command": "/bin/zsh -lc 'echo hi'",
            "aggregated_output": "",
            "exit_code": serde_json::Value::Null,
            "status": "in_progress"
        });
        let started = normalize_codex(
            &serde_json::json!({"type": "item.started", "item": item}),
            &mut state,
        );
        assert!(matches!(
            started.as_slice(),
            [ExternalAgentEvent::ToolInvoked { name, .. }] if name == "command_execution"
        ));

        let mut completed_item = item;
        completed_item["aggregated_output"] = serde_json::json!("hi\n");
        completed_item["exit_code"] = serde_json::json!(0);
        completed_item["status"] = serde_json::json!("completed");
        let completed = normalize_codex(
            &serde_json::json!({"type": "item.completed", "item": completed_item}),
            &mut state,
        );
        assert!(
            matches!(
                completed.as_slice(),
                [ExternalAgentEvent::ToolCompleted { ok: true, .. }]
            ),
            "the started row is resolved, not duplicated: {completed:?}"
        );
    }

    #[test]
    fn a_codex_file_change_completes_without_an_exit_status() {
        let events = normalize_codex(
            &serde_json::json!({
                "type": "item.completed",
                "item": {
                    "id": "item_8",
                    "type": "file_change",
                    "changes": [{"path": "/repo/note.txt", "kind": "add"}],
                    "status": "completed"
                }
            }),
            &mut TurnState::default(),
        );
        assert!(matches!(
            events.as_slice(),
            [
                ExternalAgentEvent::ToolInvoked { name, .. },
                ExternalAgentEvent::ToolCompleted { ok: true, .. }
            ] if name == "file_change"
        ));
    }
}
