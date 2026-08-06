//! Core application state: the reducer over runtime events and key presses.
//!
//! [`App`] is deliberately free of I/O. It folds [`EventEnvelope`]s and
//! [`KeyEvent`]s into state and returns [`Action`]s for the host loop to
//! perform. Everything the screen shows is derivable from this struct, which is
//! what makes the renderer testable against a fake terminal and the key map
//! testable with no terminal at all.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::content::{ContentPart, UserInput};
use agent_runtime_core::event::{EventEnvelope, PlanItemProjection, PlanSensitivity};
use agent_runtime_core::ids::{AttemptId, RequestId, TurnId};
use agent_runtime_core::steer::SteerReceipt;
use smith_host::approval::ApprovalPrompt;
use smith_host::rotation::RotationPrompt;
use smith_tools::ToolCallDisplay;

use crate::commands::CommandAction;
use crate::composer::Composer;
use crate::diff::EditReview;
use crate::picker::{ResourceEntry, ResourcePicker};
use crate::questionnaire::{QuestionnaireResolution, QuestionnaireState};
use crate::selection::Selection;
use crate::status::{Activity, Status, render_elapsed};
use crate::transcript::{ToolStatus, Transcript};

/// How long a second `Ctrl+C` still counts as the exit press.
pub(super) const FORCE_QUIT_WINDOW: Duration = Duration::from_secs(1);

/// Pastes with at least this many lines collapse to a placeholder chunk.
pub(super) const PASTE_CHUNK_MIN_LINES: usize = 3;
/// Single-line pastes longer than this also collapse to a chunk.
pub(super) const PASTE_CHUNK_MIN_CHARS: usize = 1_000;
/// Bounded process-local paste storage; the oldest chunk is dropped first.
pub(super) const MAX_PASTED_CHUNKS: usize = 50;

/// Transcript lines one wheel notch scrolls.
pub(super) const MOUSE_SCROLL_LINES: u16 = 3;

/// One large paste stored aside so the composer stays editable.
///
/// The composer holds only the placeholder text — `[Pasted text #2 +8 lines]`
/// — which the user crosses or deletes as one logical unit. The stored content
/// re-enters provider input and the committed transcript only at submit time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PastedChunk {
    pub(super) placeholder: String,
    pub(super) content: String,
}

/// Bounded clipboard-image storage; the oldest attachment is dropped first.
pub(super) const MAX_IMAGE_ATTACHMENTS: usize = 16;

/// One clipboard image stored aside behind an `[Image #N W×H]` placeholder.
///
/// The same contract as [`PastedChunk`]: the composer holds only the
/// placeholder, and the encoded image joins the outgoing turn as an image
/// content part when its placeholder is still present at submit time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ImageAttachment {
    pub(super) placeholder: String,
    pub(super) data_uri: String,
}

/// One validated ordinary composer submission before file materialization.
///
/// This process-local value owns everything that may otherwise disappear when
/// the composer clears. It performs no I/O: canonical file identities are
/// resolved by the host only when the submission is actually dispatched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSubmission {
    pub(super) display_text: String,
    pub(super) committed_text: String,
    pub(super) expanded_text: String,
    pub(super) files: Vec<String>,
    pub(super) images: Vec<ImageAttachment>,
    pub(super) pastes: Vec<PastedChunk>,
}

impl PreparedSubmission {
    /// Exact compact text shown in the composer and queue preview.
    pub fn display_text(&self) -> &str {
        &self.display_text
    }

    /// User text shown once the runtime commits this input.
    pub fn committed_text(&self) -> &str {
        &self.committed_text
    }

    /// Canonical workspace-relative files to read at dispatch time.
    pub fn files(&self) -> &[String] {
        &self.files
    }

    /// Model input that does not require file materialization.
    pub fn input_without_files(&self) -> UserInput {
        let mut parts = vec![ContentPart::text(self.expanded_text.clone())];
        parts.extend(self.images.iter().map(|attachment| ContentPart::Image {
            url: attachment.data_uri.clone(),
            detail: None,
        }));
        UserInput { parts }
    }

    pub(super) fn merge_fifo(entries: impl IntoIterator<Item = Self>) -> Option<Self> {
        let mut entries = entries.into_iter();
        let mut merged = entries.next()?;
        for entry in entries {
            merged.display_text.push_str("\n\n");
            merged.display_text.push_str(&entry.display_text);
            merged.committed_text.push_str("\n\n");
            merged.committed_text.push_str(&entry.committed_text);
            merged.expanded_text.push_str("\n\n");
            merged.expanded_text.push_str(&entry.expanded_text);
            merged.files.extend(entry.files);
            merged.images.extend(entry.images);
            merged.pastes.extend(entry.pastes);
        }
        Some(merged)
    }
}

/// Why the host is dispatching one prepared ordinary submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmissionTarget {
    /// Start one whole user turn now.
    WholeTurn,
    /// Target the tracked serving turn.
    Steer {
        /// Expected serving identity, if its start event has already arrived.
        expected_turn: Option<TurnId>,
    },
}

