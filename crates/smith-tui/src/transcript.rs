//! The transcript model.
//!
//! The transcript is a list of [`Block`]s built by folding the runtime's event
//! stream. It holds *what happened*, never how it is drawn — wrapping, width,
//! and scrolling belong to the renderer, so a resize cannot corrupt history.
//!
//! Two behaviors matter more than they look:
//!
//! - **Text deltas append to the open assistant block** rather than creating a
//!   block each. A provider that streams token-by-token would otherwise produce
//!   thousands of blocks for one reply.
//! - **Background notices never merge into an assistant block.** A monitor line
//!   arriving mid-stream gets its own block, so the transcript stays a faithful
//!   record of the conversation rather than a splice of unrelated output.

use std::time::Instant;

use agent_runtime_core::content::{ContentPart, Message, Role};
use serde_json::Value;
use smith_tools::{ToolCallDisplay, has_tool_call_display_schema, project_tool_call_display};

pub(crate) const MAX_LOCAL_RESULT_BYTES: usize = 512 * 1024;
const MAX_LOCAL_RESULT_LINES: usize = 4_096;
const MAX_LOCAL_RESULT_TITLE_CHARS: usize = 96;

/// Semantic state of a local command result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalResultState {
    /// Informational output.
    Info,
    /// A successful command with no matching data.
    Empty,
    /// A local command that could not produce its result.
    Error,
}

/// How a tool call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// Requested, and still running.
    Running,
    /// Completed successfully.
    Ok,
    /// Completed with an error.
    Failed,
    /// Denied by the approval gate.
    Denied,
    /// The call finished and its outcome was never reported.
    ///
    /// A delegated child's progress crosses the runtime boundary as
    /// identifiers only, so the parent learns that a tool ran and nothing
    /// about how it ended. Claiming `ok` there would invent a result.
    Unreported,
}

impl ToolStatus {
    /// The word rendered beside the tool row. Paired with color, never
    /// replaced by it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Denied => "denied",
            Self::Unreported => "ran",
        }
    }
}

/// One addressable unit of transcript history.
#[derive(Debug, Clone)]
pub enum Block {
    /// A message the user sent.
    User {
        /// The message text.
        text: String,
    },
    /// Model output. `open` marks the block currently receiving deltas.
    Assistant {
        /// The accumulated text.
        text: String,
        /// Whether more deltas may still arrive.
        open: bool,
    },
    /// Model reasoning.
    Reasoning {
        /// The accumulated reasoning text.
        text: String,
        /// Whether the provider redacted it.
        redacted: bool,
        /// Whether more deltas may still arrive.
        open: bool,
    },
    /// A tool call and its outcome.
    Tool {
        /// The tool-call id, used to match the completion event.
        call_id: String,
        /// The tool name.
        name: String,
        /// Reviewed built-in target metadata, when a safe projector exists.
        display: Option<ToolCallDisplay>,
        /// Value-free fallback derived from protected argument keys.
        protected_summary: String,
        /// The current status.
        status: ToolStatus,
        /// Bounded, credential-redacted first lines of the tool result,
        /// supplied by the host after completion.
        result_preview: Option<String>,
        /// When the tool call started running.
        started_at: Option<Instant>,
    },
    /// A structured error.
    Error {
        /// The redacted message.
        message: String,
    },
    /// A background notification, or a runtime notice such as a provider
    /// change.
    Notice {
        /// The source, e.g. a monitor name or `provider`.
        source: String,
        /// The notice text.
        text: String,
    },
    /// Read-only host information shown locally and excluded from canonical
    /// provider conversation history.
    LocalResult {
        /// Command or result title.
        title: String,
        /// Bounded display content.
        content: String,
        /// Text-visible result state.
        state: LocalResultState,
    },
}

