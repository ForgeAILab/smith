//! What a running child is doing in its current task, for a parent that would
//! otherwise guess from `state: running` alone.
//!
//! Built from committed history and the live presentation boundary. Committed
//! content gets the transcript's redaction; a response still streaming is
//! reported by size and age only, because it may yet be discarded.

use std::collections::BTreeMap;
use std::time::Duration;

use agent_runtime::delegation::{ChildHistoryObservation, ChildHistorySnapshot};
use agent_runtime_core::content::{ContentPart, Message, Role};
use agent_runtime_core::event::{EventEnvelope, RuntimeEvent};
use serde_json::{Value, json};

use crate::journal::DefaultRedactor;

/// Longest assistant-text tail `last_step` carries.
const EXCERPT_CHARS: usize = 160;

/// The `activity` object for one running child.
pub(super) fn child_activity(
    snapshot: &ChildHistorySnapshot,
    redactor: &DefaultRedactor,
    elapsed: Option<Duration>,
    now_ms: u64,
) -> Value {
    activity(
        &snapshot.history,
        &snapshot.retained_events,
        &snapshot.pending_events,
        redactor,
        elapsed,
        now_ms,
    )
}

fn activity(
    history: &[Message],
    retained: &[ChildHistoryObservation],
    pending: &[EventEnvelope],
    redactor: &DefaultRedactor,
    elapsed: Option<Duration>,
    now_ms: u64,
) -> Value {
    let steps = task_steps(history, retained);
    let mut open = BTreeMap::new();
    for step in &steps {
        match step {
            Step::Call { id, tool } => {
                open.insert(id.clone(), tool.clone());
            }
            Step::Result { id, .. } => {
                open.remove(id);
            }
            _ => {}
        }
    }
    json!({
        "elapsed_ms": elapsed.map(|elapsed| elapsed.as_millis().min(u128::from(u64::MAX)) as u64),
        "steps_this_task": steps.len(),
        "last_step": steps.last().map(|step| step.json(&open, redactor)),
        "pending_tools": open
            .values()
            .map(|tool| redact(tool, redactor))
            .collect::<Vec<_>>(),
        "streaming": streaming(pending, now_ms),
    })
}

#[derive(Debug)]
enum Step {
    Text(String),
    Reasoning,
    Image,
    Call {
        id: String,
        tool: String,
    },
    Result {
        id: String,
        tool: Option<String>,
        is_error: bool,
    },
    HostedSearch,
}

impl Step {
    fn json(&self, open: &BTreeMap<String, String>, redactor: &DefaultRedactor) -> Value {
        match self {
            Step::Text(text) => json!({"kind": "text", "excerpt": excerpt(text, redactor)}),
            Step::Reasoning => json!({"kind": "reasoning"}),
            Step::Image => json!({"kind": "image"}),
            Step::Call { id, tool } => json!({
                "kind": "tool_call",
                "tool": redact(tool, redactor),
                "status": if open.contains_key(id) { "pending" } else { "done" },
            }),
            Step::Result { tool, is_error, .. } => json!({
                "kind": "tool_result",
                "tool": tool.as_deref().map(|tool| redact(tool, redactor)),
                "is_error": is_error,
            }),
            Step::HostedSearch => json!({"kind": "hosted_search"}),
        }
    }
}

/// Committed steps after the latest user message, interleaved with retained
/// backend observations the same way the transcript orders them.
fn task_steps(history: &[Message], retained: &[ChildHistoryObservation]) -> Vec<Step> {
    let start = history
        .iter()
        .rposition(|message| message.role == Role::User)
        .map_or(0, |index| index + 1);
    let mut observations = retained
        .iter()
        .filter(|observation| observation.history_index >= start)
        .collect::<Vec<_>>();
    observations.sort_by_key(|observation| (observation.history_index, observation.event.seq));
    let mut observations = observations.into_iter().peekable();
    let mut steps = Vec::new();
    for index in start..=history.len() {
        while let Some(observation) =
            observations.next_if(|observation| observation.history_index <= index)
        {
            steps.extend(observed_step(&observation.event.payload));
        }
        let Some(message) = history.get(index) else {
            break;
        };
        if message.role == Role::System {
            continue;
        }
        steps.extend(message.content.iter().filter_map(part_step));
    }
    steps.extend(observations.filter_map(|observation| observed_step(&observation.event.payload)));
    steps
}

fn part_step(part: &ContentPart) -> Option<Step> {
    Some(match part {
        ContentPart::Text { text } => Step::Text(text.clone()),
        ContentPart::Reasoning { redacted: true, .. } => return None,
        ContentPart::Reasoning { .. } => Step::Reasoning,
        ContentPart::Image { .. } => Step::Image,
        ContentPart::ToolCall(call) => Step::Call {
            id: call.id.to_string(),
            tool: call.name.clone(),
        },
        ContentPart::ToolResult(result) => Step::Result {
            id: result.call_id.to_string(),
            tool: Some(result.name.clone()),
            is_error: result.is_error,
        },
    })
}