/// Bounded process-local preview exposed to the renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingInputPreview {
    /// Human-readable category label.
    pub label: &'static str,
    /// Exact compact draft texts, oldest first.
    pub entries: Vec<String>,
    /// Entries hidden by the per-section preview bound.
    pub overflow: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingSteer {
    pub(super) receipt: SteerReceipt,
    pub(super) submission: PreparedSubmission,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RejectedFollowup {
    pub(super) turn: Option<TurnId>,
    pub(super) interrupt_eligible: bool,
    pub(super) submission: PreparedSubmission,
}

/// Process-local user input not yet represented by canonical history.
#[derive(Debug, Default)]
pub(super) struct PendingInputState {
    pub(super) accepted_steers: VecDeque<PendingSteer>,
    pub(super) rejected_followups: VecDeque<RejectedFollowup>,
    pub(super) queued_turns: VecDeque<PreparedSubmission>,
    pub(super) ready_submission: Option<PreparedSubmission>,
    pub(super) interrupt_for_steer: bool,
}

pub(super) const MAX_EXPLICIT_QUEUED_TURNS: usize = 16;
pub(super) const MAX_REJECTED_FOLLOWUPS: usize = 16;
pub(crate) const MAX_PENDING_PREVIEW_ENTRIES: usize = 3;

/// Collapses a paste onto one line for single-line query surfaces.
pub(super) fn flatten_paste(text: &str) -> String {
    text.split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Resource-ID namespace for transition-release root-mode adapters.
pub const LEGACY_AGENT_PROFILE_PREFIX: &str = "legacy-agent:";

/// Something the host loop must do on the app's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Dispatch one already-prepared ordinary input with the stated intent.
    Submit {
        /// Exact process-local submission material.
        submission: PreparedSubmission,
        /// Whole-turn or active-turn intent.
        target: SubmissionTarget,
    },
    /// Execute one explicit local shell shortcut without provider spend.
    RunShell {
        /// Command after the leading `!` marker.
        command: String,
    },
    /// Cancel the running turn.
    Interrupt,
    /// Manually background the running foreground shell call (`Ctrl+B`).
    ///
    /// Distinct from `Interrupt`: the owned process keeps running and the
    /// pending call resolves with the output captured so far instead of
    /// being killed.
    BackgroundShell,
    /// Leave the application.
    Quit,
    /// Rebuild or replace the hosted session at a safe turn boundary.
    Reconfigure(PaletteCommand),
    /// Execute a local product command without sending composer text to the
    /// provider.
    Command(CommandAction),
    /// Apply the already-previewed last-turn undo.
    ApplyUndo,
    /// Record that the already-previewed undo was explicitly cancelled.
    CancelUndo,
    /// Apply the already-previewed newest exact redo candidate.
    ApplyRedo,
    /// Record that the already-previewed redo was explicitly cancelled.
    CancelRedo,
    /// Apply an exact file/hunk revert after stale-preview validation.
    ApplyRevert {
        /// File or `file#hunk` scope.
        scope: String,
        /// Preview fingerprint.
        fingerprint: String,
    },
    /// Record that the already-previewed selective revert was cancelled.
    CancelRevert {
        /// File or `file#hunk` scope.
        scope: String,
        /// Preview fingerprint.
        fingerprint: String,
    },
    /// Start the already-confirmed provider-backed read-only review.
    StartReview {
        /// Review scope.
        scope: String,
    },
    /// Start one confirmed, child-enabled read-only agent profile.
    StartAgent {
        /// Registered child-enabled profile.
        preset: String,
        /// Bounded task supplied after the reference.
        task: String,
    },
    /// Start a new turn on one existing idle child session.
    FollowUpAgent {
        /// Stable existing child identity.
        child_id: String,
        /// Bounded new task.
        task: String,
    },
    /// Continue one interrupted child's exact protected checkpoint.
    ResumeAgent {
        /// Stable existing child identity.
        child_id: String,
    },
}

/// A command selected from the TUI command palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteCommand {
    /// Create a fresh session with the current selection.
    NewSession,
    /// Resume an existing session identity.
    Resume(String),
    /// Select a configured profile and clear narrower provider/model flags.
    Profile(String),
    /// Select a provider/model pair atomically.
    Model {
        /// Serving provider.
        provider: String,
        /// Provider model ID.
        model: String,
    },
    /// Open the reviewed connection ceremony for a provider or backend.
    Connect(String),
    /// Remove one provider or backend authentication source.
    Disconnect(String),
    /// Select a deprecated legacy root mode at a safe session boundary.
    Agent(String),
    /// Select an explicit thinking state; `None` restores provider behavior.
    Think(Option<bool>),
    /// Select an advertised effort; `None` restores provider behavior.
    Effort(Option<String>),
    /// Switch the active provider credential to a pool position.
    Account(usize),
}

/// Bounded local resources available to runtime pickers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeResources {
    /// Provider-qualified models.
    pub models: Vec<ResourceEntry>,
    /// Configured providers.
    pub providers: Vec<ResourceEntry>,
    /// Providers available to connect.
    pub connections: Vec<ResourceEntry>,
    /// Currently configured providers.
    pub disconnections: Vec<ResourceEntry>,
    /// Configured profiles.
    pub profiles: Vec<ResourceEntry>,
    /// Project-scoped saved sessions.
    pub sessions: Vec<ResourceEntry>,
    /// Bounded canonical workspace-file index.
    pub files: Vec<ResourceEntry>,
    /// Child-enabled read-only agent profiles.
    pub child_agents: Vec<ResourceEntry>,
    /// Main-enabled agent profiles in configured cycle order.
    pub main_profiles: Vec<ResourceEntry>,
    /// Bounded thinking-state choices for the active binding.
    pub thinking: Vec<ResourceEntry>,
    /// Bounded effort choices for the active binding.
    pub efforts: Vec<ResourceEntry>,
    /// Declared credential-pool members for the active provider, with their
    /// server-reported usage and cooldown state. Empty when the provider
    /// declares a single credential, which is not a pool.
    pub accounts: Vec<ResourceEntry>,
    /// Active session ID.
    pub current_session: Option<String>,
}

