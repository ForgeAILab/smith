//! Claude Code stream normalization.

use agent_runtime::agent::external::ExternalAgentEvent;
use agent_runtime_core::usage::{CounterKind, UsageDelta};

use super::{TurnState, counter, session_event, string_field};

pub(super) fn normalize_claude(
    value: &serde_json::Value,
    state: &mut TurnState,
) -> Vec<ExternalAgentEvent> {
    let mut events = Vec::new();
    match value.get("type").and_then(serde_json::Value::as_str) {
        Some("system") => {
            if value.get("subtype").and_then(serde_json::Value::as_str) == Some("init") {
                events.extend(session_event(string_field(value, "session_id")));
            }
        }
        Some("assistant") => {
            let content = value
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(serde_json::Value::as_array);
            for part in content.into_iter().flatten() {
                match part.get("type").and_then(serde_json::Value::as_str) {
                    Some("text") => {
                        if let Some(text) = string_field(part, "text") {
                            events.push(ExternalAgentEvent::Text { text });
                        }
                    }
                    Some("thinking") => {
                        if let Some(text) =
                            string_field(part, "thinking").filter(|text| !text.is_empty())
                        {
                            events.push(ExternalAgentEvent::Reasoning { text });
                        }
                    }
                    Some("tool_use") => {
                        if let (Some(id), Some(name)) =
                            (string_field(part, "id"), string_field(part, "name"))
                        {
                            if state
                                .bridge_server
                                .as_ref()
                                .is_some_and(|server| name.starts_with(&format!("mcp__{server}__")))
                            {
                                state.suppressed.insert(id);
                                continue;
                            }
                            events.push(ExternalAgentEvent::ToolInvoked {
                                id,
                                name,
                                detail: part
                                    .get("input")
                                    .cloned()
                                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        Some("user") => {
            let content = value
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(serde_json::Value::as_array);
            for part in content.into_iter().flatten() {
                if part.get("type").and_then(serde_json::Value::as_str) == Some("tool_result")
                    && let Some(id) = string_field(part, "tool_use_id")
                {
                    if state.suppressed.contains(&id) {
                        continue;
                    }
                    {
                        let failed = part
                            .get("is_error")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        events.push(ExternalAgentEvent::ToolCompleted {
                            id,
                            ok: !failed,
                            detail: part
                                .get("content")
                                .cloned()
                                .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                        });
                    }
                }
            }
        }
        Some("result") => {
            if let Some(usage) = value.get("usage") {
                events.push(ExternalAgentEvent::Usage {
                    usage: claude_usage(usage),
                });
            }
            // A failed Claude run can still exit zero, reporting the reason in
            // `result`. Exit status alone is not a health signal.
            let failed = value
                .get("is_error")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if failed {
                events.push(ExternalAgentEvent::Failed {
                    message: string_field(value, "result")
                        .unwrap_or_else(|| "CLI agent reported an error".to_owned()),
                });
            } else {
                events.push(ExternalAgentEvent::Completed);
            }
        }
        _ => {}
    }
    events
}

/// Claude reports `input_tokens` already exclusive of cache reads and writes,
/// so the counters are copied straight across.
pub(super) fn claude_usage(usage: &serde_json::Value) -> UsageDelta {
    let reasoning = usage
        .get("output_tokens_details")
        .map(|details| counter(details, "thinking_tokens"))
        .unwrap_or(0);
    UsageDelta::new()
        .with(CounterKind::InputUncached, counter(usage, "input_tokens"))
        .with(
            CounterKind::InputCached,
            counter(usage, "cache_read_input_tokens"),
        )
        .with(
            CounterKind::CacheWrite,
            counter(usage, "cache_creation_input_tokens"),
        )
        .with(CounterKind::Output, counter(usage, "output_tokens"))
        .with(CounterKind::Reasoning, reasoning)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_result_reporting_an_error_fails_even_with_a_clean_exit() {
        let value = serde_json::json!({
            "type": "result",
            "subtype": "success",
            "is_error": true,
            "result": "Not logged in · Please run /login",
            "usage": {"input_tokens": 1, "output_tokens": 0}
        });
        let events = normalize_claude(&value, &mut TurnState::default());
        assert!(events.iter().any(|event| matches!(
            event,
            ExternalAgentEvent::Failed { message } if message.contains("Not logged in")
        )));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ExternalAgentEvent::Completed))
        );
    }
}