fn observed_step(event: &RuntimeEvent) -> Option<Step> {
    Some(match event {
        RuntimeEvent::ExternalToolInvoked { id, name, .. } => Step::Call {
            id: id.clone(),
            tool: name.clone(),
        },
        RuntimeEvent::ExternalToolCompleted { id, ok, .. } => Step::Result {
            id: id.clone(),
            tool: None,
            is_error: !ok,
        },
        RuntimeEvent::HostedSearch { .. } => Step::HostedSearch,
        _ => return None,
    })
}

/// The response in flight, by size and age. Its content is speculative: an
/// output-limited or retried attempt is discarded, so none of it is quoted.
fn streaming(pending: &[EventEnvelope], now_ms: u64) -> Value {
    let started = pending
        .iter()
        .filter_map(|event| match &event.payload {
            RuntimeEvent::ProviderAttemptStarted { attempt, index, .. } => {
                Some((event, attempt, *index))
            }
            _ => None,
        })
        .max_by_key(|(event, ..)| event.seq);
    let (mut text_chars, mut reasoning_chars) = (0usize, 0usize);
    let mut first = started.map(|(event, ..)| event.timestamp.as_millis());
    let mut last = first;
    for event in pending {
        let chars = match &event.payload {
            RuntimeEvent::TextDelta { attempt, text, .. }
                if started.is_some_and(|(_, current, _)| current == attempt) =>
            {
                text_chars += text.chars().count();
                true
            }
            RuntimeEvent::ReasoningDelta {
                attempt,
                text,
                redacted: false,
                ..
            } if started.is_some_and(|(_, current, _)| current == attempt) => {
                reasoning_chars += text.chars().count();
                true
            }
            RuntimeEvent::ExternalText { text } => {
                text_chars += text.chars().count();
                true
            }
            RuntimeEvent::ExternalReasoning { text } => {
                reasoning_chars += text.chars().count();
                true
            }
            _ => false,
        };
        if chars {
            let at = event.timestamp.as_millis();
            first = Some(first.map_or(at, |first| first.min(at)));
            last = Some(last.map_or(at, |last| last.max(at)));
        }
    }
    let Some(first) = first else {
        return Value::Null;
    };
    json!({
        "attempt": started.map(|(_, _, index)| index.saturating_add(1)),
        "for_ms": now_ms.saturating_sub(first),
        "last_output_ms_ago": last.map(|last| now_ms.saturating_sub(last)),
        "text_chars": text_chars,
        "reasoning_chars": reasoning_chars,
    })
}

/// The tail of committed assistant text, whitespace-collapsed and redacted.
fn excerpt(text: &str, redactor: &DefaultRedactor) -> String {
    let text = redact(text, redactor)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let count = text.chars().count();
    if count <= EXCERPT_CHARS {
        return text;
    }
    let tail = text.chars().skip(count - EXCERPT_CHARS).collect::<String>();
    format!("…{tail}")
}