/// Which typed selection one resource picker applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceTarget {
    /// Provider-qualified model pair.
    Model,
    /// Provider, followed by its one model or a filtered model picker.
    Provider,
    /// Provider connection ceremony.
    Connect,
    /// Connected provider to disconnect.
    Disconnect,
    /// Coherent configured profile.
    Profile,
    /// Project session.
    Resume,
    /// Thinking state for subsequent turns.
    Think,
    /// Advertised effort for subsequent turns.
    Effort,
    /// Insert one typed file or child-agent reference into the composer.
    Reference,
    /// Credential-pool member serving subsequent attempts.
    Account,
}

/// A temporary interactive surface. At most one exists at a time.
///
/// Consequential variants draw over the transcript; completion and resource
/// selection reserve a compact pane above the composer.
#[derive(Debug)]
pub enum Overlay {
    /// A tool is waiting for approval.
    Approval {
        /// What the runtime is asking to run, and the channel to answer on.
        prompt: Box<ApprovalPrompt>,
        /// The reviewable diff, when the request is an `edit` whose arguments
        /// parse. `None` sends the modal back to rendering raw arguments.
        review: Option<EditReview>,
    },
    /// An authority-free runtime interaction is waiting for an answer.
    Questionnaire {
        /// Pure staged-answer and keyboard state.
        state: QuestionnaireState,
    },
    /// Select a session or immutable runtime configuration.
    Palette {
        /// Selected filtered result.
        selected: usize,
        /// A parse error kept inside the completion pane.
        error: Option<String>,
        /// Draft restored when `Ctrl+P` discovery is dismissed.
        restore_on_escape: Option<String>,
    },
    /// Search locally available runtime/session resources in a compact pane.
    ResourcePicker {
        /// Pure shared picker state.
        picker: ResourcePicker,
        /// Typed application behavior.
        target: ResourceTarget,
        /// Composer draft restored on cancellation.
        restore_on_escape: String,
    },
    /// Search bounded process-local composer history in a compact pane.
    HistorySearch {
        /// Composer draft restored when search is cancelled.
        original: String,
        /// Case-insensitive substring query.
        query: String,
        /// Stable history index of the selected match.
        selected: Option<usize>,
        /// Exact selected history entry, ready to restore into the composer.
        matched: Option<String>,
    },
    /// Exact reverse patch awaiting a no-default confirmation.
    UndoConfirm {
        /// Bounded reverse patch.
        content: String,
    },
    /// Exact forward patch awaiting a no-default confirmation.
    RedoConfirm {
        /// Bounded forward patch.
        content: String,
    },
    /// Selective revert awaiting a no-default confirmation.
    RevertConfirm {
        /// File or `file#hunk` scope.
        scope: String,
        /// Stale-preview fingerprint.
        fingerprint: String,
        /// Exact reverse patch.
        content: String,
    },
    /// Provider-backed read-only review awaiting explicit confirmation.
    ReviewConfirm {
        /// Review scope.
        scope: String,
        /// Spend and scope explanation.
        content: String,
    },
    /// Explicit child invocation awaiting provider-spend confirmation.
    AgentConfirm {
        /// Registered preset identity.
        preset: String,
        /// Exact bounded task.
        task: String,
        /// Inherited model, limits, and posture summary.
        content: String,
    },
    /// Existing-child follow-up awaiting provider-spend confirmation.
    AgentFollowUpConfirm {
        /// Stable child identity.
        child_id: String,
        /// Exact bounded follow-up task.
        task: String,
        /// Continuity, scope, and spend summary.
        content: String,
    },
    /// Exact interrupted-checkpoint continuation awaiting confirmation.
    AgentResumeConfirm {
        /// Stable child identity.
        child_id: String,
        /// Recovery and spend summary.
        content: String,
    },
    /// A spent account is offering to move to another pool member.
    ///
    /// Boxed like the approval prompt, and for the same reason: dropping it
    /// without answering must decline rather than switch, so the channel is
    /// owned by the overlay.
    RotationConfirm {
        /// The offer, and the channel to answer it on.
        prompt: Box<RotationPrompt>,
        /// The rendered body, including the prompt-cache cost.
        content: String,
    },
    /// Exit was requested while work is live.
    ExitConfirm {
        /// An approval hidden by the confirmation and restored if the user
        /// cancels exit.
        approval: Option<(Box<ApprovalPrompt>, Option<EditReview>)>,
        /// A questionnaire hidden by the confirmation and restored if the
        /// user cancels exit.
        questionnaire: Option<QuestionnaireState>,
    },
}

/// A runtime-originated prompt waiting behind the visible overlay.
#[derive(Debug)]
pub(super) enum PendingPrompt {
    Approval(Box<ApprovalPrompt>, Option<EditReview>),
    Questionnaire(QuestionnaireState),
}

/// The latest user-visible state of one child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSummary {
    /// Current lifecycle label.
    pub state: String,
    /// Latest bounded result or detail.
    pub detail: Option<String>,
}

impl ChildSummary {
    /// Whether this child's lifecycle label describes in-flight work.
    ///
    /// Both the panel's row order and the inspector's keyboard order read
    /// this, so a live child can never sort one way and select another.
    pub fn is_live(&self) -> bool {
        matches!(
            self.state.as_str(),
            "running" | "working" | "resuming" | "needs input"
        )
    }
}

