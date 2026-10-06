//! Core application state: the reducer over runtime events and key presses.
//!
//! [`App`] is deliberately free of I/O. It folds [`EventEnvelope`]s and
//! [`KeyEvent`]s into state and returns [`Action`]s for the host loop to
//! perform. Everything the screen shows is derivable from this struct, which is
//! what makes the renderer testable against a fake terminal and the key map
//! testable with no terminal at all.

mod children;

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::content::{ContentPart, UserInput};
use agent_runtime_core::ids::{AttemptId, RequestId, TurnId};
use agent_runtime_core::steer::SteerReceipt;
use agent_runtime_core::usage::CounterKind;
use smith_client::agent_report::{AgentSnapshot, ChildState as ReportChildState};
use smith_client::recovery_report::RestoreReport;
use smith_client::{Notice, NoticeKind, NoticePersistence};
use smith_host::approval::ApprovalPrompt;
use smith_host::rotation::RotationPrompt;
use smith_runtime::client::{PlanItemProjection, PlanSensitivity, SmithEvent as EventEnvelope};
use smith_tools::ToolCallDisplay;

use super::conversation::{Conversation, SpeculativeState};

use crate::commands::{HostCommand, SessionControl};
use crate::composer::Composer;
use crate::diff::EditReview;
use crate::picker::{ResourceEntry, ResourcePicker};
use crate::questionnaire::{QuestionnaireResolution, QuestionnaireState};
use crate::selection::Selection;
use crate::status::{Activity, Status, render_elapsed};
use crate::theme::Tone;
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
pub(super) const MOUSE_SCROLL_LINES: usize = 3;

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
    Reconfigure(SessionControl),
    /// Execute a local product command without sending composer text to the
    /// provider.
    Command(HostCommand),
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
    /// Record the already-confirmed decision to run one MCP server.
    ///
    /// The decision is written to the trust store by the host, which is also
    /// what makes the server connectable without restarting the session.
    TrustMcpServer {
        /// The declared server name.
        server: String,
    },
    /// Record the already-confirmed decision to activate one project skill.
    ///
    /// The host writes the decision to the trust store and recomposes the
    /// catalog at the next idle boundary, which is what makes the skill usable
    /// without restarting the session.
    TrustSkill {
        /// The project skill's name.
        skill: String,
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
    /// Advisor choices: default, on, off, then each profile or model that can
    /// advise this session.
    pub advisors: Vec<ResourceEntry>,
    /// Named context windows for the active binding.
    pub context_windows: Vec<ResourceEntry>,
    /// Active context window name when the binding has named windows.
    pub context_window: Option<String>,
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
    /// Who the session's advisor tool consults, if anyone.
    Advisor,
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
    /// Ephemeral keyboard guide in the anchored pane, never transcript history.
    Shortcuts,
    /// A tool is waiting for approval.
    Approval {
        /// What the runtime is asking to run, and the channel to answer on.
        prompt: Box<ApprovalPrompt>,
        /// The reviewable diff, when the request is an `edit` whose arguments
        /// parse. Other actions show their prepared material arguments.
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
        query: crate::line_input::LineInput,
        /// Stable history index of the selected match.
        selected: Option<usize>,
        /// Exact selected history entry, ready to restore into the composer.
        matched: Option<String>,
    },
    /// A consequential decision with no default answer.
    Confirm(ConfirmDialog),
}

impl Overlay {
    pub(super) fn is_prompt(&self) -> bool {
        matches!(
            self,
            Self::Approval { .. } | Self::Questionnaire { .. } | Self::Confirm(_)
        )
    }
}

