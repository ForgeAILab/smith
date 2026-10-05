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

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use agent_runtime_core::content::{ContentPart, Message, Role};
use serde_json::Value;
use smith_client::agent_report::{AgentReport, AgentResumeReport};
use smith_client::diff_report::{DiffLine, DiffLineKind, DiffOutcome, DiffReport};
pub use smith_client::local_result::{LocalResult, LocalResultState};
use smith_client::message_report::MessageReport;
#[cfg(test)]
use smith_client::recovery_report::RecoveryReport;
use smith_client::review_report::{ReviewReport, ReviewStartReport};
use smith_client::shell_report::ShellOutput;
use smith_client::{NoticeKind, NoticePersistence};
use smith_tools::{ToolCallDisplay, project_external_tool_call_display, project_tool_call_display};

pub(crate) const MAX_LOCAL_RESULT_BYTES: usize = 512 * 1024;
const MAX_LOCAL_RESULT_LINES: usize = 4_096;
const MAX_LOCAL_RESULT_TITLE_CHARS: usize = 96;

static NEXT_BLOCK_REVISION: AtomicU64 = AtomicU64::new(1);

/// Display-ready shortcut restored by the host at a history boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoredShellShortcut {
    /// Number of history messages preceding the shortcut.
    pub anchor: usize,
    /// Runtime call identity, when one was assigned.
    pub call: Option<String>,
    /// Redacted command echoed to the user.
    pub command: String,
    /// Whether the shortcut failed.
    pub is_error: bool,
    /// Redacted, bounded result retained by the live transcript.
    pub result: Option<String>,
}

/// The current state or terminal outcome of a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// Prepared, and waiting for an approval decision.
    WaitingForApproval,
    /// Requested, and still running.
    Running,
    /// Completed successfully.
    Ok,
    /// Completed with an error.
    Failed,
    /// Denied by the approval gate.
    Denied,
    /// The call was still running when its conversation ended, and no
    /// outcome ever arrived.
    ///
    /// A delegated child that is stopped or lost mid-call leaves rows in this
    /// state. Resolving them to `ok` would invent a result; leaving them
    /// `running` would claim work that stopped is still in flight.
    Unreported,
}

impl ToolStatus {
    /// The word rendered beside the tool row. Paired with color, never
    /// replaced by it.
    pub fn label(self) -> &'static str {
        match self {
            Self::WaitingForApproval => "waiting for approval",
            Self::Running => "running",
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Denied => "denied",
            Self::Unreported => "ran",
        }
    }

    /// Recovers the approval outcome from the runtime's canonical error text.
    /// Completion events carry only an error flag, so live enrichment and
    /// history replay must use the same result evidence.
    pub(crate) fn with_result_preview(self, preview: &str) -> Self {
        if self == Self::Failed
            && (preview.starts_with("approval declined:")
                || preview.starts_with("approval denied:"))
        {
            Self::Denied
        } else {
            self
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
        display: Option<Box<ToolCallDisplay>>,
        /// The user-authored shell shortcut, when this call belongs to it.
        user_command: Option<String>,
        /// Process-local identity that survives admission failure and late output.
        shell_echo: Option<u64>,
        /// Value-free fallback for protected arguments.
        protected_summary: String,
        /// The current status.
        status: ToolStatus,
        /// Bounded, credential-redacted result detail supplied by the host.
        /// Folding belongs to the renderer so the detail can be expanded.
        result_preview: Option<String>,
        /// When the tool call started running.
        started_at: Option<Instant>,
        /// Host-confirmed facts appended after the projector's own
        /// qualifiers, kept in a field [`Transcript::set_tool_display`]
        /// never touches.
        ///
        /// A delegation spawn row is projected before the runtime confirms
        /// the child, and the host re-projects `display` from canonical
        /// arguments again when the tool completes. Storing enrichment here
        /// instead of folding it into `display`'s own qualifiers means that
        /// second projection can never silently drop it — see
        /// [`Transcript::enrich_tool_call`].
        enrichment: Vec<String>,
    },
    /// A structured error.
    Error {
        /// The redacted message.
        message: String,
    },
    /// A background notification, or a runtime notice such as a provider
    /// change.
    Notice {
        /// Fixes the source label and transcript persistence.
        kind: NoticeKind,
        /// The notice text.
        text: String,
    },
    /// Read-only host information shown locally and excluded from canonical
    /// provider conversation history.
    Local(LocalResult),
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
                    user_command: u1,
                    shell_echo: h1,
                    status: s1,
                    result_preview: r1,
                    enrichment: e1,
                    ..
                },
                Self::Tool {
                    call_id: c2,
                    name: n2,
                    display: d2,
                    protected_summary: p2,
                    user_command: u2,
                    shell_echo: h2,
                    status: s2,
                    result_preview: r2,
                    enrichment: e2,
                    ..
                },
            ) => {
                c1 == c2
                    && n1 == n2
                    && d1 == d2
                    && p1 == p2
                    && u1 == u2
                    && h1 == h2
                    && s1 == s2
                    && r1 == r2
                    && e1 == e2
            }
            (Self::Error { message: m1 }, Self::Error { message: m2 }) => m1 == m2,
            (Self::Notice { kind: s1, text: t1 }, Self::Notice { kind: s2, text: t2 }) => {
                s1 == s2 && t1 == t2
            }
            (Self::Local(r1), Self::Local(r2)) => r1 == r2,
            _ => false,
        }
    }
}