/// Most retained log lines per child.
///
/// The child's own journal keeps the whole record; this is the bounded tail
/// the inspector can show without growing without limit in a long session.
pub(super) const MAX_CHILD_LOG_LINES: usize = 200;

/// Wall-clock projection for one delegated child's agents-panel row.
///
/// Kept beside — not inside — [`ChildSummary`]: summaries are compared
/// between live application and journal replay, and a live `Instant` can
/// never replay equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ChildClock {
    /// When the live child last started running.
    started_at: Instant,
    /// Elapsed frozen at the moment the child settled.
    settled: Option<Duration>,
}

impl ChildClock {
    /// A clock started now.
    pub(super) fn started() -> Self {
        Self {
            started_at: Instant::now(),
            settled: None,
        }
    }

    /// The running or frozen elapsed time.
    pub(super) fn elapsed(&self) -> Duration {
        self.settled.unwrap_or_else(|| self.started_at.elapsed())
    }

    /// Whether the clock is still ticking.
    pub(super) fn is_live(&self) -> bool {
        self.settled.is_none()
    }

    /// Freezes the clock at the moment its child settled.
    pub(super) fn settle(&mut self) {
        if self.settled.is_none() {
            self.settled = Some(self.started_at.elapsed());
        }
    }

    /// Restarts a settled clock for a follow-up or resume turn.
    pub(super) fn resume(&mut self) {
        if self.settled.is_some() {
            *self = Self::started();
        }
    }
}

/// One running background shell task, as the host last polled it from
/// `BackgroundTaskRegistry::running_tasks`.
///
/// The TUI never reaches the registry itself — see `DESIGN.md`'s host/TUI
/// split — so this is the whole fact base it has about background tasks.
/// Pushed wholesale on every poll: a task absent from the latest push is not
/// specially removed here, it is simply no longer named, because the
/// registry itself stops returning a terminal task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningTaskSummary {
    /// Stable session-scoped task identity, e.g. `task:3`.
    pub task_id: String,
    /// Bounded, single-line command hint safe to display.
    pub command_hint: String,
}

/// Latest durable todo-plan projection.
///
/// Smith deliberately treats bounded plan text as public working state: it is
/// rendered in the anchored todo pane and may be reconstructed from the
/// redacted journal. A sensitive runtime projection retains counts only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanSummary {
    /// Monotonic plan revision.
    pub revision: u64,
    /// Whether bounded item text may be displayed.
    pub sensitivity: PlanSensitivity,
    /// Aggregate counts keyed by the stable runtime status spelling.
    pub counts: BTreeMap<String, u32>,
    /// Public bounded items, absent for a sensitive plan.
    pub items: Option<Vec<PlanItemProjection>>,
}

/// Replaceable, replay-derived evidence for the current turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct WorkSummary {
    pub(super) tools: BTreeMap<String, (String, ToolStatus, Option<Instant>)>,
}

/// One live-stream sequence gap awaiting journal replay.
///
/// The envelope that revealed the gap is parked here un-applied. Applying it
/// ahead of the missing range would fold control events out of order — a
/// skipped `TurnCompleted` could strand queued input behind a turn the UI
/// still believes is running. The host replays `first_missing..=last_missing`
/// from the canonical journal, then applies `deferred`.
#[derive(Debug, Clone)]
pub struct StreamGap {
    /// First missing sequence number.
    pub first_missing: u64,
    /// Last missing sequence number.
    pub last_missing: u64,
    /// The out-of-order envelope, to apply after the replayed range.
    pub deferred: EventEnvelope,
}

/// Which stage of one provider round-trip is live right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderPhase {
    /// The request is dispatched; no output has arrived yet.
    Sending,
    /// Reasoning deltas are arriving.
    Thinking,
    /// Visible answer text is arriving.
    Responding,
}

/// One provider attempt's speculative presentation identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct AttemptOutputKey {
    pub(super) request: RequestId,
    pub(super) attempt: AttemptId,
}

impl AttemptOutputKey {
    pub(super) fn new(request: &RequestId, attempt: &AttemptId) -> Self {
        Self {
            request: request.clone(),
            attempt: attempt.clone(),
        }
    }
}

/// A delta retained outside the canonical transcript until an explicit commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SpeculativeChunk {
    Text(String),
    Reasoning { text: String, redacted: bool },
}

/// Buffered output for one in-flight provider attempt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SpeculativeAttempt {
    pub(super) chunks: Vec<SpeculativeChunk>,
    pub(super) visible_text: String,
}

impl SpeculativeAttempt {
    pub(super) fn push_text(&mut self, text: &str) {
        self.visible_text.push_str(text);
        if let Some(SpeculativeChunk::Text(previous)) = self.chunks.last_mut() {
            previous.push_str(text);
        } else {
            self.chunks.push(SpeculativeChunk::Text(text.to_owned()));
        }
    }

    pub(super) fn push_reasoning(&mut self, text: &str, redacted: bool) {
        if let Some(SpeculativeChunk::Reasoning {
            text: previous,
            redacted: previous_redacted,
        }) = self.chunks.last_mut()
            && *previous_redacted == redacted
        {
            previous.push_str(text);
        } else {
            self.chunks.push(SpeculativeChunk::Reasoning {
                text: text.to_owned(),
                redacted,
            });
        }
    }
}