/// Presentation and outcomes for one no-default confirmation.
#[derive(Debug)]
pub struct ConfirmDialog {
    /// The modal title.
    pub title: String,
    /// Border tone.
    pub tone: Tone,
    /// Fixed introductory line, with its existing presentation tone.
    pub warning: Option<(String, Tone)>,
    /// Complete body, wrapped and scrolled by the renderer.
    pub body: Vec<String>,
    /// Existing accept label.
    pub accept_label: String,
    /// Existing accept-key tone.
    pub accept_tone: Tone,
    /// Existing cancellation wording.
    pub cancel_label: String,
    /// Existing cancellation key label.
    pub cancel_key: &'static str,
    /// Existing compact footer wording.
    pub hint: String,
    /// Outcome consumed on acceptance, boxed to keep the overlay compact.
    pub accept: Box<ConfirmOutcome>,
    /// Outcome consumed on cancellation.
    pub cancel: Box<ConfirmOutcome>,
    /// Wrapped body offset.
    pub scroll: usize,
    /// Current viewport bound.
    pub scroll_limit: usize,
    /// Rotation owns its responder until either outcome consumes it.
    pub(crate) rotation: Option<Box<RotationPrompt>>,
}

impl ConfirmDialog {
    /// Builds a confirmation with ordinary cancel controls and no responder.
    pub fn new(
        title: &str,
        tone: Tone,
        body: Vec<String>,
        accept_label: &str,
        accept: ConfirmOutcome,
        cancel: ConfirmOutcome,
    ) -> Self {
        Self {
            title: title.to_owned(),
            tone,
            warning: None,
            body,
            accept_label: accept_label.to_owned(),
            accept_tone: tone,
            cancel_label: "cancel".to_owned(),
            cancel_key: "n/esc",
            hint: format!("y {accept_label} · n/esc cancel"),
            accept: Box::new(accept),
            cancel: Box::new(cancel),
            scroll: 0,
            scroll_limit: 0,
            rotation: None,
        }
    }
}

/// A host action or a decision resolved entirely within the terminal client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmOutcome {
    /// Return this action to the host.
    Action(Action),
    /// Close without a host action.
    Dismiss,
    /// Answer the owned rotation responder with its first eligible account.
    SwitchAccount,
    /// Decline the owned rotation responder.
    StayAccount,
}

/// A prompt waiting behind the visible overlay, in cross-type arrival order.
#[derive(Debug)]
pub(super) enum PendingPrompt {
    Approval(Box<ApprovalPrompt>, Option<EditReview>),
    Questionnaire(QuestionnaireState),
    Confirm(ConfirmDialog),
}

/// A root spawn call awaiting the child identity `ChildSpawned` will report.
///
/// `RuntimeEvent::ChildSpawned` carries no originating tool-call id, so the
/// correlation is host-side: the root's own event processing pushes one
/// entry here per spawn call, in the order the calls were made, and the
/// `ChildSpawned` handler pops the front entry to enrich that row. This is
/// deliberately an explicit FIFO rather than a scan — see
/// `App::note_pending_spawn` and `App::apply` in `reducer.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingSpawn {
    /// The spawn call's tool-call id.
    pub(super) call_id: String,
    /// The profile the spawn selected, when it selected one.
    pub(super) profile: Option<String>,
}

/// Most retained transcript blocks per child.
///
/// The child's own journal keeps the whole record; this is the bounded tail
/// the inspector can show without growing without limit in a long session.
pub(super) const MAX_CHILD_BLOCKS: usize = 200;

/// Longest child answer the client retains, in characters.
///
/// The answer is the one block worth reading in full, so the budget is
/// generous rather than a one-line summary — but it is still a budget, and
/// the `…` says so when it bites.
pub(super) const MAX_CHILD_ANSWER_CHARS: usize = 8_000;

/// How long a cleanly finished child keeps its panel row after settling.
///
/// Long enough to read the green outcome, short enough that a busy session
/// does not accumulate rows nobody is looking at. Any further activity from
/// that child brings the row straight back.
pub(super) const COMPLETED_CHILD_LINGER: Duration = Duration::from_secs(6);

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

/// Root-only presentation of an admitted provider retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderRetryProgress {
    /// One-based attempt number the runtime admitted next.
    pub next_attempt: u32,
    /// Configured total number of attempts, including the first.
    pub max_attempts: u32,
    /// Remaining backoff while the next attempt has not started. A zero
    /// duration is retained until the authoritative start event arrives.
    pub backoff_remaining: Option<Duration>,
}