impl PartialEq for Block {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::User { text: t1 }, Self::User { text: t2 }) => t1 == t2,
            (Self::Assistant { text: t1, open: o1 }, Self::Assistant { text: t2, open: o2 }) => {
                t1 == t2 && o1 == o2
            }
            (
                Self::Reasoning {
                    text: t1,
                    redacted: r1,
                    open: o1,
                },
                Self::Reasoning {
                    text: t2,
                    redacted: r2,
                    open: o2,
                },
            ) => t1 == t2 && r1 == r2 && o1 == o2,
            (
                Self::Tool {
                    call_id: c1,
                    name: n1,
                    display: d1,
                    protected_summary: p1,
                    status: s1,
                    result_preview: r1,
                    ..
                },
                Self::Tool {
                    call_id: c2,
                    name: n2,
                    display: d2,
                    protected_summary: p2,
                    status: s2,
                    result_preview: r2,
                    ..
                },
            ) => c1 == c2 && n1 == n2 && d1 == d2 && p1 == p2 && s1 == s2 && r1 == r2,
            (Self::Error { message: m1 }, Self::Error { message: m2 }) => m1 == m2,
            (
                Self::Notice {
                    source: s1,
                    text: t1,
                },
                Self::Notice {
                    source: s2,
                    text: t2,
                },
            ) => s1 == s2 && t1 == t2,
            (
                Self::LocalResult {
                    title: t1,
                    content: c1,
                    state: s1,
                },
                Self::LocalResult {
                    title: t2,
                    content: c2,
                    state: s2,
                },
            ) => t1 == t2 && c1 == c2 && s1 == s2,
            _ => false,
        }
    }
}

/// The ordered transcript.
#[derive(Debug, Clone, Default)]
pub struct Transcript {
    blocks: Vec<Block>,
}

impl Transcript {
    /// An empty transcript.
    pub fn new() -> Self {
        Self::default()
    }

    /// The blocks, oldest first.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The number of blocks.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether nothing has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Appends a user message.
    pub fn push_user(&mut self, text: impl Into<String>) {
        self.close_open();
        self.blocks.push(Block::User { text: text.into() });
    }

    /// Appends a notice, which never merges with adjacent blocks.
    pub fn push_notice(&mut self, source: impl Into<String>, text: impl Into<String>) {
        self.blocks.push(Block::Notice {
            source: source.into(),
            text: text.into(),
        });
    }

    /// Appends an error.
    pub fn push_error(&mut self, message: impl Into<String>) {
        self.close_open();
        self.blocks.push(Block::Error {
            message: message.into(),
        });
    }

    /// Appends bounded local command output without representing it as model
    /// conversation history.
    pub fn push_local_result(
        &mut self,
        title: impl Into<String>,
        content: impl Into<String>,
        state: LocalResultState,
    ) {
        self.close_open();
        let title = title
            .into()
            .replace(['\r', '\n'], " ")
            .chars()
            .take(MAX_LOCAL_RESULT_TITLE_CHARS)
            .collect();
        let content = content.into();
        let (content, state) = if content.trim().is_empty() {
            ("No output.".to_owned(), LocalResultState::Empty)
        } else {
            (bound_local_result(content), state)
        };
        self.blocks.push(Block::LocalResult {
            title,
            content,
            state,
        });
    }

    /// Appends assistant text, extending the open assistant block if there is
    /// one.
    pub fn push_text_delta(&mut self, delta: &str) {
        if let Some(Block::Assistant { text, open: true }) = self.blocks.last_mut() {
            text.push_str(delta);
            return;
        }
        self.close_open();
        self.blocks.push(Block::Assistant {
            text: delta.to_owned(),
            open: true,
        });
    }

    /// Appends reasoning text, extending the open reasoning block if there is
    /// one and its redaction flag matches.
    pub fn push_reasoning_delta(&mut self, delta: &str, delta_redacted: bool) {
        if let Some(Block::Reasoning {
            text,
            redacted,
            open: true,
        }) = self.blocks.last_mut()
            && *redacted == delta_redacted
        {
            text.push_str(delta);
            return;
        }
        self.close_open();
        self.blocks.push(Block::Reasoning {
            text: delta.to_owned(),
            redacted: delta_redacted,
            open: true,
        });
    }

    /// Records a requested tool call.
    ///
    /// Runtime events keep argument values protected by default. A caller may
    /// supply a credential-redacted canonical clone for the explicit built-in
    /// projector; arbitrary values are never summarized generically.
    pub fn push_tool_call(
        &mut self,
        call_id: impl Into<String>,
        name: &str,
        arguments: Option<&Value>,
        argument_keys: &[String],
    ) {
        self.close_open();
        self.blocks.push(Block::Tool {
            call_id: call_id.into(),
            name: name.to_owned(),
            display: arguments.and_then(|arguments| project_tool_call_display(name, arguments)),
            protected_summary: summarize_unavailable_arguments(name, argument_keys),
            status: ToolStatus::Running,
            result_preview: None,
            started_at: Some(Instant::now()),
        });
    }