/// The whole client's state.
#[derive(Debug)]
pub struct App {
    /// The transcript.
    pub transcript: Transcript,
    /// Header status.
    pub status: Status,
    /// The input buffer.
    pub composer: Composer,
    /// The current overlay, if any.
    pub overlay: Option<Overlay>,
    /// Runtime prompts waiting behind the one visible overlay, in exact
    /// cross-type arrival order.
    pub(super) pending_prompts: VecDeque<PendingPrompt>,
    /// Questionnaire outcomes the host adapter has not consumed yet.
    pub(super) questionnaire_resolutions: VecDeque<(String, QuestionnaireResolution)>,
    /// Latest child states, keyed by stable child id.
    pub children: BTreeMap<String, ChildSummary>,
    /// Live panel clocks for children, keyed by stable child id.
    pub(super) child_clocks: BTreeMap<String, ChildClock>,
    /// Bounded per-child progress log, keyed by stable child id.
    ///
    /// Child progress no longer narrates itself into the root transcript, so
    /// this is where "what has that agent been doing" lives. Plain text, never
    /// timestamps: summaries are compared between live application and journal
    /// replay, and a wall-clock reading can never replay equal.
    pub(super) child_logs: BTreeMap<String, Vec<String>>,
    /// Temporary child inspector selection; the root composer keeps focus.
    pub inspected_child: Option<String>,
    /// The host's latest coordinator card for [`Self::inspected_child`].
    ///
    /// The client has no delegation access of its own, so authoritative
    /// session, turn, token, and workspace figures arrive from the host on the
    /// same poll-on-redraw cadence as background tasks. Absent until that poll
    /// answers, which is honest: the client never invents child accounting.
    pub(super) inspected_detail: Option<String>,
    /// Running background shell tasks, as of the host's latest registry poll.
    pub running_tasks: Vec<RunningTaskSummary>,
    /// When each running task was first seen, keyed by task id.
    ///
    /// The registry reports no start timestamp, so the panel clock counts
    /// from first sight. Polls are frequent enough that the difference is
    /// display noise, and a wrong-but-ticking clock is never shown for a
    /// task the registry no longer returns.
    pub(super) task_clocks: BTreeMap<String, Instant>,
    /// Latest durable todo plan, projected in the anchored composer pane.
    pub plan: Option<PlanSummary>,
    /// Bounded live tool detail available only through `/details`.
    pub(super) work: Option<WorkSummary>,
    /// Whether bounded live tool details are expanded.
    pub work_details: bool,
    /// Bounded local choices supplied by the host.
    pub resources: RuntimeResources,
    /// Whether the transcript follows new output.
    pub following: bool,
    /// Lines scrolled up from the bottom when not following.
    pub scroll_back: u16,
    /// Most lines the current transcript viewport can scroll.
    pub(super) scroll_limit: u16,
    /// The live pointer selection, in rendered-cell coordinates.
    ///
    /// Smith owns selection because enabling wheel reporting takes the
    /// terminal's own away; see [`crate::selection`].
    pub selection: Option<Selection>,
    /// The animation tick.
    pub tick: u64,
    /// Set once the host loop should exit.
    pub should_quit: bool,
    /// Large pastes stored aside behind composer placeholders, oldest first.
    pub(super) pasted_chunks: Vec<PastedChunk>,
    /// Monotonic number for `[Pasted text #N …]` labels.
    pub(super) paste_counter: usize,
    /// Clipboard images stored aside behind composer placeholders.
    pub(super) image_attachments: Vec<ImageAttachment>,
    /// Monotonic number for `[Image #N …]` labels.
    pub(super) image_counter: usize,
    /// Ephemeral "Worked for …" summary of the newest completed turn.
    ///
    /// Deliberately not a transcript block: one row per historical turn is
    /// noise in the UI, while the journal keeps the full per-turn record.
    pub turn_summary: Option<String>,
    pub(super) turn_started_at: Option<Instant>,
    pub(super) turn_started_timestamp: Option<Timestamp>,
    pub(super) last_ctrl_c: Option<Instant>,
    pub(super) last_event_seq: Option<u64>,
    /// A live-stream sequence gap parked for host-driven journal replay.
    pub(super) stream_gap: Option<StreamGap>,
    /// The live provider round-trip stage and when it started.
    pub(super) provider_phase: Option<(ProviderPhase, Instant)>,
    pub(super) speculative_attempts: BTreeMap<AttemptOutputKey, SpeculativeAttempt>,
    pub(super) speculative_order: Vec<AttemptOutputKey>,
    pub(super) finalized_attempts: BTreeSet<AttemptOutputKey>,
    /// Serving turn identity from typed runtime envelopes.
    pub(super) active_turn: Option<TurnId>,
    /// Process-local, not-yet-canonical user input.
    pub(super) pending_input: PendingInputState,
}

impl App {
    /// A fresh client for `model` rooted at `project`.
    pub fn new(model: impl Into<String>, project: impl Into<String>) -> Self {
        Self {
            transcript: Transcript::new(),
            status: Status::new(model, project),
            composer: Composer::new(),
            overlay: None,
            pending_prompts: VecDeque::new(),
            questionnaire_resolutions: VecDeque::new(),
            children: BTreeMap::new(),
            child_clocks: BTreeMap::new(),
            child_logs: BTreeMap::new(),
            inspected_child: None,
            inspected_detail: None,
            running_tasks: Vec::new(),
            task_clocks: BTreeMap::new(),
            plan: None,
            work: None,
            work_details: false,
            resources: RuntimeResources::default(),
            following: true,
            scroll_back: 0,
            scroll_limit: 0,
            selection: None,
            tick: 0,
            should_quit: false,
            pasted_chunks: Vec::new(),
            paste_counter: 0,
            image_attachments: Vec::new(),
            image_counter: 0,
            turn_summary: None,
            turn_started_at: None,
            turn_started_timestamp: None,
            last_ctrl_c: None,
            last_event_seq: None,
            stream_gap: None,
            provider_phase: None,
            speculative_attempts: BTreeMap::new(),
            speculative_order: Vec::new(),
            finalized_attempts: BTreeSet::new(),
            active_turn: None,
            pending_input: PendingInputState::default(),
        }
    }