#[derive(Debug, Clone)]
pub(super) struct ProviderRetryState {
    pub(super) next_attempt: u32,
    pub(super) max_attempts: u32,
    pub(super) delay: Duration,
    pub(super) received_at: Instant,
    pub(super) started: bool,
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

/// Root turn identity, active work, provider progress, and elapsed clocks.
#[derive(Debug, Default)]
pub(super) struct LiveTurn {
    pub(super) active_turn: Option<TurnId>,
    pub(super) work: Option<WorkSummary>,
    pub(super) provider_phase: Option<(ProviderPhase, Instant)>,
    pub(super) provider_retry: Option<ProviderRetryState>,
    pub(super) turn_started_at: Option<Instant>,
    pub(super) turn_started_timestamp: Option<Timestamp>,
}

impl LiveTurn {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The whole client's state.
#[derive(Debug)]
pub struct App {
    /// The transcript.
    pub transcript: Transcript,
    /// Keypress feedback, retained only until the next keypress.
    pub(super) feedback: Option<Notice>,
    /// Render-only wrapped rows; never part of conversation or input state.
    pub(crate) transcript_cache: RefCell<crate::render::TranscriptCache>,
    /// Header status.
    pub status: Status,
    /// Whether significant local cache-miss notices are enabled.
    pub cache_miss_notices: bool,
    /// The input buffer.
    pub composer: Composer,
    /// The current overlay, if any.
    pub overlay: Option<Overlay>,
    /// Quiet-window input guard for the visible consequential prompt.
    pub(super) prompt_input_guard: super::prompts::PromptInputGuard,
    /// Runtime prompts waiting behind the one visible overlay, in exact
    /// cross-type arrival order.
    pub(super) pending_prompts: VecDeque<PendingPrompt>,
    /// Questionnaire outcomes the host adapter has not consumed yet.
    pub(super) questionnaire_resolutions: VecDeque<(String, QuestionnaireResolution)>,
    /// Latest child states, keyed by stable child id.
    pub children: BTreeMap<String, ChildSummary>,
    /// Live panel clocks for children, keyed by stable child id.
    pub(super) child_clocks: BTreeMap<String, ChildClock>,
    /// Bounded per-child conversation, keyed by stable child id.
    ///
    /// Deliberately the same [`Conversation`] the root uses, folded by the
    /// same code from the same events — a child is a full runtime session, and
    /// the client subscribes to its stream directly. Child progress does not
    /// narrate itself into the root timeline, so this is where "what has that
    /// agent been doing" lives, in blocks rather than prose, drawn by the one
    /// renderer.
    pub(super) child_conversations: BTreeMap<String, Conversation>,
    /// When each cleanly finished child's panel row is due to retire.
    ///
    /// Armed when a child completes and nobody is inspecting it, disarmed the
    /// moment it does anything else. Kept beside — not inside —
    /// [`ChildSummary`] for the same reason [`ChildClock`] is: summaries are
    /// compared between live application and journal replay, and a live
    /// `Instant` can never replay equal.
    pub(super) child_dismiss_at: BTreeMap<String, Instant>,
    /// Children whose panel row has retired.
    ///
    /// Display state only. The child itself is still known — it still takes a
    /// follow-up, and any new activity puts its row back — because a row that
    /// scrolled off is not a child that stopped existing.
    pub(super) retired_children: BTreeSet<String>,
    /// Temporary child inspector selection; the root composer keeps focus.
    pub inspected_child: Option<String>,
    /// The host's latest coordinator card for [`Self::inspected_child`].
    ///
    /// The client has no delegation access of its own, so authoritative
    /// session, turn, token, and workspace figures arrive from the host on the
    /// same poll-on-redraw cadence as background tasks. Absent until that poll
    /// answers, which is honest: the client never invents child accounting.
    pub(super) inspected_detail: Option<AgentSnapshot>,
    /// Coordinator-reported turn and token counts, keyed by child id, kept
    /// current on the same poll-on-redraw as [`Self::inspected_detail`] —
    /// but for every visible child, not only the inspected one.
    pub(super) child_counts: BTreeMap<String, ChildCounts>,
    /// Root spawn calls awaiting the child identity `ChildSpawned` reports.
    pub(super) pending_spawns: VecDeque<PendingSpawn>,
    /// Per-counter usage delegated children reported on their own live
    /// streams this process observed, kept separate from
    /// [`Status`](crate::status::Status)'s root counters so the two can
    /// never be blended together; see `usage-accounting`'s "Delegated usage
    /// is accounted separately".
    pub(super) delegated_usage: BTreeMap<CounterKind, u64>,
    /// Child identities and frozen prices resolved by the host at spawn.
    pub(super) child_usage_bindings: BTreeMap<String, smith_client::status::BindingUsage>,
    /// Children that reported at least one delegated-usage record on their
    /// own live stream, each counted once regardless of how many records it
    /// sent.
    pub(super) delegated_contributors: BTreeSet<String>,
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
    /// The root turn's active presentation state.
    pub(super) live_turn: LiveTurn,
    /// Whether bounded tool output and live work detail are expanded.
    pub work_details: bool,
    /// Visual-row offset and bound for the approval's reviewable content.
    pub(crate) approval_scroll: u16,
    pub(crate) approval_scroll_limit: u16,
    /// The admitted local shortcut turn, used only to attribute its call row.
    /// Kept until its matching terminal, including across turn-start events.
    pub(super) local_shell_turn: Option<(TurnId, u64)>,
    /// Bounded local choices supplied by the host.
    pub resources: RuntimeResources,
    /// Whether the transcript follows new output.
    pub following: bool,
    /// Lines scrolled up from the bottom when not following.
    pub scroll_back: usize,
    /// Most lines the current transcript viewport can scroll.
    pub(crate) scroll_limit: usize,
    /// A local result to reveal from its beginning at the next valid frame.
    /// The renderer resolves this block index using the current wrap width.
    pub(crate) scroll_to_block: Option<usize>,
    /// Reading an informational result pauses following only until another
    /// block is appended; ordinary manual scrolling keeps its usual behavior.
    pub(crate) result_scroll_revision: Option<u64>,
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
    /// Rendering hides it as soon as a later block is appended.
    pub turn_summary: Option<String>,
    /// The turn's last block, rather than a later local result or notice.
    pub(super) turn_summary_revision: Option<u64>,
    pub(super) turn_block_revision: u64,
    /// Client-neutral accounting for the active root turn's output flow.
    /// Retained after completion until the next turn or session starts.
    pub turn_usage: crate::status::TurnUsage,
    pub(super) last_ctrl_c: Option<Instant>,
    pub(super) last_event_seq: Option<u64>,
    /// A live-stream sequence gap parked for host-driven journal replay.
    pub(super) stream_gap: Option<StreamGap>,
    /// Events recovered from the journal during the run of gaps currently
    /// being collapsed into one notice; see [`App::note_recovered_events`].
    pub(super) pending_recovered_events: usize,
    /// Merged span of sequence numbers permanently lost during the run of
    /// gaps currently being collapsed into one notice; see
    /// [`App::flush_gap_notices`].
    pub(super) pending_lost_range: Option<(u64, u64)>,
    /// The root conversation's held-back provider output. Its transcript is
    /// [`Self::transcript`]; the two are borrowed together as a
    /// [`ConversationMut`] whenever an event is folded into either.
    /// Finalized attempt identities survive completion until the next start.
    pub(super) speculative: SpeculativeState,
    /// Process-local, not-yet-canonical user input.
    pub(super) pending_input: PendingInputState,
    /// Last completed turn for which a local cache notice was appended.
    pub(super) last_cache_notice_turn: Option<String>,
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

mod child_models;
mod lifecycle;

pub use child_models::{ChildCounts, ChildState, ChildSummary};