    /// Records a tool call that is already over and whose outcome nobody
    /// reported.
    ///
    /// This is how a delegated child's tool activity enters a transcript: the
    /// runtime carries the name across the boundary and nothing else, so
    /// there is no call id to match a completion against and no arguments to
    /// project. It is a finished row on arrival.
    pub fn push_unreported_tool_call(&mut self, name: &str) {
        self.close_open();
        self.blocks.push(Block::Tool {
            call_id: String::new(),
            name: name.to_owned(),
            display: None,
            protected_summary: String::new(),
            status: ToolStatus::Unreported,
            result_preview: None,
            started_at: None,
        });
    }

    /// Drops the oldest blocks until at most `max` remain.
    ///
    /// The root transcript is the session and is never trimmed. A child's is
    /// a bounded tail the client keeps in memory on the child's behalf, so it
    /// has a ceiling.
    pub fn retain_newest(&mut self, max: usize) {
        if self.blocks.len() > max {
            self.blocks.drain(..self.blocks.len() - max);
        }
    }

    /// Adds a reviewed local display projection to an existing live call.
    ///
    /// This is deliberately separate from [`RuntimeEvent`](agent_runtime_core::event::RuntimeEvent)
    /// folding so protected event and journal payloads do not need to carry
    /// argument values.
    pub fn set_tool_display(&mut self, call_id: &str, display: ToolCallDisplay) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Tool {
                call_id: id,
                display: slot,
                ..
            } = block
                && id == call_id
            {
                *slot = Some(display);
                return;
            }
        }
    }

    /// Attaches a bounded, credential-redacted result preview to a call.
    ///
    /// Like [`Self::set_tool_display`], this is host-supplied enrichment: the
    /// protected event stream never carries result content, so the host reads
    /// canonical history, redacts it, and hands the transcript only what the
    /// row shows. Input is re-bounded here so no caller can flood a frame.
    pub fn set_tool_result_preview(&mut self, call_id: &str, preview: impl AsRef<str>) {
        let Some(preview) = bound_result_preview(preview.as_ref()) else {
            return;
        };
        for block in self.blocks.iter_mut().rev() {
            if let Block::Tool {
                call_id: id,
                result_preview: slot,
                ..
            } = block
                && id == call_id
            {
                *slot = Some(preview);
                return;
            }
        }
    }

    /// Marks a tool call finished. Unknown ids are ignored rather than
    /// fabricating a block for a call the transcript never saw.
    pub fn complete_tool_call(&mut self, call_id: &str, status: ToolStatus) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Tool {
                call_id: id,
                status: slot,
                ..
            } = block
                && id == call_id
            {
                *slot = status;
                return;
            }
        }
    }

    /// Marks the most recent still-running call of `name` finished.
    ///
    /// The approval gate denies by tool name — it fires before the runtime
    /// emits a completion — so the row cannot be matched by call id.
    pub fn complete_tool_call_by_name(&mut self, name: &str, status: ToolStatus) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Tool {
                name: candidate,
                status: slot,
                ..
            } = block
                && candidate == name
                && *slot == ToolStatus::Running
            {
                *slot = status;
                return;
            }
        }
    }

    /// Closes any block still receiving deltas. Called at turn boundaries so a
    /// later delta starts a new block instead of extending a finished reply.
    pub fn close_open(&mut self) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Assistant { open, .. } | Block::Reasoning { open, .. } = block {
                *open = false;
            }
        }
    }

    /// Replaces the transcript with one rebuilt from canonical history.
    ///
    /// Used when resuming a session: history is the source of truth, and any
    /// live-only blocks (notices, in-flight tools) are intentionally dropped.
    pub fn replace_from_history(&mut self, history: &[Message]) {
        self.blocks.clear();
        for message in history {
            match message.role {
                Role::User => self.push_user(user_display_text(message)),
                Role::System => {}
                Role::Assistant => {
                    for part in &message.content {
                        match part {
                            ContentPart::Text { text } => {
                                self.blocks.push(Block::Assistant {
                                    text: text.clone(),
                                    open: false,
                                });
                            }
                            ContentPart::Reasoning { text, redacted, .. } => {
                                self.blocks.push(Block::Reasoning {
                                    text: text.clone(),
                                    redacted: *redacted,
                                    open: false,
                                });
                            }
                            ContentPart::ToolCall(call) => {
                                let argument_keys = argument_keys(&call.arguments);
                                self.blocks.push(Block::Tool {
                                    call_id: call.id.as_str().to_owned(),
                                    name: call.name.clone(),
                                    // Canonical history is intentionally not
                                    // projected here. The host supplies a
                                    // credential-redacted display clone after
                                    // rebuilding the transcript.
                                    display: None,
                                    protected_summary: summarize_unavailable_arguments(
                                        &call.name,
                                        &argument_keys,
                                    ),
                                    // History records the call; the matching
                                    // result below supplies the outcome.
                                    status: ToolStatus::Running,
                                    // As with `display`, the host supplies a
                                    // redacted preview after rebuilding.
                                    result_preview: None,
                                    started_at: None,
                                });
                            }
                            // An assistant message does not carry results;
                            // those arrive under the tool role below.
                            ContentPart::Image { .. } | ContentPart::ToolResult(_) => {}
                        }
                    }
                }
                Role::Tool => {
                    for part in &message.content {
                        if let ContentPart::ToolResult(result) = part {
                            let status = if result.is_error {
                                ToolStatus::Failed
                            } else {
                                ToolStatus::Ok
                            };
                            self.complete_tool_call(result.call_id.as_str(), status);
                        }
                    }
                }
            }
        }
    }
}