    /// Replaces the local, credential-free picker inventory.
    pub fn set_resources(&mut self, resources: RuntimeResources) {
        self.resources = resources;
    }

    /// Seeds one already-persisted child before live event subscription.
    ///
    /// Recovery events may be journaled before a terminal client attaches;
    /// this owner-supplied projection keeps inspection and `@child-id`
    /// continuation available without replaying or parsing journal prose.
    pub fn restore_child(
        &mut self,
        child_id: impl Into<String>,
        state: impl Into<String>,
        detail: Option<String>,
    ) {
        self.children.insert(
            child_id.into(),
            ChildSummary {
                state: state.into(),
                detail,
            },
        );
    }

    /// Replaces the running-background-task listing with the host's latest
    /// registry poll.
    ///
    /// A wholesale replace rather than an incremental diff: the registry
    /// already filters to non-terminal tasks, so a task's disappearance from
    /// `tasks` is itself the terminal signal — no separate removal path is
    /// needed.
    pub fn set_running_tasks(&mut self, tasks: Vec<RunningTaskSummary>) {
        // First sight starts a task's panel clock; disappearance ends it.
        // A re-used id restarting at zero is honest: the registry only
        // returns live tasks, so reappearance means a new run.
        self.task_clocks = tasks
            .iter()
            .map(|task| {
                let started = self
                    .task_clocks
                    .get(&task.task_id)
                    .copied()
                    .unwrap_or_else(Instant::now);
                (task.task_id.clone(), started)
            })
            .collect();
        self.running_tasks = tasks;
    }

    /// Elapsed time since one running background task was first seen.
    pub fn task_elapsed(&self, task_id: &str) -> Option<Duration> {
        self.task_clocks
            .get(task_id)
            .map(|started| started.elapsed())
    }

    /// Compact footer segment naming running background tasks, or `None`
    /// when none are running.
    ///
    /// Bounded to a few identities plus a remainder count so a chatty session
    /// running many tasks cannot grow the identity footer without limit.
    pub fn render_running_tasks_footer(&self) -> Option<String> {
        if self.running_tasks.is_empty() {
            return None;
        }
        const SHOWN: usize = 3;
        let mut label = self
            .running_tasks
            .iter()
            .take(SHOWN)
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let remaining = self.running_tasks.len().saturating_sub(SHOWN);
        if remaining > 0 {
            label.push_str(&format!(", +{remaining} more"));
        }
        Some(format!("bg {label}"))
    }

    /// Whether a turn is in flight.
    pub fn is_busy(&self) -> bool {
        matches!(
            self.status.activity,
            Activity::Working | Activity::Interrupting
        )
    }

    /// Whether anything would be lost by quitting now.
    pub fn has_live_work(&self) -> bool {
        self.is_busy()
            || self.has_pending_input()
            || !self.pending_prompts.is_empty()
            || !self.running_tasks.is_empty()
            || matches!(
                self.overlay,
                Some(Overlay::Approval { .. })
                    | Some(Overlay::Questionnaire { .. })
                    | Some(Overlay::ExitConfirm {
                        approval: Some(_),
                        ..
                    })
                    | Some(Overlay::ExitConfirm {
                        questionnaire: Some(_),
                        ..
                    })
            )
    }

    /// Number of approval prompts still awaiting one decision each.
    pub fn pending_approval_count(&self) -> usize {
        let visible = usize::from(matches!(
            self.overlay,
            Some(Overlay::Approval { .. })
                | Some(Overlay::ExitConfirm {
                    approval: Some(_),
                    ..
                })
        ));
        visible.saturating_add(
            self.pending_prompts
                .iter()
                .filter(|prompt| matches!(prompt, PendingPrompt::Approval(..)))
                .count(),
        )
    }

    /// Number of questionnaire requests awaiting one terminal answer each.
    pub fn pending_questionnaire_count(&self) -> usize {
        let visible = usize::from(matches!(
            self.overlay,
            Some(Overlay::Questionnaire { .. })
                | Some(Overlay::ExitConfirm {
                    questionnaire: Some(_),
                    ..
                })
        ));
        visible.saturating_add(
            self.pending_prompts
                .iter()
                .filter(|prompt| matches!(prompt, PendingPrompt::Questionnaire(_)))
                .count(),
        )
    }

    /// Takes the next questionnaire result for the host interaction adapter.
    ///
    /// Every visible or queued request contributes at most one result. Taking
    /// removes it, so a host loop cannot answer one runtime responder twice.
    pub fn take_questionnaire_resolution(&mut self) -> Option<(String, QuestionnaireResolution)> {
        self.questionnaire_resolutions.pop_front()
    }