/// The ordered transcript.
#[derive(Debug, Clone, Default)]
pub struct Transcript {
    blocks: Vec<Block>,
    block_revisions: Vec<u64>,
    next_shell_echo: u64,
    append_revision: u64,
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

    /// Identifies the newest appended block, even after history replacement.
    pub(crate) fn append_revision(&self) -> u64 {
        self.append_revision
    }

    pub(crate) fn block_revision(&self, index: usize) -> u64 {
        self.block_revisions[index]
    }

    // The only mutable borrow of a stored block also invalidates its rows.
    // Unique revisions keep replacement transcripts and diverging clones from
    // reusing rows left at the same position in an App's render cache.
    fn block_mut(&mut self, index: usize) -> &mut Block {
        self.block_revisions[index] = NEXT_BLOCK_REVISION.fetch_add(1, Ordering::Relaxed);
        &mut self.blocks[index]
    }

    fn push_block(&mut self, block: Block) {
        self.append_revision = self.append_revision.wrapping_add(1);
        self.blocks.push(block);
        self.block_revisions.push(0);
        self.block_mut(self.blocks.len() - 1);
    }

    fn tool_index(&self, call_id: &str) -> Option<usize> {
        self.blocks
            .iter()
            .rposition(|block| matches!(block, Block::Tool { call_id: id, .. } if id == call_id))
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
        self.push_block(Block::User { text: text.into() });
    }

    /// Appends a transcript notice, which never merges with adjacent blocks.
    /// Keypress feedback belongs to `App` and cannot enter the transcript.
    pub fn push_notice(&mut self, kind: NoticeKind, text: impl Into<String>) {
        if kind.persistence() == NoticePersistence::Feedback {
            return;
        }
        self.push_block(Block::Notice {
            kind,
            text: text.into(),
        });
    }

    /// Appends an error.
    pub fn push_error(&mut self, message: impl Into<String>) {
        self.close_open();
        self.push_block(Block::Error {
            message: message.into(),
        });
    }

    /// Appends a typed local report, bounding patches and free-text messages.
    pub fn push_local(&mut self, result: LocalResult) {
        // Resume, review, and recovery notices used to append through `push_notice`,
        // which leaves the stream open until its next delta or turn boundary.
        if !matches!(
            &result,
            LocalResult::Agent(report)
                if matches!(
                    report.as_ref(),
                    AgentReport::Resume(
                        AgentResumeReport::RequiresIdle | AgentResumeReport::Started { .. }
                    )
                )
        ) && !matches!(
            &result,
            LocalResult::Review(report)
                if matches!(
                    report.as_ref(),
                    ReviewReport::Empty
                        | ReviewReport::Start(
                            ReviewStartReport::Started { .. } | ReviewStartReport::Queued { .. }
                        )
                )
        ) && !matches!(&result, LocalResult::Recovery(report) if report.is_notice())
        {
            self.close_open();
        }
        let result = match result {
            LocalResult::Diff(report) => LocalResult::Diff(Box::new(bound_diff_report(*report))),
            LocalResult::Shell(mut report) => {
                if let ShellOutput::Output(output) = &mut report.output {
                    *output = bound_local_result(std::mem::take(output));
                }
                LocalResult::Shell(report)
            }
            LocalResult::Message(report) => {
                LocalResult::Message(Box::new(bound_message_report(*report)))
            }
            report => report,
        };
        self.push_block(Block::Local(result));
    }