/// Text projection of a user message, marking image parts in place so a
/// resumed transcript still shows that an image travelled with the turn.
fn user_display_text(message: &Message) -> String {
    let mut text = String::new();
    for part in &message.content {
        let rendered = match part {
            ContentPart::Text { text } => text.as_str(),
            ContentPart::Image { .. } => "[image]",
            _ => continue,
        };
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(rendered);
    }
    text
}

const MAX_PREVIEW_LINES: usize = 3;
const MAX_PREVIEW_LINE_CHARS: usize = 160;

/// Bounds a raw result to its first non-blank lines, one-line-safe and
/// control-free, with an honest `… +N lines` tail when content was dropped.
fn bound_result_preview(raw: &str) -> Option<String> {
    let mut lines = raw
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty());
    let mut kept = Vec::new();
    let mut dropped = 0usize;
    for line in lines.by_ref() {
        if kept.len() < MAX_PREVIEW_LINES {
            kept.push(sanitize_preview_line(line));
        } else {
            dropped += 1;
        }
    }
    if kept.is_empty() {
        return None;
    }
    if dropped > 0 {
        kept.push(format!(
            "… +{dropped} more line{}",
            if dropped == 1 { "" } else { "s" }
        ));
    }
    Some(kept.join("\n"))
}

fn sanitize_preview_line(line: &str) -> String {
    let mut sanitized = String::new();
    let mut chars = 0usize;
    for character in line.chars() {
        if chars == MAX_PREVIEW_LINE_CHARS {
            sanitized.push('…');
            break;
        }
        // The same unsafe-control set the display projector strips: C0/C1
        // plus zero-width and bidi override codepoints.
        if character.is_control()
            || matches!(
                character,
                '\u{200b}'..='\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2060}'..='\u{206f}'
                    | '\u{feff}'
            )
        {
            continue;
        }
        sanitized.push(character);
        chars += 1;
    }
    sanitized
}

fn bound_local_result(content: String) -> String {
    let mut bounded = String::with_capacity(content.len().min(MAX_LOCAL_RESULT_BYTES));
    let mut lines = 1;
    let mut truncated = false;
    for character in content.chars() {
        if bounded.len() + character.len_utf8() > MAX_LOCAL_RESULT_BYTES
            || (character == '\n' && lines >= MAX_LOCAL_RESULT_LINES)
        {
            truncated = true;
            break;
        }
        bounded.push(character);
        if character == '\n' {
            lines += 1;
        }
    }
    if truncated {
        if !bounded.ends_with('\n') {
            bounded.push('\n');
        }
        bounded.push_str("[local result truncated at the display limit]");
    }
    bounded
}