    /// Advances the animation clock.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.expire_prompts();
    }

    /// Whether the footer should ask for the confirming `Ctrl+C` press.
    pub fn ctrl_c_exit_hint_active(&self) -> bool {
        self.last_ctrl_c
            .is_some_and(|pressed| pressed.elapsed() < FORCE_QUIT_WINDOW)
    }

    /// Expires the first-press footer hint after the double-press window.
    ///
    /// Returns whether visible footer state changed so the host can request an
    /// idle redraw without continuously animating the status line.
    pub fn expire_ctrl_c_exit_hint(&mut self) -> bool {
        self.expire_ctrl_c_exit_hint_at(Instant::now())
    }

    pub(crate) fn expire_ctrl_c_exit_hint_at(&mut self, now: Instant) -> bool {
        let expired = self
            .last_ctrl_c
            .is_some_and(|pressed| now.duration_since(pressed) >= FORCE_QUIT_WINDOW);
        if expired {
            self.last_ctrl_c = None;
        }
        expired
    }

    /// Monotonic elapsed time for the active turn.
    pub fn turn_elapsed(&self) -> Option<Duration> {
        self.turn_started_at.map(|started| started.elapsed())
    }

    /// The ticking or frozen elapsed time for one delegated child.
    ///
    /// Absent for children known only through durable recovery: their live
    /// runtime was in another process, so no honest wall-clock exists.
    pub fn child_elapsed(&self, child: &str) -> Option<Duration> {
        self.child_clocks.get(child).map(ChildClock::elapsed)
    }

    /// Appends one bounded line to a child's retained progress log.
    ///
    /// Every child lifecycle event lands here, including the ones the root
    /// transcript deliberately stays quiet about: the inspector is the only
    /// place that record still exists in the client.
    pub(super) fn push_child_log(&mut self, child: &str, line: impl Into<String>) {
        let log = self.child_logs.entry(child.to_owned()).or_default();
        log.push(line.into());
        if log.len() > MAX_CHILD_LOG_LINES {
            log.remove(0);
        }
    }

    /// One child's retained progress log, oldest line first.
    pub fn child_log(&self, child: &str) -> &[String] {
        self.child_logs
            .get(child)
            .map_or(&[] as &[String], Vec::as_slice)
    }

    /// Children in the order the delegated-work panel lists them: live work
    /// first, settled children after, so keyboard order matches what the eye
    /// reads. Background shell tasks are not children and are skipped — they
    /// have no child session, log, or follow-up.
    pub fn inspectable_children(&self) -> Vec<&str> {
        let (live, settled): (Vec<_>, Vec<_>) = self
            .children
            .iter()
            .partition(|(_, summary)| summary.is_live());
        live.into_iter()
            .chain(settled)
            .map(|(child, _)| child.as_str())
            .collect()
    }

    /// Moves the inspector one row down the panel: the root timeline first,
    /// then each child. Stops at the last child rather than wrapping, so a
    /// held key settles somewhere predictable.
    ///
    /// Returns whether the selection moved.
    pub fn inspect_next_child(&mut self) -> bool {
        let children = self.inspectable_children();
        let next = match &self.inspected_child {
            None => children.first().map(|child| (*child).to_owned()),
            Some(current) => children
                .iter()
                .position(|child| child == current)
                .and_then(|index| children.get(index + 1))
                .map(|child| (*child).to_owned()),
        };
        match next {
            Some(child) => {
                self.inspect_child(child);
                true
            }
            None => false,
        }
    }

    /// Opens the read-only inspector on one child.
    pub fn inspect_child(&mut self, child: impl Into<String>) {
        let child = child.into();
        if self.inspected_child.as_deref() != Some(child.as_str()) {
            // The card belongs to the child it was polled for; carrying it
            // across a selection would report one child's turns under
            // another's name until the next redraw.
            self.inspected_detail = None;
        }
        self.inspected_child = Some(child);
        // The two views scroll independently; carrying one's offset into the
        // other would open a log part-way up for no reason the user can see.
        self.follow_newest();
    }

    /// The host's latest coordinator card for the inspected child.
    pub fn inspected_detail(&self) -> Option<&str> {
        self.inspected_detail.as_deref()
    }

    /// Records the host's latest coordinator card for the inspected child.
    ///
    /// Ignored unless it names the child currently on screen: a poll that
    /// answered after the user moved on describes a view that is gone.
    pub fn set_inspected_detail(&mut self, child: &str, detail: Option<String>) {
        if self.inspected_child.as_deref() == Some(child) {
            self.inspected_detail = detail;
        }
    }

    /// Moves the inspector one row up the panel, returning to the root
    /// timeline from the first child.
    ///
    /// Returns whether the selection moved.
    pub fn inspect_previous_child(&mut self) -> bool {
        let Some(current) = self.inspected_child.clone() else {
            return false;
        };
        let children = self.inspectable_children();
        match children
            .iter()
            .position(|child| *child == current)
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| children.get(index))
            .map(|child| (*child).to_owned())
        {
            Some(previous) => self.inspect_child(previous),
            None => {
                self.leave_child_inspection();
            }
        }
        true
    }

    /// Leaves the child inspector for the root timeline.
    ///
    /// Returns whether a child was being inspected.
    pub fn leave_child_inspection(&mut self) -> bool {
        let inspected = self.inspected_child.take().is_some();
        self.inspected_detail = None;
        if inspected {
            self.follow_newest();
        }
        inspected
    }

    /// Delegated children whose panel clock is still ticking.
    pub fn live_child_count(&self) -> usize {
        self.child_clocks
            .values()
            .filter(|clock| clock.is_live())
            .count()
    }

    /// Takes the parked live-stream gap so the host can replay it from the
    /// canonical journal.
    ///
    /// While a gap is parked the envelope that revealed it has **not** been
    /// applied. The host replays the missing range with
    /// [`App::apply_recovered`], then applies the gap's `deferred` envelope
    /// the same way.
    pub fn take_stream_gap(&mut self) -> Option<StreamGap> {
        self.stream_gap.take()
    }

    /// The live provider round-trip stage and how long it has been in it.
    pub fn provider_phase(&self) -> Option<(ProviderPhase, Duration)> {
        self.provider_phase
            .map(|(phase, since)| (phase, since.elapsed()))
    }

    /// Visible text from the newest live provider attempt.
    ///
    /// This is presentation-only speculative state. It is never returned from
    /// [`Transcript::blocks`] and therefore cannot become canonical history or
    /// journal-replayed output without an explicit runtime commit event.
    pub fn speculative_text(&self) -> Option<&str> {
        self.speculative_order.iter().rev().find_map(|key| {
            self.speculative_attempts
                .get(key)
                .map(|attempt| attempt.visible_text.as_str())
                .filter(|text| !text.is_empty())
        })
    }

    /// Number of provider attempts with output awaiting an explicit terminal.
    pub fn speculative_attempt_count(&self) -> usize {
        self.speculative_attempts.len()
    }

    /// Projects metadata-only process-exit reconciliation into the transcript.
    ///
    /// Child, monitor, and background-task identities remain in the
    /// protected recovery record; the UI only needs deterministic counts and
    /// the explicit fact that process-owned work was not restarted.
    pub fn present_recovered_ephemeral_work(
        &mut self,
        interrupted_children: usize,
        interrupted_monitors: usize,
        interrupted_tasks: usize,
    ) {
        let mut work = Vec::new();
        if interrupted_children > 0 {
            work.push(format!(
                "{interrupted_children} prior {}",
                if interrupted_children == 1 {
                    "child"
                } else {
                    "children"
                }
            ));
        }
        if interrupted_monitors > 0 {
            work.push(format!(
                "{interrupted_monitors} prior {}",
                if interrupted_monitors == 1 {
                    "monitor"
                } else {
                    "monitors"
                }
            ));
        }
        if interrupted_tasks > 0 {
            work.push(format!(
                "{interrupted_tasks} prior background {}",
                if interrupted_tasks == 1 {
                    "task"
                } else {
                    "tasks"
                }
            ));
        }
        if work.is_empty() {
            return;
        }
        self.transcript.push_notice(
            "recovery",
            format!(
                "{} interrupted when the prior Smith process exited · not restarted",
                work.join(" and ")
            ),
        );
    }

    /// Enriches a protected live tool event with a reviewed local projection.
    pub fn set_tool_display(&mut self, call_id: &str, display: ToolCallDisplay) {
        self.transcript.set_tool_display(call_id, display);
    }

    /// Attaches host-supplied, credential-redacted result lines to a tool row.
    pub fn set_tool_result_preview(&mut self, call_id: &str, preview: impl AsRef<str>) {
        self.transcript.set_tool_result_preview(call_id, preview);
    }

    /// Toggles bounded, redaction-safe tool detail beneath the working row.
    pub fn toggle_work_details(&mut self) {
        self.work_details = !self.work_details;
    }

    /// Render-ready lines for explicitly requested active-work detail.
    pub(crate) fn work_detail_lines(&self) -> Vec<String> {
        let Some(work) = &self.work else {
            return Vec::new();
        };
        if !self.work_details {
            return Vec::new();
        }
        work.tools
            .values()
            .take(12)
            .map(|(name, status, started_at)| match status {
                ToolStatus::Running => {
                    if let Some(started) = started_at {
                        format!(
                            "tool {name} · running {}",
                            render_elapsed(started.elapsed())
                        )
                    } else {
                        format!("tool {name} · {}", status.label())
                    }
                }
                _ => format!("tool {name} · {}", status.label()),
            })
            .collect()
    }
}

