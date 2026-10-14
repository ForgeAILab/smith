//! Merge volatile CLI observations into canonical message blocks before budgeting.

use std::collections::BTreeMap;
use std::fmt::Write;

use agent_runtime::agent::external::{
    BackendToolObservation, BackendToolObservationKind, BackendToolObservations,
};
use agent_runtime::context::{CharRatioSizer, RequestSizer};
use agent_runtime_core::content::{ContentPart, Message, Role};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::ToolCallId;

use super::{TRANSCRIPT_CLOSE, TRANSCRIPT_OPEN};

const CLI_TOOL_LABEL: &str = "[tool run by the CLI itself, not dispatched by Smith]";
// The observation log applies this same byte bound before exposing JSON details.
const CLI_DETAIL_BYTES: usize = 8 * 1024;

fn render_parts(parts: &[ContentPart], rendered: &mut String, current_call_id: &ToolCallId) {
    for part in parts {
        match part {
            ContentPart::Text { text } => {
                rendered.push_str(text);
                rendered.push('\n');
            }
            ContentPart::Image { .. } => rendered.push_str("[image omitted]\n"),
            ContentPart::Reasoning { .. } => {}
            ContentPart::ToolCall(call) => {
                let current = if &call.id == current_call_id {
                    " — this is the consultation you are answering now"
                } else {
                    ""
                };
                let _ = writeln!(
                    rendered,
                    "Tool call: {} {}{current}",
                    call.name, call.arguments
                );
            }
            ContentPart::ToolResult(result) => {
                let error = if result.is_error { " [error]" } else { "" };
                let _ = writeln!(rendered, "Tool result: {}{error}", result.name);
                render_parts(&result.content, rendered, current_call_id);
            }
        }
    }
}

/// Renders one data-framed message, retaining the first user task and newest message.
pub(super) fn render_transcript(
    history: &[Message],
    input_budget: u32,
    current_call_id: &ToolCallId,
) -> Result<String, RuntimeError> {
    let rendered = history
        .iter()
        .map(|message| {
            let role = match message.role {
                Role::System => "System",
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::Tool => "Tool",
            };
            let mut text = format!("{role}:\n");
            render_parts(&message.content, &mut text, current_call_id);
            text.push('\n');
            text
        })
        .collect::<Vec<_>>();
    trim_transcript(history, rendered, input_budget)
}

pub(super) fn render_transcript_with_observations(
    history: &[Message],
    observations: &BackendToolObservations,
    input_budget: u32,
    current_call_id: &ToolCallId,
) -> Result<String, RuntimeError> {
    if observations.entries.is_empty() && observations.dropped == 0 {
        return render_transcript(history, input_budget, current_call_id);
    }
    // Markers name canonical calls, so earlier turns remain untouched even
    // when the current turn was an internal delivery without a user message.
    let mut before_call = BTreeMap::new();
    let mut pending = String::new();
    for entry in &observations.entries {
        if entry.kind == BackendToolObservationKind::BridgedCall {
            before_call.insert(entry.id.as_str(), std::mem::take(&mut pending));
        } else {
            render_observation(entry, &mut pending);
        }
    }
    let current_start = history
        .iter()
        .position(|message| {
            message.content.iter().any(|part| {
                matches!(part, ContentPart::ToolCall(call) if before_call.contains_key(call.id.as_str()))
            })
        })
        .unwrap_or_else(|| history.len().saturating_sub(1));
    let mut rendered = Vec::with_capacity(history.len());
    for (index, message) in history.iter().enumerate() {
        let role = match message.role {
            Role::System => "System",
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool",
        };
        let mut text = format!("{role}:\n");
        // Eviction removes an oldest prefix, including its position markers.
        // Report it at the earliest surviving anchor without altering a prior
        // turn when this call follows an internal delivery.
        if index == current_start && observations.dropped > 0 {
            let _ = writeln!(
                text,
                "[{} earlier CLI tool observations dropped]",
                observations.dropped
            );
        }
        for part in &message.content {
            if let ContentPart::ToolCall(call) = part
                && let Some(observed) = before_call.remove(call.id.as_str())
            {
                text.push_str(&observed);
            }
            render_parts(std::slice::from_ref(part), &mut text, current_call_id);
        }
        if index + 1 == history.len() {
            // The tail is after the last bridged round. Attaching it to the
            // newest block makes observations participate in message trimming.
            text.push_str(&pending);
        }
        text.push('\n');
        rendered.push(text);
    }
    trim_transcript(history, rendered, input_budget)
}

fn render_observation(entry: &BackendToolObservation, text: &mut String) {
    let name = if entry.name.is_empty() {
        entry.id.as_str()
    } else {
        entry.name.as_str()
    };
    let detail = bounded_detail(&entry.detail);
    match entry.kind {
        BackendToolObservationKind::Invocation => {
            let _ = writeln!(text, "{CLI_TOOL_LABEL} {name} input: {detail}");
        }
        BackendToolObservationKind::Outcome { ok } => {
            let outcome = if ok { "succeeded" } else { "failed" };
            let _ = writeln!(text, "{CLI_TOOL_LABEL} {name} {outcome} output: {detail}");
        }
        BackendToolObservationKind::BridgedCall => {}
    }
}

fn bounded_detail(detail: &serde_json::Value) -> String {
    let mut rendered = detail.to_string();
    if rendered.len() > CLI_DETAIL_BYTES {
        const MARKER: &str = "…[truncated]";
        let mut end = CLI_DETAIL_BYTES - MARKER.len();
        while !rendered.is_char_boundary(end) {
            end -= 1;
        }
        rendered.truncate(end);
        rendered.push_str(MARKER);
    }
    rendered
}

fn trim_transcript(
    history: &[Message],
    rendered: Vec<String>,
    input_budget: u32,
) -> Result<String, RuntimeError> {
    let sizer = CharRatioSizer::new();
    // Summed block estimates conservatively bound the single framed message
    // and keep trimming linear without re-rendering after each omission.
    let block_sizer = sizer.with_message_framing_tokens(0);
    let costs = rendered
        .iter()
        .map(|text| block_sizer.size_message(&Message::user(text)))
        .collect::<Vec<_>>();
    let mut cost = costs.iter().map(|cost| u64::from(*cost)).sum::<u64>();
    let framing = sizer.size_message(&Message::user(format!(
        "{TRANSCRIPT_OPEN}{TRANSCRIPT_CLOSE}"
    )));
    let keep_prefix = history
        .iter()
        .position(|message| message.role == Role::User)
        .map_or(0, |index| index + 1);
    let mut omitted = 0;
    loop {
        let marker = if omitted == 0 {
            String::new()
        } else {
            format!("[{omitted} earlier messages omitted]\n\n")
        };
        let marker_cost = block_sizer.size_message(&Message::user(&marker));
        if cost + u64::from(framing) + u64::from(marker_cost) <= u64::from(input_budget) {
            let mut transcript = TRANSCRIPT_OPEN.to_owned();
            for text in &rendered[..keep_prefix] {
                transcript.push_str(text);
            }
            transcript.push_str(&marker);
            for text in &rendered[keep_prefix + omitted..] {
                transcript.push_str(text);
            }
            transcript.push_str(TRANSCRIPT_CLOSE);
            return Ok(transcript);
        }
        let next = keep_prefix + omitted;
        if next >= history.len().saturating_sub(1) {
            return Err(RuntimeError::tool(
                "advisor input budget cannot fit the first user task and latest message",
            ));
        }
        cost = cost.saturating_sub(u64::from(costs[next]));
        omitted += 1;
    }
}