/// A stable, value-free fallback when no reviewed projection is available.
fn summarize_unavailable_arguments(name: &str, argument_keys: &[String]) -> String {
    let reason = if has_tool_call_display_schema(name) {
        "details unavailable"
    } else {
        "arguments hidden"
    };
    if argument_keys.is_empty() {
        reason.to_owned()
    } else {
        const MAX_KEYS: usize = 6;
        let mut keys = argument_keys
            .iter()
            .take(MAX_KEYS)
            .map(|key| {
                let normalized = key
                    .chars()
                    .take(32)
                    .map(|character| {
                        if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
                        {
                            character
                        } else {
                            '_'
                        }
                    })
                    .collect::<String>();
                if normalized.is_empty() {
                    "?".to_owned()
                } else {
                    normalized
                }
            })
            .collect::<Vec<_>>();
        if argument_keys.len() > MAX_KEYS {
            keys.push("…".to_owned());
        }
        format!("{} · {reason}", keys.join(", "))
    }
}

/// Extracts the same sorted top-level key view the runtime emits.
fn argument_keys(arguments: &Value) -> Vec<String> {
    let mut keys = arguments
        .as_object()
        .map(|map| map.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::content::{ToolCall, ToolResultBlock};
    use agent_runtime_core::ids::ToolCallId;
    use serde_json::json;

    #[test]
    fn streamed_deltas_accumulate_into_one_block() {
        let mut transcript = Transcript::new();
        for delta in ["The ", "retry ", "policy"] {
            transcript.push_text_delta(delta);
        }
        assert_eq!(transcript.len(), 1);
        assert_eq!(
            transcript.blocks()[0],
            Block::Assistant {
                text: "The retry policy".into(),
                open: true
            }
        );
    }

    #[test]
    fn a_closed_block_does_not_absorb_the_next_reply() {
        let mut transcript = Transcript::new();
        transcript.push_text_delta("first");
        transcript.close_open();
        transcript.push_text_delta("second");
        assert_eq!(transcript.len(), 2);
    }

    #[test]
    fn a_notice_never_splices_into_a_streaming_reply() {
        let mut transcript = Transcript::new();
        transcript.push_text_delta("analyzing");
        transcript.push_notice("monitor:build", "error[E0433]: failed to resolve");
        transcript.push_text_delta(" the failure");

        // The notice stands alone, and the reply resumes in a fresh block
        // rather than having the monitor line spliced into its text.
        assert_eq!(transcript.len(), 3);
        assert!(matches!(
            transcript.blocks()[0],
            Block::Assistant { open: false, .. }
        ));
        assert!(matches!(transcript.blocks()[1], Block::Notice { .. }));
        match &transcript.blocks()[2] {
            Block::Assistant { text, .. } => assert_eq!(text, " the failure"),
            other => panic!("expected an assistant block, got {other:?}"),
        }
    }

    #[test]
    fn local_results_are_bounded_and_never_merge_with_model_output() {
        let mut transcript = Transcript::new();
        transcript.push_text_delta("answer");
        transcript.push_local_result(
            "diff\ninjected",
            "x".repeat(MAX_LOCAL_RESULT_BYTES + 16),
            LocalResultState::Info,
        );
        transcript.push_text_delta("next");

        assert_eq!(transcript.len(), 3);
        match &transcript.blocks()[1] {
            Block::LocalResult { title, content, .. } => {
                assert_eq!(title, "diff injected");
                assert!(content.ends_with("[local result truncated at the display limit]"));
            }
            other => panic!("expected a local result, got {other:?}"),
        }
        assert!(matches!(
            transcript.blocks()[2],
            Block::Assistant { open: true, .. }
        ));
    }

    #[test]
    fn reasoning_and_text_occupy_separate_blocks() {
        let mut transcript = Transcript::new();
        transcript.push_reasoning_delta("considering", false);
        transcript.push_text_delta("answer");
        assert_eq!(transcript.len(), 2);
        assert!(matches!(transcript.blocks()[0], Block::Reasoning { .. }));
    }

    #[test]
    fn redacted_reasoning_starts_a_new_block() {
        let mut transcript = Transcript::new();
        transcript.push_reasoning_delta("visible", false);
        transcript.push_reasoning_delta("hidden", true);
        assert_eq!(transcript.len(), 2);
    }

    #[test]
    fn a_completion_updates_the_matching_tool_row() {
        let mut transcript = Transcript::new();
        transcript.push_tool_call(
            "c1",
            "read",
            Some(&json!({"path": "src/retry.rs"})),
            &["path".into()],
        );
        transcript.push_tool_call(
            "c2",
            "shell",
            Some(&json!({"command": "cargo test"})),
            &["command".into()],
        );
        transcript.complete_tool_call("c1", ToolStatus::Ok);
        transcript.complete_tool_call("c2", ToolStatus::Failed);

        match &transcript.blocks()[0] {
            Block::Tool {
                display, status, ..
            } => {
                assert_eq!(
                    display.as_ref().map(ToolCallDisplay::invocation),
                    Some("Read(src/retry.rs)".to_owned())
                );
                assert_eq!(*status, ToolStatus::Ok);
            }
            other => panic!("expected a tool block, got {other:?}"),
        }
        match &transcript.blocks()[1] {
            Block::Tool { status, .. } => assert_eq!(*status, ToolStatus::Failed),
            other => panic!("expected a tool block, got {other:?}"),
        }
    }

    #[test]
    fn result_previews_are_bounded_sanitized_and_matched_by_id() {
        let mut transcript = Transcript::new();
        transcript.push_tool_call(
            "c1",
            "registry.search",
            Some(&json!({"query": "browser", "max_results": 2})),
            &["max_results".into(), "query".into()],
        );
        transcript.complete_tool_call("c1", ToolStatus::Ok);
        transcript.set_tool_result_preview(
            "c1",
            "\nfirst card\u{202e}\nsecond card\n\nthird card\nfourth card\nfifth card\n",
        );
        transcript.set_tool_result_preview("unknown", "never lands");
        transcript.set_tool_result_preview("c1", "   \n \n");

        match &transcript.blocks()[0] {
            Block::Tool {
                result_preview: Some(preview),
                ..
            } => {
                assert_eq!(
                    preview,
                    "first card\nsecond card\nthird card\n… +2 more lines"
                );
            }
            other => panic!("expected a tool block with a preview, got {other:?}"),
        }
    }

    #[test]
    fn completing_an_unknown_call_changes_nothing() {
        let mut transcript = Transcript::new();
        transcript.push_tool_call(
            "c1",
            "read",
            Some(&json!({"path": "a.rs"})),
            &["path".into()],
        );
        let before = transcript.blocks().to_vec();
        transcript.complete_tool_call("nonexistent", ToolStatus::Ok);
        assert_eq!(transcript.blocks(), before.as_slice());
    }

    #[test]
    fn unavailable_details_show_keys_and_an_honest_reason() {
        let mut transcript = Transcript::new();
        transcript.push_tool_call("c1", "shell", None, &["command".into(), "cwd".into()]);

        match &transcript.blocks()[0] {
            Block::Tool {
                display,
                protected_summary,
                ..
            } => {
                assert!(display.is_none());
                assert_eq!(protected_summary, "command, cwd · details unavailable");
                assert!(!protected_summary.contains("cargo test"));
            }
            other => panic!("expected a tool block, got {other:?}"),
        }
    }

    #[test]
    fn credential_redacted_arguments_use_only_the_reviewed_projector() {
        let mut transcript = Transcript::new();
        transcript.push_tool_call(
            "c1",
            "shell",
            Some(&json!({
                "command": "printf [redacted]",
                "cwd": "crates/smith-cli",
                "unknown": "omitted"
            })),
            &["command".into(), "cwd".into(), "unknown".into()],
        );

        let Block::Tool { display, .. } = &transcript.blocks()[0] else {
            panic!("expected a tool block");
        };
        let invocation = display.as_ref().expect("safe projection").invocation();
        assert_eq!(
            invocation,
            "Shell(printf [redacted] · cwd crates/smith-cli)"
        );
        assert!(!invocation.contains("omitted"));
    }

    #[test]
    fn history_replay_reconstructs_calls_with_their_outcomes() {
        let history = vec![
            Message::user("read the retry policy"),
            Message::assistant(vec![
                ContentPart::text("Looking."),
                ContentPart::ToolCall(ToolCall {
                    id: ToolCallId::new("c1"),
                    name: "read".into(),
                    arguments: json!({"path": "src/retry.rs"}),
                }),
            ]),
            Message::tool_result(ToolResultBlock {
                call_id: ToolCallId::new("c1"),
                name: "read".into(),
                content: vec![ContentPart::text("fn retry() {}")],
                is_error: false,
            }),
        ];

        let mut transcript = Transcript::new();
        transcript.push_notice("stale", "dropped on replay");
        transcript.push_local_result("status", "model: old", LocalResultState::Info);
        transcript.replace_from_history(&history);

        assert_eq!(transcript.len(), 3);
        assert!(matches!(transcript.blocks()[0], Block::User { .. }));
        match &transcript.blocks()[2] {
            Block::Tool {
                status,
                name,
                display,
                protected_summary,
                ..
            } => {
                assert_eq!(name, "read");
                assert_eq!(*status, ToolStatus::Ok);
                assert!(display.is_none());
                assert_eq!(protected_summary, "path · details unavailable");
            }
            other => panic!("expected a tool block, got {other:?}"),
        }
    }

    #[test]
    fn live_enrichment_and_history_replay_have_built_in_and_unknown_parity() {
        let cases = [
            (
                "read",
                json!({"path": "src/lib.rs", "offset": 4, "limit": 2}),
                Some("Read(src/lib.rs · offset 4 · limit 2)"),
            ),
            (
                "list",
                json!({"recursive": true}),
                Some("List(. · recursive)"),
            ),
            (
                "search",
                json!({"pattern": "needle", "path": "crates"}),
                Some("Search(\"needle\" · crates)"),
            ),
            (
                "edit",
                json!({
                    "path": "src/lib.rs",
                    "old_string": "before",
                    "new_string": "after"
                }),
                Some("Edit(src/lib.rs)"),
            ),
            (
                "shell",
                json!({"command": "cargo test", "cwd": "crates"}),
                Some("Shell(cargo test · cwd crates)"),
            ),
            ("third_party", json!({"path": "TOP_SECRET_UNKNOWN"}), None),
        ];

        for (index, (name, arguments, expected)) in cases.into_iter().enumerate() {
            let call_id = format!("call-{index}");
            let keys = argument_keys(&arguments);
            let mut live = Transcript::new();
            live.push_tool_call(&call_id, name, None, &keys);
            if let Some(display) = project_tool_call_display(name, &arguments) {
                live.set_tool_display(&call_id, display);
            }
            live.complete_tool_call(&call_id, ToolStatus::Ok);

            let history = vec![
                Message::assistant(vec![ContentPart::ToolCall(ToolCall {
                    id: ToolCallId::new(&call_id),
                    name: name.to_owned(),
                    arguments: arguments.clone(),
                })]),
                Message::tool_result(ToolResultBlock {
                    call_id: ToolCallId::new(&call_id),
                    name: name.to_owned(),
                    content: vec![ContentPart::text("TOP_SECRET_RESULT")],
                    is_error: false,
                }),
            ];
            let mut replay = Transcript::new();
            replay.replace_from_history(&history);
            if let Some(display) = project_tool_call_display(name, &arguments) {
                replay.set_tool_display(&call_id, display);
            }

            let (
                Block::Tool {
                    display: live_display,
                    protected_summary: live_fallback,
                    status: live_status,
                    ..
                },
                Block::Tool {
                    display: replay_display,
                    protected_summary: replay_fallback,
                    status: replay_status,
                    ..
                },
            ) = (&live.blocks()[0], &replay.blocks()[0])
            else {
                panic!("expected matching tool blocks for {name}");
            };
            assert_eq!(live_display, replay_display, "{name}");
            assert_eq!(live_fallback, replay_fallback, "{name}");
            assert_eq!(live_status, replay_status, "{name}");
            assert_eq!(
                live_display.as_ref().map(ToolCallDisplay::invocation),
                expected.map(str::to_owned),
                "{name}"
            );
        }
    }

    #[test]
    fn replayed_user_images_keep_a_visible_marker() {
        let history = vec![Message {
            role: Role::User,
            content: vec![
                ContentPart::text("what is this?"),
                ContentPart::Image {
                    url: "data:image/png;base64,SECRETPIXELS".into(),
                    detail: None,
                },
            ],
        }];
        let mut transcript = Transcript::new();
        transcript.replace_from_history(&history);

        match &transcript.blocks()[0] {
            Block::User { text } => {
                assert_eq!(text, "what is this?\n[image]");
                assert!(!text.contains("SECRETPIXELS"));
            }
            other => panic!("expected a user block, got {other:?}"),
        }
    }

    #[test]
    fn system_messages_stay_out_of_the_transcript() {
        let mut transcript = Transcript::new();
        transcript.replace_from_history(&[Message::system("be concise"), Message::user("hi")]);
        assert_eq!(transcript.len(), 1);
        assert!(matches!(transcript.blocks()[0], Block::User { .. }));
    }
}