    /// Appends assistant text, extending the open assistant block if there is
    /// one.
    pub fn push_text_delta(&mut self, delta: &str) {
        if matches!(
            self.blocks.last(),
            Some(Block::Assistant { open: true, .. })
        ) {
            if let Block::Assistant { text, .. } = self.block_mut(self.blocks.len() - 1) {
                text.push_str(delta);
            }
            return;
        }
        self.close_open();
        self.push_block(Block::Assistant {
            text: delta.to_owned(),
            open: true,
        });
    }

    /// Appends reasoning text, extending the open reasoning block if there is
    /// one and its redaction flag matches.
    pub fn push_reasoning_delta(&mut self, delta: &str, delta_redacted: bool) {
        if matches!(self.blocks.last(),
            Some(Block::Reasoning { redacted, open: true, .. }) if *redacted == delta_redacted
        ) {
            if let Block::Reasoning { text, .. } = self.block_mut(self.blocks.len() - 1) {
                text.push_str(delta);
            }
            return;
        }
        self.close_open();
        self.push_block(Block::Reasoning {
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
        let call_id = call_id.into();
        if self
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Tool { call_id: id, .. } if id == &call_id))
        {
            return;
        }
        self.close_open();
        self.push_block(Block::Tool {
            call_id,
            name: name.to_owned(),
            display: arguments
                .and_then(|arguments| project_tool_call_display(name, arguments))
                .map(Box::new),
            protected_summary: summarize_unavailable_arguments(name, argument_keys),
            user_command: None,
            shell_echo: None,
            status: ToolStatus::Running,
            result_preview: None,
            started_at: Some(Instant::now()),
            enrichment: Vec::new(),
        });
    }

    /// Echoes a shortcut until the host supplies its exact runtime identity.
    pub fn push_shell_shortcut(&mut self, command: impl Into<String>) -> u64 {
        let echo = self.next_shell_echo;
        self.next_shell_echo = self.next_shell_echo.wrapping_add(1);
        self.close_open();
        self.push_block(Block::Tool {
            call_id: String::new(),
            name: "shell".to_owned(),
            display: None,
            protected_summary: String::new(),
            user_command: Some(command.into()),
            shell_echo: Some(echo),
            status: ToolStatus::Running,
            result_preview: None,
            started_at: Some(Instant::now()),
            enrichment: Vec::new(),
        });
        echo
    }

    /// The echo just submitted through the composer.
    pub fn latest_shell_echo(&self) -> Option<u64> {
        self.blocks.iter().rev().find_map(|block| match block {
            Block::Tool { shell_echo, .. } => *shell_echo,
            _ => None,
        })
    }

    /// The command displayed by a particular user shell echo.
    pub fn shell_shortcut_command(&self, echo: u64) -> Option<&str> {
        self.blocks.iter().find_map(|block| match block {
            Block::Tool {
                shell_echo: Some(id),
                user_command,
                ..
            } if *id == echo => user_command.as_deref(),
            _ => None,
        })
    }

    /// Joins a specific user echo and runtime call by the host's identity.
    pub fn bind_shell_shortcut(&mut self, echo: u64, call_id: &str) {
        let Some(index) = self.blocks.iter().position(|block| {
            matches!(block,
                Block::Tool { shell_echo: Some(id), .. } if *id == echo
            )
        }) else {
            return;
        };
        if matches!(&self.blocks[index], Block::Tool { call_id: id, .. } if id == call_id) {
            return;
        }
        if let Some(existing) = self.blocks.iter().position(|block| {
            matches!(block,
                Block::Tool { call_id: id, .. } if id == call_id
            )
        }) {
            let mut call = self.blocks.remove(existing);
            self.block_revisions.remove(existing);
            let index = index - usize::from(existing < index);
            if let (
                Block::Tool {
                    user_command: command,
                    ..
                },
                Block::Tool {
                    user_command,
                    shell_echo,
                    ..
                },
            ) = (self.block_mut(index), &mut call)
            {
                *user_command = command.take();
                *shell_echo = Some(echo);
            }
            *self.block_mut(index) = call;
        } else if let Block::Tool { call_id: id, .. } = self.block_mut(index) {
            *id = call_id.to_owned();
        }
    }