/// Finished copy for a child's declared workspace posture.
pub(super) fn describe_workspace(
    workspace: &agent_runtime_core::delegation::WorkspacePolicy,
) -> String {
    use agent_runtime_core::delegation::WorkspacePolicy;
    match workspace {
        WorkspacePolicy::SharedProject => "shared project workspace".to_owned(),
        WorkspacePolicy::ExplicitDirectory { path } => format!("workspace {path}"),
        WorkspacePolicy::IsolatedWorktree => "isolated worktree".to_owned(),
        WorkspacePolicy::ReadOnlyView => "read-only".to_owned(),
    }
}

/// Finished copy for why a child stopped.
pub(super) fn describe_cancel_reason(reason: &agent_runtime_core::cancel::CancelReason) -> String {
    use agent_runtime_core::cancel::CancelReason;
    match reason {
        CancelReason::UserRequested => "stopped by request".to_owned(),
        CancelReason::Timeout => "deadline elapsed".to_owned(),
        CancelReason::LimitReached => "limit reached".to_owned(),
        CancelReason::Shutdown => "session ended".to_owned(),
        CancelReason::Host(reason) => reason.clone(),
    }
}

/// Stable user-facing label for a child request's content-handling posture.
pub(super) fn describe_interaction_sensitivity(
    sensitivity: &agent_runtime_core::interaction::InteractionSensitivity,
) -> &'static str {
    use agent_runtime_core::interaction::InteractionSensitivity;
    match sensitivity {
        InteractionSensitivity::Public => "public",
        InteractionSensitivity::Sensitive => "sensitive",
    }
}

include!("tests/mod.rs");