fn redact(text: &str, redactor: &DefaultRedactor) -> String {
    match redactor.redacted_clone(&Value::String(text.to_owned())) {
        Value::String(text) => text,
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::clock::Timestamp;
    use agent_runtime_core::content::{ToolCall, ToolResultBlock};
    use agent_runtime_core::ids::{AttemptId, EventId, RequestId, SessionId, ToolCallId};

    fn event(seq: u64, at: u64, payload: RuntimeEvent) -> EventEnvelope {
        EventEnvelope::new(
            seq,
            EventId::new(format!("e{seq}")),
            SessionId::new("child-session"),
            None,
            Timestamp(at),
            payload,
        )
    }

    fn call(id: &str, name: &str) -> ContentPart {
        ContentPart::ToolCall(ToolCall {
            id: ToolCallId::new(id),
            name: name.into(),
            arguments: json!({"path": "src/lib.rs"}),
        })
    }

    fn result(id: &str, name: &str, is_error: bool) -> Message {
        Message::tool_result(ToolResultBlock {
            call_id: ToolCallId::new(id),
            name: name.into(),
            content: vec![ContentPart::text("file body")],
            is_error,
        })
    }

    #[test]
    fn counts_only_the_current_task_and_pairs_tool_calls() {
        let redactor = DefaultRedactor::new();
        let history = vec![
            Message::user("first task"),
            Message::assistant(vec![ContentPart::text("first answer")]),
            Message::user("second task"),
            Message::assistant(vec![
                ContentPart::text("Reading both files."),
                call("c1", "read"),
                call("c2", "shell"),
            ]),
            result("c1", "read", false),
        ];
        let value = activity(
            &history,
            &[],
            &[],
            &redactor,
            Some(Duration::from_millis(4_200)),
            0,
        );
        assert_eq!(value["elapsed_ms"], 4_200);
        assert_eq!(value["steps_this_task"], 4, "{value}");
        assert_eq!(
            value["last_step"],
            json!({"kind": "tool_result", "tool": "read", "is_error": false})
        );
        assert_eq!(value["pending_tools"], json!(["shell"]));
        assert!(value["streaming"].is_null());
    }

    #[test]
    fn a_trailing_call_is_pending_and_text_is_a_redacted_tail() {
        let redactor = DefaultRedactor::new().with_secret("TOP_SECRET");
        let long = format!(
            "{} key TOP_SECRET then\n\nwrite the document.",
            "x".repeat(400)
        );
        let history = vec![
            Message::user("task"),
            Message::assistant(vec![ContentPart::text(long)]),
        ];
        let text = activity(&history, &[], &[], &redactor, None, 0);
        let excerpt = text["last_step"]["excerpt"].as_str().unwrap();
        assert!(excerpt.starts_with('…') && excerpt.ends_with("then write the document."));
        assert!(excerpt.chars().count() <= EXCERPT_CHARS + 1);
        assert!(!excerpt.contains("TOP_SECRET"), "{excerpt}");
        assert!(text["elapsed_ms"].is_null());

        let history = vec![
            Message::user("task"),
            Message::assistant(vec![call("c1", "edit")]),
        ];
        let pending = activity(&history, &[], &[], &redactor, None, 0);
        assert_eq!(
            pending["last_step"],
            json!({"kind": "tool_call", "tool": "edit", "status": "pending"})
        );
        assert_eq!(pending["pending_tools"], json!(["edit"]));
    }

    #[test]
    fn streaming_reports_sizes_and_age_of_the_latest_attempt_only() {
        let redactor = DefaultRedactor::new();
        let request = RequestId::new("r1");
        let (old, new) = (AttemptId::new("a1"), AttemptId::new("a2"));
        let pending = vec![
            event(
                1,
                1_000,
                RuntimeEvent::ProviderAttemptStarted {
                    request: request.clone(),
                    attempt: old.clone(),
                    index: 0,
                    model: "m".into(),
                },
            ),
            event(
                2,
                1_500,
                RuntimeEvent::TextDelta {
                    request: request.clone(),
                    attempt: old,
                    text: "discarded draft".into(),
                },
            ),
            event(
                3,
                2_000,
                RuntimeEvent::ProviderAttemptStarted {
                    request: request.clone(),
                    attempt: new.clone(),
                    index: 1,
                    model: "m".into(),
                },
            ),
            event(
                4,
                2_500,
                RuntimeEvent::ReasoningDelta {
                    request: request.clone(),
                    attempt: new.clone(),
                    text: "thinking".into(),
                    redacted: false,
                },
            ),
            event(
                5,
                3_000,
                RuntimeEvent::TextDelta {
                    request,
                    attempt: new,
                    text: "SPECULATIVE".into(),
                },
            ),
        ];
        let value = activity(
            &[Message::user("task")],
            &[],
            &pending,
            &redactor,
            None,
            10_000,
        );
        assert_eq!(
            value["streaming"],
            json!({
                "attempt": 2,
                "for_ms": 8_000,
                "last_output_ms_ago": 7_000,
                "text_chars": 11,
                "reasoning_chars": 8,
            })
        );
        assert!(!value.to_string().contains("SPECULATIVE"), "{value}");
        assert_eq!(value["steps_this_task"], 0);
        assert!(value["last_step"].is_null());
    }

    #[test]
    fn backend_tool_observations_count_as_current_steps() {
        let redactor = DefaultRedactor::new();
        let history = vec![Message::user("earlier"), Message::user("task")];
        let observation = |index, seq, payload| ChildHistoryObservation {
            history_index: index,
            event: event(seq, 0, payload),
        };
        let retained = vec![
            observation(
                1,
                1,
                RuntimeEvent::ExternalToolInvoked {
                    id: "old".into(),
                    name: "Read".into(),
                    detail: json!({}),
                },
            ),
            observation(
                2,
                2,
                RuntimeEvent::ExternalToolInvoked {
                    id: "n1".into(),
                    name: "Bash".into(),
                    detail: json!({"command": "cargo test"}),
                },
            ),
        ];
        let value = activity(&history, &retained, &[], &redactor, None, 0);
        assert_eq!(value["steps_this_task"], 1, "{value}");
        assert_eq!(value["pending_tools"], json!(["Bash"]));
        assert_eq!(value["last_step"]["status"], "pending");
    }
}