    /// Settles the echo even when admission failed before a call existed,
    /// returning the exact bounded result retained by the row.
    pub fn finish_shell_shortcut(
        &mut self,
        echo: u64,
        call_id: Option<&str>,
        is_error: bool,
        output: &str,
    ) -> Option<String> {
        if let Some(call_id) = call_id {
            self.bind_shell_shortcut(echo, call_id);
        }
        let preview = bound_result_preview(if output.trim().is_empty() {
            "No output."
        } else {
            output
        });
        let index = self.blocks.iter().position(
            |block| matches!(block, Block::Tool { shell_echo: Some(id), .. } if *id == echo),
        )?;
        if let Block::Tool {
            status,
            result_preview,
            started_at,
            ..
        } = self.block_mut(index)
        {
            *status = if is_error {
                ToolStatus::Failed.with_result_preview(output)
            } else {
                ToolStatus::Ok
            };
            *result_preview = preview.clone();
            *started_at = None;
            return preview;
        }
        None
    }

    /// Records a tool an installed agent ran inside a harness turn.
    ///
    /// Smith did not dispatch it and did not approve it, but the agent says
    /// what it ran, and a row that withholds that tells the reader less than
    /// the stream already carries. So the call is projected from the agent's
    /// own reported detail through the same reviewed, bounded projector a
    /// built-in call goes through, and the row keeps `agent` as its last
    /// qualifier so the origin stays on screen beside the shape. A tool this
    /// build has no reviewed projection for keeps the value-free row.
    pub fn push_external_tool_call(
        &mut self,
        call_id: impl Into<String>,
        name: &str,
        detail: &Value,
    ) {
        self.close_open();
        let display = project_external_tool_call_display(name, detail);
        self.push_block(Block::Tool {
            call_id: call_id.into(),
            name: name.to_owned(),
            enrichment: match display {
                Some(_) => vec!["agent".to_owned()],
                None => Vec::new(),
            },
            display: display.map(Box::new),
            protected_summary: "run by the agent".to_owned(),
            user_command: None,
            shell_echo: None,
            status: ToolStatus::Running,
            result_preview: None,
            started_at: Some(Instant::now()),
        });
    }

    /// Resolves every still-running call to `status`.
    ///
    /// Called when a conversation reaches a terminal state with rows left
    /// open. A row frozen at `running 4s` would keep claiming work is in
    /// flight for a session that has ended.
    pub fn settle_running_tool_calls(&mut self, status: ToolStatus) {
        for index in (0..self.blocks.len()).rev() {
            if matches!(
                &self.blocks[index],
                Block::Tool {
                    status: ToolStatus::Running | ToolStatus::WaitingForApproval,
                    ..
                }
            ) && let Block::Tool { status: slot, .. } = self.block_mut(index)
            {
                *slot = status;
            }
        }
    }

    /// Drops the oldest blocks until at most `max` remain.
    ///
    /// The root transcript is the session and is never trimmed. A child's is
    /// a bounded tail the client keeps in memory on the child's behalf, so it
    /// has a ceiling.
    pub fn retain_newest(&mut self, max: usize) {
        if self.blocks.len() > max {
            let removed = self.blocks.len() - max;
            self.blocks.drain(..removed);
            self.block_revisions.drain(..removed);
        }
    }

    /// Adds a reviewed local display projection to an existing live call.
    ///
    /// This is deliberately separate from [`SmithEventKind`](smith_runtime::client::SmithEventKind)
    /// folding so protected event and journal payloads do not need to carry
    /// argument values. The host calls this again when a tool completes, to
    /// re-project from canonical arguments — that second call replaces
    /// `display` wholesale, which is exactly why enrichment lives in its own
    /// `enrichment` field this method never touches; see
    /// [`Self::enrich_tool_call`].
    pub fn set_tool_display(&mut self, call_id: &str, display: ToolCallDisplay) {
        if let Some(index) = self.tool_index(call_id)
            && let Block::Tool { display: slot, .. } = self.block_mut(index)
        {
            *slot = Some(Box::new(display));
        }
    }

    /// Appends host-confirmed facts to an existing call's row, independent
    /// of its reviewed `display` projection.
    ///
    /// A delegation spawn row is projected from the call's own arguments
    /// before the runtime confirms the child, so the projector cannot yet
    /// know the child's id, its resolved workspace posture, or its turn
    /// ceiling. Once the runtime reports those facts, the caller correlates
    /// them back to this row by call id and enriches it here rather than
    /// rendering a second row for the same spawn — and because this is a
    /// field of its own, a later [`Self::set_tool_display`] re-projection at
    /// tool completion cannot silently drop it.
    ///
    /// Qualifiers are normalized and bounded the same way a projector's own
    /// qualifiers are, by borrowing the row's current display to do it: this
    /// enrichment is host/event-sourced, not from a reviewed schema, so a
    /// caller cannot smuggle unbounded text or line, terminal, and bidi
    /// control characters onto the transcript through it either. Does
    /// nothing for an unknown call id or a row with no display yet, since
    /// there is nothing safe to bound enrichment against.
    pub fn enrich_tool_call(
        &mut self,
        call_id: &str,
        qualifiers: impl IntoIterator<Item = String>,
    ) {
        if let Some(index) = self.tool_index(call_id)
            && let Block::Tool {
                display,
                enrichment,
                ..
            } = self.block_mut(index)
        {
            let Some(current) = display.as_deref().cloned() else {
                return;
            };
            let before = current.qualifiers().len();
            let bounded = current.with_qualifiers(qualifiers);
            enrichment.extend(bounded.qualifiers()[before..].iter().cloned());
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
        if let Some(index) = self.tool_index(call_id)
            && let Block::Tool {
                result_preview: slot,
                status,
                ..
            } = self.block_mut(index)
        {
            *status = status.with_result_preview(&preview);
            *slot = Some(preview);
        }
    }

    /// Updates a tool call's status, returning whether its row exists.
    /// Unknown ids never fabricate a block for a call the transcript never saw.
    pub fn complete_tool_call(&mut self, call_id: &str, status: ToolStatus) -> bool {
        if let Some(index) = self.tool_index(call_id)
            && let Block::Tool {
                status: slot,
                started_at,
                result_preview,
                ..
            } = self.block_mut(index)
        {
            if status == ToolStatus::Running && *slot == ToolStatus::WaitingForApproval {
                *started_at = Some(Instant::now());
            } else if status == ToolStatus::WaitingForApproval {
                *started_at = None;
            }
            // A canonical completion reports a denied call as an error.
            // Do not erase the approval decision before enrichment arrives.
            if !(*slot == ToolStatus::Denied && status == ToolStatus::Failed) {
                *slot = status.with_result_preview(result_preview.as_deref().unwrap_or(""));
            }
            return true;
        }
        false
    }

    /// The current presentation status for one stable call identity.
    pub(crate) fn tool_status(&self, call_id: &str) -> Option<ToolStatus> {
        self.blocks.iter().rev().find_map(|block| match block {
            Block::Tool {
                call_id: id,
                status,
                ..
            } if id == call_id => Some(*status),
            _ => None,
        })
    }

    /// Marks the most recent still-running call of `name` finished.
    ///
    /// A name-only fallback for callers without a stable call identity.
    /// Prepared approvals use [`Self::complete_tool_call`] with their call id.
    pub fn complete_tool_call_by_name(&mut self, name: &str, status: ToolStatus) {
        if let Some(index) = self.blocks.iter().rposition(|block| {
            matches!(block, Block::Tool { name: candidate, status, .. }
                if candidate == name
                    && matches!(status, ToolStatus::Running | ToolStatus::WaitingForApproval))
        }) && let Block::Tool { status: slot, .. } = self.block_mut(index)
        {
            *slot = status;
        }
    }

    /// Closes any block still receiving deltas. Called at turn boundaries so a
    /// later delta starts a new block instead of extending a finished reply.
    pub fn close_open(&mut self) {
        for index in (0..self.blocks.len()).rev() {
            if matches!(
                &self.blocks[index],
                Block::Assistant { open: true, .. } | Block::Reasoning { open: true, .. }
            ) && let Block::Assistant { open, .. } | Block::Reasoning { open, .. } =
                self.block_mut(index)
            {
                *open = false;
            }
        }
    }
}

/// A single-line, control-free tool name safe to interpolate into a
/// fallback row.
///
/// `pub(crate)` so the delegated-work panel names a child's current tool the
/// same way the transcript's own unknown-tool fallback does; see
/// `App::apply_child`.
pub(crate) fn safe_tool_name(name: &str) -> String {
    let name = name
        .chars()
        .take(64)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if name.is_empty() {
        "tool".to_owned()
    } else {
        name
    }
}

/// A stable, value-free fallback when no reviewed projection is available.
///
/// `pub(crate)` so the delegated-work panel can show the identical honest
/// label — never raw argument values — when a child's tool call has no
/// reviewed projection either; see `App::apply_child`.
pub(crate) fn summarize_unavailable_arguments(_name: &str, _argument_keys: &[String]) -> String {
    "arguments hidden".to_owned()
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

mod history;
mod limits;

use limits::{bound_diff_report, bound_local_result, bound_message_report, bound_result_preview};

#[cfg(test)]
mod detail_tests;
#[cfg(test)]
mod tests;
