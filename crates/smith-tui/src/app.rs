//! Application state: the reducer over runtime events and key presses.
//!
//! [`App`] is deliberately free of I/O. It folds [`EventEnvelope`]s and
//! [`KeyEvent`]s into state and returns [`Action`]s for the host loop to
//! perform. Everything the screen shows is derivable from this struct, which is
//! what makes the renderer testable against a fake terminal and the key map
//! testable with no terminal at all.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use agent_runtime_core::clock::{SystemClock, Timestamp};
#[cfg(test)]
use agent_runtime_core::event::PlanItemStatus;
use agent_runtime_core::event::{
    ChildPhase, ChildRecoveryState, EventEnvelope, PlanItemProjection, PlanSensitivity,
    RuntimeEvent, TurnFinish,
};
use agent_runtime_core::ids::{AttemptId, RequestId};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use smith_host::approval::{ApprovalPrompt, PromptScope};
use smith_tools::ToolCallDisplay;

use crate::commands::{self, CommandAction};
use crate::composer::Composer;
use crate::diff::EditReview;
use crate::picker::{PickerOutcome, ResourceEntry, ResourcePicker};
use crate::questionnaire::{QuestionnaireForm, QuestionnaireResolution, QuestionnaireState};
use crate::references::{ComposerReference, parse_references};
use crate::status::{Activity, ContextPlanUpdate, Status, render_elapsed, render_terminal_elapsed};
use crate::transcript::{LocalResultState, ToolStatus, Transcript};

/// How long a second `Ctrl+C` still counts as the exit press.
const FORCE_QUIT_WINDOW: Duration = Duration::from_secs(1);

/// Something the host loop must do on the app's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Submit this text as a user turn.
    Send(String),
    /// Submit a user turn after exact local file reads are attached.
    SendWithFiles {
        /// Prompt text, including visible `@path` provenance.
        text: String,
        /// Canonical workspace-relative file identities.
        files: Vec<String>,
    },
    /// Execute one explicit local shell shortcut without provider spend.
    RunShell {
        /// Command after the leading `!` marker.
        command: String,
    },
    /// Cancel the running turn.
    Interrupt,
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
    /// Start one confirmed, host-registered read-only child preset.
    StartAgent {
        /// Registered child preset.
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
    /// Select a root-agent mode at a safe session boundary.
    Agent(String),
}

/// Bounded local resources available to runtime pickers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeResources {
    /// Provider-qualified models.
    pub models: Vec<ResourceEntry>,
    /// Configured providers.
    pub providers: Vec<ResourceEntry>,
    /// Configured profiles.
    pub profiles: Vec<ResourceEntry>,
    /// Project-scoped saved sessions.
    pub sessions: Vec<ResourceEntry>,
    /// Bounded canonical workspace-file index.
    pub files: Vec<ResourceEntry>,
    /// Host-registered read-only child presets.
    pub child_agents: Vec<ResourceEntry>,
    /// Authorized root-agent modes in configured cycle order.
    pub agent_modes: Vec<ResourceEntry>,
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
    /// Coherent configured profile.
    Profile,
    /// Project session.
    Resume,
    /// Insert one typed file or child-agent reference into the composer.
    Reference,
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
enum PendingPrompt {
    Approval(Box<ApprovalPrompt>, Option<EditReview>),
    Questionnaire(QuestionnaireState),
}

/// One background notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// Where it came from, e.g. a monitor name.
    pub source: String,
    /// The one-line summary.
    pub text: String,
    /// Whether it is terminal — a monitor stopping, a child finishing.
    ///
    /// Terminal events are never coalesced away, because "the monitor died" is
    /// not noise even when it arrives amid noise.
    pub terminal: bool,
}

/// The latest user-visible state of one child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSummary {
    /// Current lifecycle label.
    pub state: String,
    /// Latest bounded result or detail.
    pub detail: Option<String>,
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
struct WorkSummary {
    tools: BTreeMap<String, (String, ToolStatus)>,
}

/// One provider attempt's speculative presentation identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AttemptOutputKey {
    request: RequestId,
    attempt: AttemptId,
}

impl AttemptOutputKey {
    fn new(request: &RequestId, attempt: &AttemptId) -> Self {
        Self {
            request: request.clone(),
            attempt: attempt.clone(),
        }
    }
}

/// A delta retained outside the canonical transcript until an explicit commit.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SpeculativeChunk {
    Text(String),
    Reasoning { text: String, redacted: bool },
}

/// Buffered output for one in-flight provider attempt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SpeculativeAttempt {
    chunks: Vec<SpeculativeChunk>,
    visible_text: String,
}

impl SpeculativeAttempt {
    fn push_text(&mut self, text: &str) {
        self.visible_text.push_str(text);
        if let Some(SpeculativeChunk::Text(previous)) = self.chunks.last_mut() {
            previous.push_str(text);
        } else {
            self.chunks.push(SpeculativeChunk::Text(text.to_owned()));
        }
    }

    fn push_reasoning(&mut self, text: &str, redacted: bool) {
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
    pending_prompts: VecDeque<PendingPrompt>,
    /// Questionnaire outcomes the host adapter has not consumed yet.
    questionnaire_resolutions: VecDeque<(String, QuestionnaireResolution)>,
    /// Latest child states, keyed by stable child id.
    pub children: BTreeMap<String, ChildSummary>,
    /// Temporary child inspector selection; the root composer keeps focus.
    pub inspected_child: Option<String>,
    /// Latest durable todo plan, projected in the anchored composer pane.
    pub plan: Option<PlanSummary>,
    /// Bounded live tool detail available only through `/details`.
    work: Option<WorkSummary>,
    /// Whether bounded live tool details are expanded.
    pub work_details: bool,
    /// Bounded local choices supplied by the host.
    pub resources: RuntimeResources,
    /// Whether the transcript follows new output.
    pub following: bool,
    /// Lines scrolled up from the bottom when not following.
    pub scroll_back: u16,
    /// Most lines the current transcript viewport can scroll.
    scroll_limit: u16,
    /// The animation tick.
    pub tick: u64,
    /// Set once the host loop should exit.
    pub should_quit: bool,
    turn_started_at: Option<Instant>,
    turn_started_timestamp: Option<Timestamp>,
    last_ctrl_c: Option<Instant>,
    last_event_seq: Option<u64>,
    speculative_attempts: BTreeMap<AttemptOutputKey, SpeculativeAttempt>,
    speculative_order: Vec<AttemptOutputKey>,
    finalized_attempts: BTreeSet<AttemptOutputKey>,
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
            inspected_child: None,
            plan: None,
            work: None,
            work_details: false,
            resources: RuntimeResources::default(),
            following: true,
            scroll_back: 0,
            scroll_limit: 0,
            tick: 0,
            should_quit: false,
            turn_started_at: None,
            turn_started_timestamp: None,
            last_ctrl_c: None,
            last_event_seq: None,
            speculative_attempts: BTreeMap::new(),
            speculative_order: Vec::new(),
            finalized_attempts: BTreeSet::new(),
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
            || !self.pending_prompts.is_empty()
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

    /// Renders a background notification inline without stealing composer
    /// focus.
    pub fn notify(&mut self, notification: Notification) {
        let suffix = if notification.terminal {
            " · finished"
        } else {
            ""
        };
        self.transcript.push_notice(
            notification.source,
            format!("{}{suffix}", notification.text),
        );
    }

    /// Projects metadata-only process-exit reconciliation into the transcript.
    ///
    /// Child and monitor identities remain in the protected recovery record;
    /// the UI only needs deterministic counts and the explicit fact that
    /// process-owned work was not restarted.
    pub fn present_recovered_ephemeral_work(
        &mut self,
        interrupted_children: usize,
        interrupted_monitors: usize,
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
            .map(|(name, status)| format!("tool {name} · {}", status.label()))
            .collect()
    }

    fn finish_work(&mut self) {
        self.work = None;
    }

    fn begin_speculative_attempt(&mut self, request: &RequestId, attempt: &AttemptId) {
        let key = AttemptOutputKey::new(request, attempt);
        if self.finalized_attempts.contains(&key) {
            self.transcript.push_error(format!(
                "provider attempt {} for request {} restarted after its output terminal",
                attempt, request
            ));
            return;
        }
        if !self.speculative_attempts.contains_key(&key) {
            self.speculative_order.push(key.clone());
            self.speculative_attempts
                .insert(key, SpeculativeAttempt::default());
        }
    }

    fn buffer_speculative_text(&mut self, request: &RequestId, attempt: &AttemptId, text: &str) {
        self.begin_speculative_attempt(request, attempt);
        let key = AttemptOutputKey::new(request, attempt);
        if let Some(output) = self.speculative_attempts.get_mut(&key) {
            output.push_text(text);
        }
    }

    fn buffer_speculative_reasoning(
        &mut self,
        request: &RequestId,
        attempt: &AttemptId,
        text: &str,
        redacted: bool,
    ) {
        self.begin_speculative_attempt(request, attempt);
        let key = AttemptOutputKey::new(request, attempt);
        if let Some(output) = self.speculative_attempts.get_mut(&key) {
            output.push_reasoning(text, redacted);
        }
    }

    fn finish_speculative_attempt(
        &mut self,
        request: &RequestId,
        attempt: &AttemptId,
        commit: bool,
    ) {
        let key = AttemptOutputKey::new(request, attempt);
        let Some(output) = self.speculative_attempts.remove(&key) else {
            if !self.finalized_attempts.contains(&key) {
                self.transcript.push_error(format!(
                    "provider attempt {} for request {} ended without a start",
                    attempt, request
                ));
                self.finalized_attempts.insert(key);
            }
            return;
        };
        self.speculative_order.retain(|candidate| candidate != &key);
        if !self.finalized_attempts.insert(key) {
            return;
        }

        if commit {
            for chunk in output.chunks {
                match chunk {
                    SpeculativeChunk::Text(text) => self.transcript.push_text_delta(&text),
                    SpeculativeChunk::Reasoning { text, redacted } => {
                        self.transcript.push_reasoning_delta(&text, redacted);
                    }
                }
            }
        } else if !output.chunks.is_empty() {
            self.transcript.push_notice(
                "retry",
                format!("discarded speculative output from provider attempt {attempt}"),
            );
        }
    }

    fn discard_orphaned_speculative_output(&mut self, boundary: &str) {
        let orphaned = self.speculative_attempts.len();
        if orphaned == 0 {
            return;
        }
        self.speculative_attempts.clear();
        self.speculative_order.clear();
        self.transcript.push_notice(
            "integrity",
            format!(
                "discarded {orphaned} unterminated speculative provider attempt(s) at {boundary}"
            ),
        );
    }

    /// Folds one runtime event into state.
    pub fn apply(&mut self, envelope: &EventEnvelope) {
        if let Some(previous) = self.last_event_seq
            && envelope.seq > previous.saturating_add(1)
        {
            self.transcript.push_error(format!(
                "live event stream skipped sequence {} through {}; \
                 the persisted session journal remains canonical",
                previous.saturating_add(1),
                envelope.seq.saturating_sub(1)
            ));
        }
        self.last_event_seq = Some(envelope.seq);

        match &envelope.payload {
            RuntimeEvent::SessionStarted => {
                self.status.activity = Activity::Idle;
                self.status.capabilities = Default::default();
                self.status.context_plan = None;
                self.plan = None;
                self.work = None;
                self.turn_started_at = None;
                self.turn_started_timestamp = None;
                self.speculative_attempts.clear();
                self.speculative_order.clear();
                self.finalized_attempts.clear();
            }
            RuntimeEvent::TurnStarted => {
                self.discard_orphaned_speculative_output("next turn start");
                self.status.activity = Activity::Working;
                self.plan = None;
                self.work = Some(WorkSummary::default());
                self.turn_started_at = Some(Instant::now());
                self.turn_started_timestamp =
                    (envelope.timestamp != Timestamp::ZERO).then_some(envelope.timestamp);
                self.finalized_attempts.clear();
            }
            RuntimeEvent::ModelProfileResolved {
                provider, model, ..
            } => {
                let changed = self.status.provider.as_deref() != Some(provider.as_str())
                    || self.status.model != model.as_str();
                let had_provider = self.status.provider.is_some();
                if changed {
                    if had_provider {
                        self.transcript.push_notice(
                            "provider",
                            format!("changed to {provider}/{model} · prior cache not transferable"),
                        );
                    }
                    self.status
                        .switch_model(Some(provider.clone()), model.as_str());
                }
            }
            RuntimeEvent::RegistrySnapshotSealed { snapshot, entries } => {
                self.status.record_registry(snapshot.as_str(), *entries);
            }
            RuntimeEvent::ScopedViewDerived {
                view,
                visible_entries,
                ..
            } => {
                self.status
                    .record_scoped_view(view.as_str(), *visible_entries);
            }
            RuntimeEvent::CapabilityRetrievalPerformed {
                resolver_revision,
                candidates,
                ..
            } => {
                self.status.record_retrieval(
                    resolver_revision.as_str(),
                    candidates
                        .iter()
                        .take(16)
                        .map(ToString::to_string)
                        .collect(),
                );
            }
            RuntimeEvent::CapabilitiesActivated { epoch, activation } => {
                let capabilities = activation
                    .iter()
                    .take(16)
                    .map(|capability| capability.id.to_string())
                    .collect::<Vec<_>>();
                self.status.record_activation(*epoch, capabilities.clone());
                self.transcript.push_notice(
                    "capabilities",
                    if capabilities.is_empty() {
                        format!("activation epoch {epoch} has no optional capabilities")
                    } else {
                        format!("activation epoch {epoch}: {}", capabilities.join(", "))
                    },
                );
            }
            RuntimeEvent::ProviderAttemptStarted {
                request, attempt, ..
            } => {
                self.begin_speculative_attempt(request, attempt);
            }
            RuntimeEvent::TextDelta {
                request,
                attempt,
                text,
            } => {
                self.buffer_speculative_text(request, attempt, text);
            }
            RuntimeEvent::ReasoningDelta {
                request,
                attempt,
                text,
                redacted,
            } => {
                self.buffer_speculative_reasoning(request, attempt, text, *redacted);
            }
            RuntimeEvent::ProviderAttemptOutputCommitted { request, attempt } => {
                self.finish_speculative_attempt(request, attempt, true);
            }
            RuntimeEvent::ProviderAttemptOutputDiscarded { request, attempt } => {
                self.finish_speculative_attempt(request, attempt, false);
            }
            RuntimeEvent::ToolCallRequested {
                call,
                name,
                argument_keys,
                arguments,
                ..
            } => {
                if let Some(work) = &mut self.work {
                    work.tools.insert(
                        call.as_str().to_owned(),
                        (name.clone(), ToolStatus::Running),
                    );
                }
                self.transcript.push_tool_call(
                    call.as_str(),
                    name,
                    arguments.as_ref(),
                    argument_keys,
                );
            }
            RuntimeEvent::ToolCallCompleted { call, is_error, .. } => {
                let status = if *is_error {
                    ToolStatus::Failed
                } else {
                    ToolStatus::Ok
                };
                if let Some(work) = &mut self.work
                    && let Some((_, work_status)) = work.tools.get_mut(call.as_str())
                {
                    *work_status = status;
                }
                self.transcript.complete_tool_call(call.as_str(), status);
            }
            RuntimeEvent::ContextPlanned {
                context,
                cache_plan,
                segment_count,
                totals,
                input_tokens,
                input_budget_tokens,
                reserved_tokens,
                confidence,
                ..
            } => {
                self.status.record_context_plan(ContextPlanUpdate {
                    fingerprint: context.as_str(),
                    cache_fingerprint: cache_plan.as_str(),
                    input_tokens: *input_tokens,
                    input_budget_tokens: *input_budget_tokens,
                    reserved_tokens: *reserved_tokens,
                    segment_count: *segment_count,
                    totals,
                    confidence: *confidence,
                });
            }
            RuntimeEvent::ContextCompacted {
                reclaimed_tokens,
                evicted,
                summaries,
                ..
            } => {
                self.status.record_compaction(*reclaimed_tokens);
                self.transcript.push_notice(
                    "context",
                    format!(
                        "compacted context · reclaimed {reclaimed_tokens} tokens · \
                         {} evicted · {} summaries",
                        evicted.len(),
                        summaries.len()
                    ),
                );
            }
            RuntimeEvent::PlanUpdated {
                revision,
                sensitivity,
                counts,
                items,
            } => {
                let plan = PlanSummary {
                    revision: *revision,
                    sensitivity: *sensitivity,
                    counts: counts.clone(),
                    items: if *sensitivity == PlanSensitivity::Public {
                        items.clone()
                    } else {
                        None
                    },
                };
                self.plan = Some(plan);
            }
            RuntimeEvent::Usage { record } => {
                self.status.record_usage(&record.delta);
            }
            RuntimeEvent::CacheObservation { read_tokens, .. } => {
                self.status.record_cache(*read_tokens);
            }
            RuntimeEvent::Downgrade { capability, detail } => {
                self.transcript
                    .push_notice("downgrade", format!("{capability}: {detail}"));
            }
            RuntimeEvent::LimitReached { limit } => {
                self.transcript
                    .push_notice("limit", format!("{limit:?} reached"));
            }
            RuntimeEvent::Error { error } => {
                self.transcript.push_error(error.to_string());
            }
            RuntimeEvent::TurnCompleted { finish, .. } => {
                self.cancel_pending_prompts();
                // A valid v5 stream has already committed or discarded every
                // attempt. A gap, corrupt journal, or incompatible producer
                // must fail closed: orphan text cannot become visible while
                // idle or leak into the next turn.
                self.discard_orphaned_speculative_output("turn completion");
                let elapsed = self.turn_started_at.take().map(|started| started.elapsed());
                let terminal_elapsed = self.turn_started_timestamp.take().and_then(|started| {
                    envelope
                        .timestamp
                        .as_millis()
                        .checked_sub(started.as_millis())
                        .map(Duration::from_millis)
                });
                self.transcript.close_open();
                self.status.activity = Activity::Idle;
                self.finish_work();
                match finish {
                    TurnFinish::Cancelled { reason } => {
                        self.transcript.push_notice(
                            "turn",
                            elapsed.map_or_else(
                                || format!("interrupted ({reason:?})"),
                                |elapsed| {
                                    format!(
                                        "interrupted after {} ({reason:?})",
                                        render_elapsed(elapsed)
                                    )
                                },
                            ),
                        );
                    }
                    TurnFinish::LimitReached { limit } => {
                        self.transcript.push_notice(
                            "turn",
                            elapsed.map_or_else(
                                || format!("stopped at the {limit:?} limit"),
                                |elapsed| {
                                    format!(
                                        "stopped after {} at the {limit:?} limit",
                                        render_elapsed(elapsed)
                                    )
                                },
                            ),
                        );
                    }
                    TurnFinish::NeedsInput { request } => {
                        self.transcript.push_notice(
                            "turn",
                            elapsed.map_or_else(
                                || format!("waiting for parent input · request {request}"),
                                |elapsed| {
                                    format!(
                                        "waiting for parent input after {} · request {request}",
                                        render_elapsed(elapsed)
                                    )
                                },
                            ),
                        );
                    }
                    TurnFinish::Completed => {
                        self.transcript.push_notice(
                            "turn",
                            terminal_elapsed.map_or_else(
                                || "completed".to_owned(),
                                |elapsed| {
                                    format!("completed in {}", render_terminal_elapsed(elapsed))
                                },
                            ),
                        );
                    }
                    TurnFinish::Failed => {
                        self.transcript.push_notice(
                            "turn",
                            elapsed.map_or_else(
                                || "failed".to_owned(),
                                |elapsed| format!("failed after {}", render_elapsed(elapsed)),
                            ),
                        );
                    }
                }
            }
            // Child lifecycle appears immediately, before the parent model is
            // told anything: results reach the model only at a safe boundary,
            // but the user watches the child work in real time.
            RuntimeEvent::ChildSpawned {
                child,
                workspace,
                max_turns,
                ..
            } => {
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "running".to_owned(),
                        detail: Some(describe_workspace(workspace)),
                    },
                );
                self.transcript.push_notice(
                    "sub-agent",
                    format!(
                        "{child} started · {} · up to {max_turns} turns",
                        describe_workspace(workspace)
                    ),
                );
            }
            RuntimeEvent::ChildProgress { child, phase } => match phase {
                ChildPhase::Recovered {
                    child_session,
                    state,
                    resumable,
                } => {
                    let state = match state {
                        ChildRecoveryState::Idle => "idle",
                        ChildRecoveryState::Interrupted => "interrupted",
                        ChildRecoveryState::Blocked => "blocked",
                        ChildRecoveryState::Expired => "expired",
                        ChildRecoveryState::Terminal => "terminal",
                    };
                    let detail = format!(
                        "durable · session {child_session}{}",
                        if *resumable { " · resumable" } else { "" }
                    );
                    self.children.insert(
                        child.to_string(),
                        ChildSummary {
                            state: state.to_owned(),
                            detail: Some(detail.clone()),
                        },
                    );
                    self.transcript
                        .push_notice("sub-agent", format!("{child} recovered {state} · {detail}"));
                }
                ChildPhase::TurnStarted => {
                    if let Some(summary) = self.children.get_mut(&child.to_string()) {
                        summary.state = "working".to_owned();
                    }
                    self.transcript
                        .push_notice("sub-agent", format!("{child} is working"));
                }
                ChildPhase::ResumeStarted { child_session } => {
                    self.children.insert(
                        child.to_string(),
                        ChildSummary {
                            state: "resuming".to_owned(),
                            detail: Some(format!("exact checkpoint · session {child_session}")),
                        },
                    );
                    self.transcript.push_notice(
                        "sub-agent",
                        format!("{child} is resuming its exact checkpoint"),
                    );
                }
                ChildPhase::Interrupted {
                    child_session,
                    resumable,
                } => {
                    let detail = format!(
                        "durable · session {child_session}{}",
                        if *resumable {
                            " · exact resume available"
                        } else {
                            " · no compatible checkpoint"
                        }
                    );
                    self.children.insert(
                        child.to_string(),
                        ChildSummary {
                            state: "interrupted".to_owned(),
                            detail: Some(detail.clone()),
                        },
                    );
                    self.transcript
                        .push_notice("sub-agent", format!("{child} interrupted · {detail}"));
                }
                ChildPhase::ToolCall { name } => {
                    if let Some(summary) = self.children.get_mut(&child.to_string()) {
                        summary.detail = Some(format!("ran {name}"));
                    }
                    self.transcript
                        .push_notice("sub-agent", format!("{child} ran {name}"));
                }
                // The completed/stopped notice that follows says everything a
                // bare "finished a turn" would.
                ChildPhase::TurnFinished => {}
            },
            RuntimeEvent::ChildNeedsInput {
                child,
                request,
                question_ids,
                sensitivity,
                ..
            } => {
                let question_count = question_ids.len();
                let detail = format!(
                    "{question_count} {} · {} · request {request}",
                    if question_count == 1 {
                        "question"
                    } else {
                        "questions"
                    },
                    describe_interaction_sensitivity(sensitivity),
                );
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "needs input".to_owned(),
                        detail: Some(detail.clone()),
                    },
                );
                self.transcript
                    .push_notice("sub-agent", format!("{child} needs input · {detail}"));
            }
            RuntimeEvent::ChildCompleted { child, result } => {
                let mut summary: String = result.chars().take(200).collect();
                if summary.chars().count() < result.chars().count() {
                    summary.push('…');
                }
                if summary.is_empty() {
                    summary = "(no visible answer)".to_owned();
                }
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "completed".to_owned(),
                        detail: Some(summary.clone()),
                    },
                );
                self.transcript
                    .push_notice("sub-agent", format!("{child} completed: {summary}"));
            }
            RuntimeEvent::ChildStopped { child, reason } => {
                let detail = describe_cancel_reason(reason);
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "stopped".to_owned(),
                        detail: Some(detail.clone()),
                    },
                );
                self.transcript
                    .push_notice("sub-agent", format!("{child} stopped · {detail}"));
            }
            RuntimeEvent::ChildFailed { child, error } => {
                self.children.insert(
                    child.to_string(),
                    ChildSummary {
                        state: "failed".to_owned(),
                        detail: Some(error.message.clone()),
                    },
                );
                self.transcript
                    .push_error(format!("sub-agent {child} failed: {}", error.message));
            }
            RuntimeEvent::SessionShutdown => {
                self.cancel_pending_prompts();
                self.discard_orphaned_speculative_output("session shutdown");
                self.transcript.close_open();
                self.status.activity = Activity::Ended;
                self.turn_started_at = None;
                self.turn_started_timestamp = None;
            }
            // Planning-lifecycle events carry diagnostics the basic TUI does not
            // surface yet; they are recorded by the session log regardless.
            _ => {}
        }
    }

    /// Presents an approval request.
    ///
    /// The diff is derived here rather than in the renderer: the arguments
    /// cannot change while the modal is open, and the redraw budget is 30 fps
    /// (`DESIGN.md` §6), so paying for it once per request is the whole cost.
    pub fn present_approval(&mut self, prompt: ApprovalPrompt) {
        if prompt.deadline().is_expired(&SystemClock) {
            prompt.time_out();
            self.transcript.push_notice(
                "approval",
                "approval timed out before it could be presented",
            );
            return;
        }
        let review = EditReview::from_call(prompt.tool(), prompt.prepared().arguments());
        let approval = PendingPrompt::Approval(Box::new(prompt), review);
        if self.overlay.is_none() {
            self.show_prompt(approval);
        } else {
            self.pending_prompts.push_back(approval);
        }
    }

    /// Presents one authority-free questionnaire.
    pub fn present_questionnaire(&mut self, form: QuestionnaireForm) {
        let request_id = form.request_id.clone();
        if form.deadline.is_expired(&SystemClock) {
            self.questionnaire_resolutions
                .push_back((request_id, QuestionnaireResolution::TimedOut));
            self.transcript.push_notice(
                "questionnaire",
                "question timed out before it could be presented",
            );
            return;
        }
        let prompt = PendingPrompt::Questionnaire(QuestionnaireState::new(form));
        if self.overlay.is_none() {
            self.show_prompt(prompt);
        } else {
            self.pending_prompts.push_back(prompt);
        }
    }

    /// Removes a runtime-closed questionnaire without manufacturing a second
    /// host response.
    ///
    /// The runtime calls its interaction broker's synchronous close hook when
    /// cancellation or its deadline wins, including when the broker future
    /// was dropped. The host adapter projects that close here so a visible or
    /// queued overlay cannot outlive the owning turn.
    pub fn dismiss_questionnaire(&mut self, request_id: &str) {
        self.overlay = match self.overlay.take() {
            Some(Overlay::Questionnaire { state }) if state.form().request_id == request_id => None,
            Some(Overlay::ExitConfirm {
                approval,
                questionnaire: Some(state),
            }) if state.form().request_id == request_id => Some(Overlay::ExitConfirm {
                approval,
                questionnaire: None,
            }),
            other => other,
        };
        self.pending_prompts.retain(|prompt| {
            !matches!(
                prompt,
                PendingPrompt::Questionnaire(state)
                    if state.form().request_id == request_id
            )
        });
        self.present_next_prompt();
    }

    fn show_prompt(&mut self, prompt: PendingPrompt) {
        self.overlay = Some(match prompt {
            PendingPrompt::Approval(prompt, review) => Overlay::Approval { prompt, review },
            PendingPrompt::Questionnaire(state) => Overlay::Questionnaire { state },
        });
    }

    fn present_next_prompt(&mut self) {
        if self.overlay.is_some() {
            return;
        }
        if let Some(prompt) = self.pending_prompts.pop_front() {
            self.show_prompt(prompt);
        }
    }

    fn expire_prompts(&mut self) {
        let mut expired_approvals = 0_usize;
        let mut expired_questions = 0_usize;
        self.overlay = match self.overlay.take() {
            Some(Overlay::Approval { prompt, review }) => {
                if prompt.deadline().is_expired(&SystemClock) {
                    prompt.time_out();
                    expired_approvals += 1;
                    None
                } else {
                    Some(Overlay::Approval { prompt, review })
                }
            }
            Some(Overlay::Questionnaire { state }) => {
                if state.form().deadline.is_expired(&SystemClock) {
                    self.resolve_questionnaire(state, QuestionnaireResolution::TimedOut);
                    expired_questions += 1;
                    None
                } else {
                    Some(Overlay::Questionnaire { state })
                }
            }
            Some(Overlay::ExitConfirm {
                approval: Some((prompt, review)),
                questionnaire,
            }) => {
                if prompt.deadline().is_expired(&SystemClock) {
                    prompt.time_out();
                    expired_approvals += 1;
                    Some(Overlay::ExitConfirm {
                        approval: None,
                        questionnaire,
                    })
                } else {
                    Some(Overlay::ExitConfirm {
                        approval: Some((prompt, review)),
                        questionnaire,
                    })
                }
            }
            Some(Overlay::ExitConfirm {
                approval,
                questionnaire: Some(state),
            }) => {
                if state.form().deadline.is_expired(&SystemClock) {
                    self.resolve_questionnaire(state, QuestionnaireResolution::TimedOut);
                    expired_questions += 1;
                    Some(Overlay::ExitConfirm {
                        approval,
                        questionnaire: None,
                    })
                } else {
                    Some(Overlay::ExitConfirm {
                        approval,
                        questionnaire: Some(state),
                    })
                }
            }
            other => other,
        };

        let mut waiting = VecDeque::with_capacity(self.pending_prompts.len());
        while let Some(prompt) = self.pending_prompts.pop_front() {
            match prompt {
                PendingPrompt::Approval(prompt, review) => {
                    if prompt.deadline().is_expired(&SystemClock) {
                        prompt.time_out();
                        expired_approvals += 1;
                    } else {
                        waiting.push_back(PendingPrompt::Approval(prompt, review));
                    }
                }
                PendingPrompt::Questionnaire(state) => {
                    if state.form().deadline.is_expired(&SystemClock) {
                        self.resolve_questionnaire(state, QuestionnaireResolution::TimedOut);
                        expired_questions += 1;
                    } else {
                        waiting.push_back(PendingPrompt::Questionnaire(state));
                    }
                }
            }
        }
        self.pending_prompts = waiting;

        if expired_approvals > 0 {
            self.transcript.push_notice(
                "approval",
                format!("timed out {expired_approvals} pending approval request(s)"),
            );
        }
        if expired_questions > 0 {
            self.transcript.push_notice(
                "questionnaire",
                format!("timed out {expired_questions} pending question request(s)"),
            );
        }
        self.present_next_prompt();
    }

    fn answer_approval(&mut self, allow: Option<PromptScope>) {
        let Some(Overlay::Approval { prompt, .. }) = self.overlay.take() else {
            return;
        };
        let tool = prompt.tool().to_owned();
        match allow {
            Some(scope) => {
                prompt.allow(scope);
                if scope == PromptScope::Session {
                    self.transcript
                        .push_notice("approval", format!("{tool} allowed for this session"));
                }
            }
            None => {
                prompt.deny("the user declined");
                self.transcript
                    .push_notice("approval", format!("{tool} denied"));
                self.transcript
                    .complete_tool_call_by_name(&tool, ToolStatus::Denied);
            }
        }
        self.present_next_prompt();
    }

    fn resolve_questionnaire(
        &mut self,
        state: QuestionnaireState,
        resolution: QuestionnaireResolution,
    ) {
        let request_id = state.form().request_id.clone();
        let notice = match &resolution {
            QuestionnaireResolution::Submitted(_) => "submitted",
            QuestionnaireResolution::Declined => "declined",
            QuestionnaireResolution::Cancelled => "cancelled",
            QuestionnaireResolution::TimedOut => "timed out",
        };
        self.questionnaire_resolutions
            .push_back((request_id, resolution));
        self.transcript.push_notice("questionnaire", notice);
    }

    /// Handles a key press, returning an action for the host loop.
    pub fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        let action = self.reduce_key(key);
        if !matches!(action, Some(Action::Quit | Action::Reconfigure(_))) {
            self.present_next_prompt();
        }
        action
    }

    fn reduce_key(&mut self, key: KeyEvent) -> Option<Action> {
        // Terminals that report both press and release would otherwise act on
        // every keystroke twice.
        if key.kind == KeyEventKind::Release {
            return None;
        }

        // Ctrl+C is checked before overlays: two consecutive presses must
        // always be able to leave, even while a prompt owns input.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return self.on_ctrl_c();
        }
        self.last_ctrl_c = None;

        match &self.overlay {
            Some(Overlay::Approval { .. }) => return self.on_approval_key(key),
            Some(Overlay::Questionnaire { .. }) => return self.on_questionnaire_key(key),
            Some(Overlay::Palette { .. }) => return self.on_palette_key(key),
            Some(Overlay::ResourcePicker { .. }) => {
                return self.on_resource_picker_key(key);
            }
            Some(Overlay::UndoConfirm { .. }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        self.overlay = None;
                        Some(Action::ApplyUndo)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = None;
                        Some(Action::CancelUndo)
                    }
                    _ => None,
                };
            }
            Some(Overlay::RedoConfirm { .. }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        self.overlay = None;
                        Some(Action::ApplyRedo)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = None;
                        Some(Action::CancelRedo)
                    }
                    _ => None,
                };
            }
            Some(Overlay::RevertConfirm {
                scope, fingerprint, ..
            }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        let action = Action::ApplyRevert {
                            scope: scope.clone(),
                            fingerprint: fingerprint.clone(),
                        };
                        self.overlay = None;
                        Some(action)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        let action = Action::CancelRevert {
                            scope: scope.clone(),
                            fingerprint: fingerprint.clone(),
                        };
                        self.overlay = None;
                        Some(action)
                    }
                    _ => None,
                };
            }
            Some(Overlay::ReviewConfirm { scope, .. }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        let action = Action::StartReview {
                            scope: scope.clone(),
                        };
                        self.overlay = None;
                        Some(action)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = None;
                        None
                    }
                    _ => None,
                };
            }
            Some(Overlay::AgentConfirm { preset, task, .. }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        let action = Action::StartAgent {
                            preset: preset.clone(),
                            task: task.clone(),
                        };
                        self.overlay = None;
                        self.composer.clear();
                        Some(action)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = None;
                        None
                    }
                    _ => None,
                };
            }
            Some(Overlay::AgentFollowUpConfirm { child_id, task, .. }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        let action = Action::FollowUpAgent {
                            child_id: child_id.clone(),
                            task: task.clone(),
                        };
                        self.overlay = None;
                        self.composer.clear();
                        Some(action)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = None;
                        None
                    }
                    _ => None,
                };
            }
            Some(Overlay::AgentResumeConfirm { child_id, .. }) => {
                return match key.code {
                    KeyCode::Char('y') => {
                        let action = Action::ResumeAgent {
                            child_id: child_id.clone(),
                        };
                        self.overlay = None;
                        self.composer.clear();
                        Some(action)
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = None;
                        None
                    }
                    _ => None,
                };
            }
            Some(Overlay::ExitConfirm { .. }) => return self.on_exit_confirm_key(key),
            None => {}
        }

        match (key.code, key.modifiers) {
            // At the empty idle point of action, Tab cycles only the
            // configured root-agent modes. Overlay-specific Tab behavior was
            // handled above and a non-empty draft is never changed.
            (KeyCode::Tab, _) if !self.is_busy() && self.composer.is_empty() => {
                self.cycle_agent_mode(false)
            }
            (KeyCode::BackTab, _) if !self.is_busy() && self.composer.is_empty() => {
                self.cycle_agent_mode(true)
            }
            (KeyCode::Tab | KeyCode::BackTab, _) => None,
            (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                let original = self.composer.text().to_owned();
                if !self.composer.text().starts_with('/') {
                    self.composer.replace("/");
                }
                self.overlay = Some(Overlay::Palette {
                    selected: 0,
                    error: None,
                    restore_on_escape: Some(original),
                });
                None
            }
            (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
                self.follow_newest();
                None
            }
            (KeyCode::Char('?'), KeyModifiers::NONE) if self.composer.is_empty() => {
                self.follow_newest();
                self.dispatch_command(CommandAction::Help)
            }
            (KeyCode::Esc, _) => self.on_escape(),
            (KeyCode::PageUp, _) => {
                self.scroll_up(10);
                None
            }
            (KeyCode::PageDown, _) => {
                self.scroll_down(10);
                None
            }
            (KeyCode::Up, _) if self.composer.is_empty() || self.composer.is_recalling() => {
                if self.composer.recall_previous() {
                    None
                } else {
                    self.on_scroll_key(key)
                }
            }
            (KeyCode::Down, _) if self.composer.is_recalling() => {
                self.composer.recall_next();
                None
            }
            (KeyCode::Up | KeyCode::Down | KeyCode::Home | KeyCode::End, _) => {
                self.on_scroll_key(key)
            }
            _ => self.on_composer_key(key),
        }
    }

    fn on_ctrl_c(&mut self) -> Option<Action> {
        let now = Instant::now();
        let second_press = self
            .last_ctrl_c
            .is_some_and(|previous| now.duration_since(previous) < FORCE_QUIT_WINDOW);

        if second_press {
            self.cancel_pending_prompts();
            self.should_quit = true;
            return Some(Action::Quit);
        }

        self.last_ctrl_c = Some(now);
        match self.overlay.take() {
            Some(Overlay::Palette {
                restore_on_escape, ..
            }) => {
                if let Some(original) = restore_on_escape {
                    self.composer.replace(original);
                }
            }
            Some(Overlay::ResourcePicker {
                restore_on_escape, ..
            }) => self.composer.replace(restore_on_escape),
            other => self.overlay = other,
        }
        self.composer.stash_for_recall();
        None
    }

    fn request_exit(&mut self) -> Option<Action> {
        if !self.has_live_work() {
            self.cancel_pending_prompts();
            self.should_quit = true;
            return Some(Action::Quit);
        }

        let (approval, questionnaire) = match self.overlay.take() {
            Some(Overlay::Approval { prompt, review }) => (Some((prompt, review)), None),
            Some(Overlay::Questionnaire { state }) => (None, Some(state)),
            _ => (None, None),
        };
        self.overlay = Some(Overlay::ExitConfirm {
            approval,
            questionnaire,
        });
        None
    }

    fn cancel_pending_prompts(&mut self) {
        self.overlay = match self.overlay.take() {
            Some(Overlay::Approval { prompt, .. }) => {
                prompt.cancel();
                None
            }
            Some(Overlay::Questionnaire { state }) => {
                self.resolve_questionnaire(state, QuestionnaireResolution::Cancelled);
                None
            }
            Some(Overlay::ExitConfirm {
                approval,
                questionnaire,
            }) => {
                if let Some((prompt, _)) = approval {
                    prompt.cancel();
                }
                if let Some(state) = questionnaire {
                    self.resolve_questionnaire(state, QuestionnaireResolution::Cancelled);
                }
                None
            }
            other => other,
        };
        while let Some(prompt) = self.pending_prompts.pop_front() {
            match prompt {
                PendingPrompt::Approval(prompt, _) => prompt.cancel(),
                PendingPrompt::Questionnaire(state) => {
                    self.resolve_questionnaire(state, QuestionnaireResolution::Cancelled);
                }
            }
        }
    }

    fn on_escape(&mut self) -> Option<Action> {
        if self.is_busy() {
            self.status.activity = Activity::Interrupting;
            return Some(Action::Interrupt);
        }
        if !self.composer.is_empty() {
            self.composer.clear();
        }
        None
    }

    fn on_approval_key(&mut self, key: KeyEvent) -> Option<Action> {
        // No default action: an approval modal must not be answerable by a
        // stray Enter arriving from the composer.
        match key.code {
            KeyCode::Char('y') => self.answer_approval(Some(PromptScope::Once)),
            KeyCode::Char('a') => self.answer_approval(Some(PromptScope::Session)),
            KeyCode::Char('n') | KeyCode::Esc => self.answer_approval(None),
            _ => {}
        }
        None
    }

    fn on_questionnaire_key(&mut self, key: KeyEvent) -> Option<Action> {
        let resolution = match &mut self.overlay {
            Some(Overlay::Questionnaire { state }) => state.on_key(key),
            _ => None,
        };
        if let Some(resolution) = resolution
            && let Some(Overlay::Questionnaire { state }) = self.overlay.take()
        {
            self.resolve_questionnaire(state, resolution);
            self.present_next_prompt();
        }
        None
    }

    fn on_resource_picker_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.code == KeyCode::Char('@')
            && matches!(
                self.overlay,
                Some(Overlay::ResourcePicker {
                    target: ResourceTarget::Reference,
                    ref picker,
                    ..
                }) if picker.query.is_empty()
            )
        {
            self.overlay = None;
            self.composer.insert_str("@@");
            return None;
        }
        let outcome = match &mut self.overlay {
            Some(Overlay::ResourcePicker { picker, .. }) => picker.on_key(key),
            _ => return None,
        };
        match outcome {
            PickerOutcome::Pending => None,
            PickerOutcome::Cancelled => {
                if let Some(Overlay::ResourcePicker {
                    target,
                    restore_on_escape,
                    ..
                }) = self.overlay.take()
                    && target != ResourceTarget::Reference
                {
                    self.composer.replace(restore_on_escape);
                }
                None
            }
            PickerOutcome::Selected(id) => {
                let (target, restore) = match self.overlay.take() {
                    Some(Overlay::ResourcePicker {
                        target,
                        restore_on_escape,
                        ..
                    }) => (target, restore_on_escape),
                    _ => return None,
                };
                self.apply_resource_selection(target, id, restore)
            }
        }
    }

    fn open_resource_picker(
        &mut self,
        target: ResourceTarget,
        entries: Vec<ResourceEntry>,
        empty_guidance: &str,
        restore_on_escape: String,
        initial_query: Option<&str>,
    ) {
        let title = match target {
            ResourceTarget::Model => "Choose model",
            ResourceTarget::Provider => "Choose provider",
            ResourceTarget::Profile => "Choose profile",
            ResourceTarget::Resume => "Resume session",
            ResourceTarget::Reference => "Attach file or invoke agent",
        };
        let mut picker = ResourcePicker::new(title, entries, empty_guidance);
        if let Some(query) = initial_query {
            picker.query = query.to_owned();
        }
        self.overlay = Some(Overlay::ResourcePicker {
            picker,
            target,
            restore_on_escape,
        });
    }

    fn open_target_picker(&mut self, target: ResourceTarget, restore: String) {
        let (entries, guidance) = match target {
            ResourceTarget::Model => (
                self.resources.models.clone(),
                "No local model is selectable · run smith setup add-model",
            ),
            ResourceTarget::Provider => (
                self.resources.providers.clone(),
                "No provider is selectable · run smith setup add-provider",
            ),
            ResourceTarget::Profile => (
                self.resources.profiles.clone(),
                "No profile is selectable · run smith setup",
            ),
            ResourceTarget::Resume => (
                self.resources.sessions.clone(),
                "Nothing to resume for this project · use /new",
            ),
            ResourceTarget::Reference => {
                let mut entries = self
                    .resources
                    .child_agents
                    .iter()
                    .chain(&self.resources.files)
                    .cloned()
                    .collect::<Vec<_>>();
                entries.extend(self.children.iter().map(|(child, summary)| {
                    ResourceEntry::new(
                        format!("agent:{child}"),
                        format!("@{child}"),
                        format!(
                            "existing child · {}{}",
                            summary.state,
                            summary
                                .detail
                                .as_deref()
                                .map(|detail| format!(" · {detail}"))
                                .unwrap_or_default()
                        ),
                    )
                }));
                (
                    entries,
                    "No matching file, child preset, or existing child in the bounded local index",
                )
            }
        };
        self.open_resource_picker(target, entries, guidance, restore, None);
    }

    fn apply_resource_selection(
        &mut self,
        target: ResourceTarget,
        id: String,
        restore: String,
    ) -> Option<Action> {
        match target {
            ResourceTarget::Model => self.apply_model_id(&id),
            ResourceTarget::Provider => {
                let models: Vec<ResourceEntry> = self
                    .resources
                    .models
                    .iter()
                    .filter(|entry| {
                        model_pair(&self.resources.providers, &entry.id)
                            .is_some_and(|(provider, _)| provider == id)
                    })
                    .cloned()
                    .collect();
                match models.as_slice() {
                    [] => {
                        self.transcript.push_error(format!(
                            "provider `{id}` has no selectable local model; run `smith setup add-model --provider {id}`"
                        ));
                        None
                    }
                    [only] => self.apply_model_id(&only.id),
                    _ => {
                        self.open_resource_picker(
                            ResourceTarget::Model,
                            models,
                            "This provider has no selectable model · run smith setup add-model",
                            restore,
                            None,
                        );
                        None
                    }
                }
            }
            ResourceTarget::Profile => {
                self.composer.clear();
                Some(Action::Reconfigure(PaletteCommand::Profile(id)))
            }
            ResourceTarget::Resume => {
                self.composer.clear();
                if self.resources.current_session.as_deref() == Some(id.as_str()) {
                    self.transcript
                        .push_notice("resume", "already in the selected session");
                    None
                } else {
                    Some(Action::Reconfigure(PaletteCommand::Resume(id)))
                }
            }
            ResourceTarget::Reference => {
                let selected = id
                    .strip_prefix("file:")
                    .map(|identity| ("file", identity))
                    .or_else(|| {
                        id.strip_prefix("agent:")
                            .map(|identity| ("agent", identity))
                    });
                match selected {
                    Some((kind, identity)) => {
                        let collides = self
                            .resources
                            .files
                            .iter()
                            .any(|entry| entry.id.strip_prefix("file:") == Some(identity))
                            && self
                                .resources
                                .child_agents
                                .iter()
                                .any(|entry| entry.id.strip_prefix("agent:") == Some(identity));
                        if collides {
                            self.composer.insert_str(&format!("@{kind}:{identity} "));
                        } else {
                            self.composer.insert_str(&format!("@{identity} "));
                        }
                    }
                    None => self
                        .transcript
                        .push_error("reference picker returned an invalid typed identity"),
                }
                None
            }
        }
    }

    fn cycle_agent_mode(&mut self, backwards: bool) -> Option<Action> {
        let selectable = self
            .resources
            .agent_modes
            .iter()
            .filter(|entry| entry.disabled_reason.is_none())
            .collect::<Vec<_>>();
        if selectable.len() < 2 {
            return None;
        }
        let current = selectable
            .iter()
            .position(|entry| entry.active)
            .unwrap_or(0);
        let next = if backwards {
            current.checked_sub(1).unwrap_or(selectable.len() - 1)
        } else {
            (current + 1) % selectable.len()
        };
        Some(Action::Reconfigure(PaletteCommand::Agent(
            selectable[next].id.clone(),
        )))
    }

    fn apply_model_id(&mut self, id: &str) -> Option<Action> {
        let Some((provider, model)) = model_pair(&self.resources.providers, id) else {
            self.transcript
                .push_error(format!("model choice `{id}` has no provider identity"));
            return None;
        };
        self.composer.clear();
        Some(Action::Reconfigure(PaletteCommand::Model {
            provider,
            model,
        }))
    }

    fn direct_model(&mut self, value: &str, restore: String) -> Option<Action> {
        if self
            .resources
            .models
            .iter()
            .any(|entry| entry.id == value && entry.disabled_reason.is_none())
        {
            return self.apply_model_id(value);
        }
        let mut matches: Vec<ResourceEntry> = self
            .resources
            .models
            .iter()
            .filter(|entry| {
                entry.disabled_reason.is_none()
                    && model_pair(&self.resources.providers, &entry.id)
                        .is_some_and(|(_, model)| model == value)
            })
            .cloned()
            .collect();
        if let Some(active_provider) = self.status.provider.as_deref()
            && let Some(position) = matches.iter().position(|entry| {
                model_pair(&self.resources.providers, &entry.id)
                    .is_some_and(|(provider, _)| provider == active_provider)
            })
        {
            let selected = matches.remove(position);
            return self.apply_model_id(&selected.id);
        }
        match matches.as_slice() {
            [only] => self.apply_model_id(&only.id),
            [] => {
                self.transcript.push_error(format!(
                    "model `{value}` is not locally selectable; run `smith setup add-model`"
                ));
                None
            }
            _ => {
                self.transcript.push_error(format!(
                    "model `{value}` is available from multiple providers; choose a qualified pair"
                ));
                self.open_resource_picker(
                    ResourceTarget::Model,
                    matches,
                    "No matching provider/model pair",
                    restore,
                    Some(value),
                );
                None
            }
        }
    }

    fn on_exit_confirm_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('y') => {
                self.cancel_pending_prompts();
                self.should_quit = true;
                Some(Action::Quit)
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                let (approval, questionnaire) = match self.overlay.take() {
                    Some(Overlay::ExitConfirm {
                        approval,
                        questionnaire,
                    }) => (approval, questionnaire),
                    _ => (None, None),
                };
                self.overlay = match (approval, questionnaire) {
                    (Some((prompt, review)), None) => Some(Overlay::Approval { prompt, review }),
                    (None, Some(state)) => Some(Overlay::Questionnaire { state }),
                    _ => None,
                };
                self.present_next_prompt();
                None
            }
            _ => None,
        }
    }

    fn on_palette_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => {
                if let Some(Overlay::Palette {
                    restore_on_escape, ..
                }) = self.overlay.take()
                    && let Some(original) = restore_on_escape
                {
                    self.composer.replace(original);
                }
                None
            }
            KeyCode::Backspace => {
                self.composer.backspace();
                if self.composer.is_empty() {
                    self.overlay = None;
                    return None;
                }
                if let Some(Overlay::Palette {
                    selected, error, ..
                }) = &mut self.overlay
                {
                    *selected = 0;
                    *error = None;
                }
                None
            }
            KeyCode::Tab | KeyCode::Down => {
                let count = commands::matches(self.composer.text()).len();
                if count > 0
                    && let Some(Overlay::Palette { selected, .. }) = &mut self.overlay
                {
                    *selected = (*selected + 1) % count;
                    if key.code == KeyCode::Tab {
                        let command = commands::matches(self.composer.text())[*selected];
                        self.composer.replace(commands::completion(command));
                        self.overlay = None;
                    }
                }
                None
            }
            KeyCode::BackTab | KeyCode::Up => {
                let matches = commands::matches(self.composer.text());
                if !matches.is_empty()
                    && let Some(Overlay::Palette { selected, .. }) = &mut self.overlay
                {
                    *selected = selected.checked_sub(1).unwrap_or(matches.len() - 1);
                }
                None
            }
            KeyCode::Enter => {
                if self.composer.text().starts_with("//") {
                    self.overlay = None;
                    return self.on_composer_key(key);
                }
                let matches = commands::matches(self.composer.text());
                let selected = match &self.overlay {
                    Some(Overlay::Palette { selected, .. }) => *selected,
                    _ => return None,
                };
                let input = self.composer.text().to_owned();
                match commands::parse(&input) {
                    Ok(command) => self.dispatch_command(command),
                    Err(message) => {
                        let needs_value = matches.get(selected).is_some_and(|command| {
                            matches!(command.name, "resume" | "profile" | "provider" | "model")
                        });
                        if needs_value && self.composer.text().split_whitespace().count() == 1 {
                            let command = matches[selected];
                            self.composer.replace(commands::completion(command));
                            self.overlay = None;
                            return None;
                        }
                        if let Some(Overlay::Palette { error, .. }) = &mut self.overlay {
                            *error = Some(message);
                        }
                        None
                    }
                }
            }
            KeyCode::Char(character)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                self.composer.insert(character);
                if let Some(Overlay::Palette {
                    selected, error, ..
                }) = &mut self.overlay
                {
                    *selected = 0;
                    *error = None;
                }
                None
            }
            _ => None,
        }
    }

    fn on_composer_key(&mut self, key: KeyEvent) -> Option<Action> {
        match (key.code, key.modifiers) {
            (KeyCode::Enter, m)
                if m.contains(KeyModifiers::SHIFT) || m.contains(KeyModifiers::ALT) =>
            {
                self.composer.insert('\n');
                None
            }
            (KeyCode::Enter, _) => {
                if self.composer.is_blank() {
                    return None;
                }
                let text = self.composer.text().trim().to_owned();
                // `//…` is the documented escape for a literal leading slash:
                // exactly one slash is stripped and the rest goes to the model
                // as an ordinary prompt.
                if let Some(literal) = text.strip_prefix("//") {
                    let literal = format!("/{literal}");
                    self.composer.clear();
                    self.transcript.push_user(&literal);
                    self.follow_newest();
                    return Some(Action::Send(literal));
                }
                // `!!…` is the literal escape for a provider prompt beginning
                // with `!`; one marker is removed and no local process starts.
                if let Some(literal) = text.strip_prefix("!!") {
                    let literal = format!("!{literal}");
                    self.composer.clear();
                    self.transcript.push_user(&literal);
                    self.follow_newest();
                    return Some(Action::Send(literal));
                }
                // One leading marker is an explicit local prepared shell
                // action. Keep the draft on validation failure so no command
                // disappears before it has been accepted.
                if let Some(command) = text.strip_prefix('!') {
                    let command = command.trim();
                    if command.is_empty() {
                        self.transcript
                            .push_error("a shell shortcut requires a command after `!`");
                        return None;
                    }
                    let command = command.to_owned();
                    self.composer.clear();
                    self.transcript.push_notice("shell", format!("$ {command}"));
                    self.follow_newest();
                    return Some(Action::RunShell { command });
                }
                // `/…` is a local command and never reaches the provider.
                if text.starts_with('/') {
                    self.follow_newest();
                    return match commands::parse(&text) {
                        Ok(command) => self.dispatch_command(command),
                        Err(error) => {
                            self.transcript.push_error(error);
                            None
                        }
                    };
                }
                let files = self
                    .resources
                    .files
                    .iter()
                    .filter_map(|entry| entry.id.strip_prefix("file:"))
                    .map(str::to_owned)
                    .collect();
                let agents = self
                    .resources
                    .child_agents
                    .iter()
                    .filter_map(|entry| entry.id.strip_prefix("agent:"))
                    .map(str::to_owned)
                    .chain(self.children.keys().cloned())
                    .collect();
                let parsed = match parse_references(&text, &files, &agents) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        self.transcript.push_error(error);
                        return None;
                    }
                };
                let attached_files = parsed
                    .references
                    .iter()
                    .filter_map(|reference| match reference {
                        ComposerReference::File(path) => Some(path.clone()),
                        ComposerReference::Agent(_) => None,
                    })
                    .collect::<Vec<_>>();
                let referenced_agents = parsed
                    .references
                    .iter()
                    .filter_map(|reference| match reference {
                        ComposerReference::Agent(agent) => Some(agent.clone()),
                        ComposerReference::File(_) => None,
                    })
                    .collect::<Vec<_>>();
                if let Some(agent) = referenced_agents.first() {
                    let trimmed = parsed.text.trim_start();
                    let plain = format!("@{agent}");
                    let typed = format!("@agent:{agent}");
                    let task = trimmed
                        .strip_prefix(&typed)
                        .or_else(|| trimmed.strip_prefix(&plain));
                    let Some(task) = task else {
                        self.transcript.push_error(
                            "a child preset or existing child must be the first token, for example `@review inspect the diff` or `@child-1 check that edge case`",
                        );
                        return None;
                    };
                    if referenced_agents.len() != 1 || !attached_files.is_empty() {
                        self.transcript.push_error(
                            "one explicit child preset must be submitted without file attachments",
                        );
                        return None;
                    }
                    let task = task.trim();
                    if task.is_empty() {
                        self.transcript.push_error(format!(
                            "`@{agent}` requires a bounded task after the child identity"
                        ));
                        return None;
                    }
                    if let Some(existing) = self.children.get(agent).cloned() {
                        if !matches!(
                            existing.state.as_str(),
                            "idle" | "completed" | "needs input"
                        ) {
                            self.transcript.push_error(format!(
                                "`{agent}` is {}; use `/agent {agent}` to inspect it{}",
                                existing.state,
                                if existing.state == "interrupted" {
                                    " and `/agent resume <id>` for exact continuation"
                                } else {
                                    ""
                                }
                            ));
                            return None;
                        }
                        let model = match &self.status.provider {
                            Some(provider) => format!("{provider}/{}", self.status.model),
                            None => self.status.model.clone(),
                        };
                        self.overlay = Some(Overlay::AgentFollowUpConfirm {
                            child_id: agent.clone(),
                            task: task.to_owned(),
                            content: format!(
                                "child: {agent}\noperation: new follow-up turn\ncontinuity: reuse prior child history and cumulative limits\nprovider/model: {model}\nprovider spend: yes\ncheckpoint replay: no"
                            ),
                        });
                        return None;
                    }
                    let model = match &self.status.provider {
                        Some(provider) => format!("{provider}/{}", self.status.model),
                        None => self.status.model.clone(),
                    };
                    self.overlay = Some(Overlay::AgentConfirm {
                        preset: agent.clone(),
                        task: task.to_owned(),
                        content: format!(
                            "preset: {agent}\nprovider/model: {model}\nworkspace: read-only\nturn limit: 1\nprovider spend: yes\nresult: bounded child summary"
                        ),
                    });
                    return None;
                }
                self.composer.clear();
                self.transcript.push_user(&parsed.text);
                self.follow_newest();
                if attached_files.is_empty() {
                    Some(Action::Send(parsed.text))
                } else {
                    Some(Action::SendWithFiles {
                        text: parsed.text,
                        files: attached_files,
                    })
                }
            }
            (KeyCode::Backspace, _) => {
                self.composer.backspace();
                None
            }
            (KeyCode::Delete, _) => {
                self.composer.delete();
                None
            }
            (KeyCode::Left, _) => {
                self.composer.move_left();
                None
            }
            (KeyCode::Right, _) => {
                self.composer.move_right();
                None
            }
            (KeyCode::Home, _) => {
                self.composer.move_home();
                None
            }
            (KeyCode::End, _) => {
                self.composer.move_end();
                None
            }
            (KeyCode::Char(ch), m) if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT => {
                if ch == '@' && self.composer_at_token_boundary() {
                    let restore = self.composer.text().to_owned();
                    self.open_target_picker(ResourceTarget::Reference, restore);
                    return None;
                }
                self.composer.insert(ch);
                if self.composer.text().starts_with('/') {
                    self.overlay = Some(Overlay::Palette {
                        selected: 0,
                        error: None,
                        restore_on_escape: None,
                    });
                }
                None
            }
            _ => None,
        }
    }

    fn composer_at_token_boundary(&self) -> bool {
        let cursor = self.composer.cursor();
        cursor == 0
            || self
                .composer
                .text()
                .chars()
                .nth(cursor.saturating_sub(1))
                .is_some_and(|character| {
                    character.is_whitespace()
                        || matches!(character, '(' | '[' | '{' | ',' | ';' | ':')
                })
    }

    fn dispatch_command(&mut self, command: CommandAction) -> Option<Action> {
        let Some(spec) = commands::COMMANDS.iter().find(|spec| {
            let name = match &command {
                CommandAction::Help => "help",
                CommandAction::Status => "status",
                CommandAction::Context => "context",
                CommandAction::Details => "details",
                CommandAction::Timeline => "timeline",
                CommandAction::NewSession => "new",
                CommandAction::Resume(_) => "resume",
                CommandAction::Profile(_) => "profile",
                CommandAction::Provider(_) => "provider",
                CommandAction::Model(_) => "model",
                CommandAction::Agent(_) | CommandAction::AgentResume(_) => "agent",
                CommandAction::Diff(_) => "diff",
                CommandAction::Review(_) => "review",
                CommandAction::Undo => "undo",
                CommandAction::Redo => "redo",
                CommandAction::Revert(_) => "revert",
                CommandAction::Quit => "quit",
            };
            spec.name == name
        }) else {
            unreachable!("parsed commands always have registry entries");
        };

        if spec.requires_idle && self.is_busy() {
            self.overlay = None;
            self.transcript.push_notice(
                "smith",
                format!(
                    "/{name} requires an idle turn; draft preserved",
                    name = spec.name
                ),
            );
            return None;
        }

        self.overlay = None;
        let restore = self.composer.text().to_owned();
        match command {
            CommandAction::Help => {
                self.composer.clear();
                self.show_local_result("help", commands::help());
                None
            }
            CommandAction::Details => {
                self.composer.clear();
                self.toggle_work_details();
                self.transcript.push_notice(
                    "details",
                    if self.work_details {
                        "bounded tool details shown"
                    } else {
                        "bounded tool details hidden"
                    },
                );
                None
            }
            CommandAction::Quit => {
                self.composer.clear();
                self.request_exit()
            }
            CommandAction::NewSession => {
                self.composer.clear();
                Some(Action::Reconfigure(PaletteCommand::NewSession))
            }
            CommandAction::Resume(None) => {
                self.composer.clear();
                self.open_target_picker(ResourceTarget::Resume, restore);
                None
            }
            CommandAction::Resume(Some(id)) => {
                self.composer.clear();
                let selectable = self
                    .resources
                    .sessions
                    .iter()
                    .any(|entry| entry.id == id && entry.disabled_reason.is_none());
                if selectable {
                    self.apply_resource_selection(ResourceTarget::Resume, id, restore)
                } else {
                    self.transcript.push_error(format!(
                        "session `{id}` is not available for this project; use `/resume` to choose"
                    ));
                    None
                }
            }
            CommandAction::Profile(None) => {
                self.composer.clear();
                self.open_target_picker(ResourceTarget::Profile, restore);
                None
            }
            CommandAction::Profile(Some(name)) => {
                self.composer.clear();
                let selectable = self
                    .resources
                    .profiles
                    .iter()
                    .any(|entry| entry.id == name && entry.disabled_reason.is_none());
                if selectable {
                    self.apply_resource_selection(ResourceTarget::Profile, name, restore)
                } else {
                    self.transcript.push_error(format!(
                        "profile `{name}` is not locally selectable; use `/profile` to choose"
                    ));
                    None
                }
            }
            CommandAction::Provider(None) => {
                self.composer.clear();
                self.open_target_picker(ResourceTarget::Provider, restore);
                None
            }
            CommandAction::Provider(Some(name)) => {
                self.composer.clear();
                let selectable = self
                    .resources
                    .providers
                    .iter()
                    .any(|entry| entry.id == name && entry.disabled_reason.is_none());
                if selectable {
                    self.apply_resource_selection(ResourceTarget::Provider, name, restore)
                } else {
                    self.transcript.push_error(format!(
                        "provider `{name}` is not locally selectable; run `smith setup add-provider`"
                    ));
                    None
                }
            }
            CommandAction::Model(None) => {
                self.composer.clear();
                self.open_target_picker(ResourceTarget::Model, restore);
                None
            }
            CommandAction::Model(Some(name)) => {
                self.composer.clear();
                self.direct_model(&name, restore)
            }
            CommandAction::AgentResume(child_id) => {
                if self.is_busy() {
                    self.overlay = None;
                    self.transcript.push_notice(
                        "agent",
                        "exact child resume requires an idle root turn; draft preserved",
                    );
                    return None;
                }
                self.composer.clear();
                let Some(summary) = self.children.get(&child_id) else {
                    self.transcript.push_error(format!(
                        "No child named `{child_id}`; use `/agent` to list retained children."
                    ));
                    return None;
                };
                let resumable = summary.state == "interrupted"
                    && summary.detail.as_deref().is_some_and(|detail| {
                        detail.contains("resumable") || detail.contains("exact resume available")
                    });
                if !resumable {
                    self.transcript.push_error(format!(
                        "`{child_id}` has no compatible interrupted checkpoint; inspect it with `/agent {child_id}`"
                    ));
                    return None;
                }
                self.overlay = Some(Overlay::AgentResumeConfirm {
                    child_id: child_id.clone(),
                    content: format!(
                        "child: {child_id}\noperation: continue exact interrupted checkpoint\nnew task: no\nturn slot consumed: no\nprovider spend: may continue\nside effects: committed work is not replayed"
                    ),
                });
                None
            }
            local => {
                self.composer.clear();
                Some(Action::Command(local))
            }
        }
    }

    fn on_scroll_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(1),
            KeyCode::Home => self.scroll_up(u16::MAX),
            KeyCode::End => self.follow_newest(),
            _ => {}
        }
        None
    }

    /// Scrolls up, which pauses following.
    pub fn scroll_up(&mut self, lines: u16) {
        if self.scroll_limit == 0 || lines == 0 {
            return;
        }
        self.scroll_back = self
            .scroll_back
            .saturating_add(lines)
            .min(self.scroll_limit);
        self.following = self.scroll_back == 0;
    }

    /// Scrolls down, resuming following at the bottom.
    pub fn scroll_down(&mut self, lines: u16) {
        self.scroll_back = self
            .scroll_back
            .min(self.scroll_limit)
            .saturating_sub(lines);
        if self.scroll_back == 0 {
            self.following = true;
        }
    }

    /// Jumps to newest output and resumes following.
    pub fn follow_newest(&mut self) {
        self.scroll_back = 0;
        self.following = true;
    }

    /// Synchronizes scroll state with the viewport computed by the renderer.
    ///
    /// Keeping the visible offset stable while paused prevents streaming output
    /// or a resize from pulling the reader toward the newest content.
    pub(crate) fn sync_scroll_limit(&mut self, limit: u16) {
        if self.following {
            self.scroll_limit = limit;
            self.scroll_back = 0;
            return;
        }

        let visible_offset = self
            .scroll_limit
            .saturating_sub(self.scroll_back.min(self.scroll_limit));
        self.scroll_limit = limit;
        if limit == 0 {
            self.follow_newest();
            return;
        }

        self.scroll_back = limit.saturating_sub(visible_offset.min(limit));
        if self.scroll_back == 0 {
            self.following = true;
        }
    }

    /// Appends bounded informational command output to the transcript.
    pub fn show_local_result(&mut self, title: impl Into<String>, content: impl Into<String>) {
        self.follow_newest();
        self.transcript
            .push_local_result(title, content, LocalResultState::Info);
    }

    /// Appends an explicit empty informational result to the transcript.
    pub fn show_local_empty(&mut self, title: impl Into<String>, content: impl Into<String>) {
        self.follow_newest();
        self.transcript
            .push_local_result(title, content, LocalResultState::Empty);
    }

    /// Appends a titled local command failure to the transcript.
    pub fn show_local_error(&mut self, title: impl Into<String>, content: impl Into<String>) {
        self.follow_newest();
        self.transcript
            .push_local_result(title, content, LocalResultState::Error);
    }

    /// Shows an exact undo preview with no default action.
    pub fn confirm_undo(&mut self, content: impl Into<String>) {
        self.overlay = Some(Overlay::UndoConfirm {
            content: content.into(),
        });
    }

    /// Shows an exact redo preview with no default action.
    pub fn confirm_redo(&mut self, content: impl Into<String>) {
        self.overlay = Some(Overlay::RedoConfirm {
            content: content.into(),
        });
    }

    /// Shows an exact selective-revert preview with no default action.
    pub fn confirm_revert(
        &mut self,
        scope: impl Into<String>,
        fingerprint: impl Into<String>,
        content: impl Into<String>,
    ) {
        self.overlay = Some(Overlay::RevertConfirm {
            scope: scope.into(),
            fingerprint: fingerprint.into(),
            content: content.into(),
        });
    }

    /// Shows review scope and provider spend before dispatch.
    pub fn confirm_review(&mut self, scope: impl Into<String>, content: impl Into<String>) {
        self.overlay = Some(Overlay::ReviewConfirm {
            scope: scope.into(),
            content: content.into(),
        });
    }
}

/// Splits a provider-qualified model ID without assuming provider names cannot
/// themselves contain `/`. The local provider inventory is authoritative; the
/// first-slash fallback only keeps synthetic test inventories useful.
fn model_pair(providers: &[ResourceEntry], id: &str) -> Option<(String, String)> {
    if let Some(provider) = providers
        .iter()
        .filter(|entry| {
            id.strip_prefix(&entry.id)
                .is_some_and(|suffix| suffix.starts_with('/') && suffix.len() > 1)
        })
        .max_by_key(|entry| entry.id.len())
    {
        let model = id.strip_prefix(&provider.id)?.strip_prefix('/')?;
        return Some((provider.id.clone(), model.to_owned()));
    }
    let (provider, model) = id.split_once('/')?;
    (!provider.is_empty() && !model.is_empty()).then(|| (provider.to_owned(), model.to_owned()))
}

/// Finished copy for a child's declared workspace posture.
fn describe_workspace(workspace: &agent_runtime_core::delegation::WorkspacePolicy) -> String {
    use agent_runtime_core::delegation::WorkspacePolicy;
    match workspace {
        WorkspacePolicy::SharedProject => "shared project workspace".to_owned(),
        WorkspacePolicy::ExplicitDirectory { path } => format!("workspace {path}"),
        WorkspacePolicy::IsolatedWorktree => "isolated worktree".to_owned(),
        WorkspacePolicy::ReadOnlyView => "read-only".to_owned(),
    }
}

/// Finished copy for why a child stopped.
fn describe_cancel_reason(reason: &agent_runtime_core::cancel::CancelReason) -> String {
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
fn describe_interaction_sensitivity(
    sensitivity: &agent_runtime_core::interaction::InteractionSensitivity,
) -> &'static str {
    use agent_runtime_core::interaction::InteractionSensitivity;
    match sensitivity {
        InteractionSensitivity::Public => "public",
        InteractionSensitivity::Sensitive => "sensitive",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::approval::{
        ApprovalDecision, ApprovalOrigin, ApprovalPolicy, ApprovalRequest,
    };
    use agent_runtime_core::cancel::CancelReason;
    use agent_runtime_core::clock::{Deadline, Timestamp};
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::error::RuntimeError;
    use agent_runtime_core::event::EstimationConfidence;
    use agent_runtime_core::ids::{
        AttemptId, ChildId, EventId, InteractionRequestId, QuestionId, RequestId, SessionId,
        ToolCallId, TurnId,
    };
    use agent_runtime_core::interaction::InteractionSensitivity;
    use agent_runtime_core::manifest::{ActivatedCapability, SegmentKind};
    use agent_runtime_core::provider::ModelId;
    use agent_runtime_core::tool::{PreparedToolCall, ToolCallDisplay, ToolEffects};
    use agent_runtime_core::usage::{
        CounterKind, Provenance, UsageDelta, UsageRecord, UsageSource,
    };
    use smith_host::approval::InteractiveApproval;

    use crate::questionnaire::{QuestionnaireChoice, QuestionnaireQuestion};
    use crate::transcript::Block;

    fn fingerprint(seed: &str) -> agent_runtime_registry::Fingerprint {
        agent_runtime_registry::Fingerprint::of(seed)
    }

    fn app() -> App {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.set_resources(RuntimeResources {
            models: vec![ResourceEntry::new(
                "local/model-2",
                "local/model-2",
                "configured limits",
            )],
            providers: vec![ResourceEntry::new(
                "local",
                "local",
                "openai-compatible · 1 model",
            )],
            profiles: vec![ResourceEntry::new("work", "work", "local/model-2")],
            sessions: vec![ResourceEntry::new(
                "session-7",
                "session-7 · recent work",
                "2 turns · local/model-2",
            )],
            current_session: Some("current-session".into()),
            ..RuntimeResources::default()
        });
        app
    }

    fn event(payload: RuntimeEvent) -> EventEnvelope {
        event_at(Timestamp::ZERO, payload)
    }

    fn event_at(timestamp: Timestamp, payload: RuntimeEvent) -> EventEnvelope {
        EventEnvelope::new(
            0,
            EventId::new("e"),
            SessionId::new("s"),
            None,
            timestamp,
            payload,
        )
    }

    fn text_delta(text: &str) -> RuntimeEvent {
        RuntimeEvent::TextDelta {
            request: RequestId::new("request-fixture"),
            attempt: AttemptId::new("attempt-fixture"),
            text: text.to_owned(),
        }
    }

    fn commit_output() -> RuntimeEvent {
        RuntimeEvent::ProviderAttemptOutputCommitted {
            request: RequestId::new("request-fixture"),
            attempt: AttemptId::new("attempt-fixture"),
        }
    }

    fn discard_output() -> RuntimeEvent {
        RuntimeEvent::ProviderAttemptOutputDiscarded {
            request: RequestId::new("request-fixture"),
            attempt: AttemptId::new("attempt-fixture"),
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    fn questionnaire_form(request_id: &str, deadline: Deadline) -> QuestionnaireForm {
        QuestionnaireForm::new(
            request_id,
            vec![QuestionnaireQuestion::new(
                "direction",
                "Direction",
                "Which direction should Smith take?",
                vec![
                    QuestionnaireChoice::new("minimal", "Minimal"),
                    QuestionnaireChoice::new("complete", "Complete"),
                ],
            )],
            deadline,
        )
        .expect("valid questionnaire fixture")
    }

    fn type_text(app: &mut App, text: &str) {
        for ch in text.chars() {
            app.on_key(key(KeyCode::Char(ch)));
        }
    }

    async fn prompt(tool: &str) -> ApprovalPrompt {
        prompt_with(tool, serde_json::json!({"command": "rm -rf build"})).await
    }

    async fn prompt_with(tool: &str, arguments: serde_json::Value) -> ApprovalPrompt {
        pending_prompt_with(tool, arguments).await.0
    }

    async fn pending_prompt_with(
        tool: &str,
        arguments: serde_json::Value,
    ) -> (ApprovalPrompt, tokio::task::JoinHandle<ApprovalDecision>) {
        pending_prompt_with_deadline(tool, arguments, Deadline::never()).await
    }

    async fn pending_prompt_with_deadline(
        tool: &str,
        arguments: serde_json::Value,
        deadline: Deadline,
    ) -> (ApprovalPrompt, tokio::task::JoinHandle<ApprovalDecision>) {
        // The simplest way to obtain a real prompt is to drive the policy the
        // runtime would call.
        let (policy, mut requests) = InteractiveApproval::new(1);
        let tool = tool.to_owned();
        let decision = tokio::spawn(async move {
            let effects = ToolEffects::read_only().with_write("/repo");
            let (permissions, resource) = effects.authorization_request(&tool, "/repo");
            let request = ApprovalRequest::new(
                PreparedToolCall::new(
                    ToolCallId::new("c1"),
                    &tool,
                    arguments,
                    permissions,
                    resource,
                    effects,
                    ToolCallDisplay::new(format!("Run {tool}")),
                ),
                deadline,
                ApprovalOrigin::new(
                    agent_runtime_core::ids::SessionId::new("session-1"),
                    RequestId::new("request-1"),
                ),
            );
            policy.decide(&request).await
        });
        let prompt = requests.recv().await.expect("an approval prompt");
        (prompt, decision)
    }

    #[test]
    fn sending_records_the_message_and_clears_the_composer() {
        let mut app = app();
        type_text(&mut app, "run the tests");
        let action = app.on_key(key(KeyCode::Enter));

        assert_eq!(action, Some(Action::Send("run the tests".into())));
        assert!(app.composer.is_empty());
        assert_eq!(
            app.transcript.blocks()[0],
            Block::User {
                text: "run the tests".into()
            }
        );
    }

    #[test]
    fn a_known_slash_command_dispatches_locally_without_a_send() {
        let mut app = app();
        type_text(&mut app, "/model model-2");
        let action = app.on_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            Some(Action::Reconfigure(PaletteCommand::Model {
                provider: "local".into(),
                model: "model-2".into(),
            })),
            "a slash command runs the same host action as the palette"
        );
        assert!(
            !app.transcript
                .blocks()
                .iter()
                .any(|block| matches!(block, Block::User { .. })),
            "an intercepted command must not become a user turn"
        );
    }

    #[test]
    fn an_unknown_slash_command_fails_locally_and_names_help() {
        let mut app = app();
        type_text(&mut app, "/frobnicate");
        let action = app.on_key(key(KeyCode::Enter));
        assert_eq!(action, None, "no provider request may result");
        let error = match &app.overlay {
            Some(Overlay::Palette {
                error: Some(error), ..
            }) => error,
            other => panic!("expected a local command error, got {other:?}"),
        };
        assert!(error.contains("/help"), "{error}");
    }

    #[test]
    fn slash_help_lists_every_command_locally() {
        let mut app = app();
        type_text(&mut app, "/help");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        let help = app
            .transcript
            .blocks()
            .iter()
            .find_map(|block| match block {
                Block::LocalResult { title, content, .. } if title == "help" => {
                    Some(content.clone())
                }
                _ => None,
            })
            .expect("an inline help result");
        for command in [
            "/help",
            "/status",
            "/context",
            "/new",
            "/resume",
            "/profile",
            "/provider",
            "/model",
            "/agent",
            "/diff",
            "/review",
            "/undo",
            "/revert",
            "/quit",
        ] {
            assert!(help.contains(command), "help must list {command}");
        }
    }

    #[test]
    fn a_double_slash_escapes_to_a_literal_prompt() {
        let mut app = app();
        type_text(&mut app, "//help me understand slashes");
        let action = app.on_key(key(KeyCode::Enter));
        assert_eq!(
            action,
            Some(Action::Send("/help me understand slashes".into())),
            "the escape sends the literal message to the model"
        );
        assert_eq!(
            app.transcript.blocks()[0],
            Block::User {
                text: "/help me understand slashes".into()
            }
        );
    }

    #[test]
    fn slash_quit_follows_the_exit_policy() {
        let mut idle = app();
        type_text(&mut idle, "/quit");
        assert_eq!(idle.on_key(key(KeyCode::Enter)), Some(Action::Quit));

        let mut busy = app();
        busy.apply(&event(RuntimeEvent::TurnStarted));
        type_text(&mut busy, "/quit");
        assert_eq!(busy.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(busy.overlay, Some(Overlay::ExitConfirm { .. })));
    }

    #[test]
    fn a_blank_composer_sends_nothing() {
        let mut app = app();
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        type_text(&mut app, "   ");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(app.transcript.is_empty());
    }

    #[test]
    fn shift_enter_inserts_a_newline_instead_of_sending() {
        let mut app = app();
        type_text(&mut app, "line one");
        let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(action, None);
        assert_eq!(app.composer.text(), "line one\n");
    }

    #[test]
    fn escape_interrupts_a_running_turn_and_otherwise_clears_input() {
        let mut app = app();
        type_text(&mut app, "draft");
        assert_eq!(app.on_key(key(KeyCode::Esc)), None);
        assert!(app.composer.is_empty());

        app.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(app.on_key(key(KeyCode::Esc)), Some(Action::Interrupt));
        assert_eq!(app.status.activity, Activity::Interrupting);
    }

    #[test]
    fn first_ctrl_c_clears_and_stashes_the_draft_without_quitting() {
        let mut idle = app();
        type_text(&mut idle, "recover this draft");
        assert_eq!(idle.on_key(ctrl('c')), None);
        assert!(idle.composer.is_empty());
        assert!(!idle.should_quit);

        assert_eq!(idle.on_key(key(KeyCode::Up)), None);
        assert_eq!(idle.composer.text(), "recover this draft");
        assert!(!idle.should_quit);

        let mut busy = app();
        busy.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(busy.on_key(ctrl('c')), None);
        assert!(busy.overlay.is_none());
        assert!(!busy.should_quit);
        assert!(busy.is_busy());
    }

    #[test]
    fn a_non_ctrl_c_key_disarms_the_double_press_exit() {
        let mut app = app();
        assert_eq!(app.on_key(ctrl('c')), None);
        assert_eq!(app.on_key(key(KeyCode::Char('x'))), None);
        assert_eq!(app.on_key(ctrl('c')), None);
        assert!(!app.should_quit);
        assert!(app.composer.is_empty());
    }

    #[test]
    fn the_ctrl_c_exit_hint_expires_at_the_double_press_boundary() {
        let mut app = app();
        let pressed = Instant::now();
        app.last_ctrl_c = Some(pressed);

        assert!(app.ctrl_c_exit_hint_active());
        assert!(
            !app.expire_ctrl_c_exit_hint_at(pressed + FORCE_QUIT_WINDOW - Duration::from_millis(1))
        );
        assert!(app.expire_ctrl_c_exit_hint_at(pressed + FORCE_QUIT_WINDOW));
        assert!(!app.ctrl_c_exit_hint_active());
        assert!(app.last_ctrl_c.is_none());
    }

    #[test]
    fn a_second_ctrl_c_exits_when_idle_or_busy() {
        let mut idle = app();
        assert_eq!(idle.on_key(ctrl('c')), None);
        assert_eq!(idle.on_key(ctrl('c')), Some(Action::Quit));
        assert!(idle.should_quit);

        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(app.on_key(ctrl('c')), None);
        assert_eq!(app.on_key(ctrl('c')), Some(Action::Quit));
        assert!(app.should_quit);
    }

    #[test]
    fn question_mark_opens_the_same_local_help_without_a_provider_send() {
        let mut app = app();
        assert_eq!(app.on_key(key(KeyCode::Char('?'))), None);
        assert!(app.composer.is_empty());
        let help = app
            .transcript
            .blocks()
            .iter()
            .find_map(|block| match block {
                Block::LocalResult { title, content, .. } if title == "help" => Some(content),
                _ => None,
            })
            .expect("question mark should render local help");
        assert!(help.contains("Ctrl+C twice"), "{help}");
        assert!(help.contains("Up recalls"), "{help}");
    }

    #[test]
    fn the_palette_emits_typed_safe_boundary_commands() {
        let cases = [
            ("new", PaletteCommand::NewSession),
            (
                "resume session-7",
                PaletteCommand::Resume("session-7".into()),
            ),
            ("profile work", PaletteCommand::Profile("work".into())),
            (
                "provider local",
                PaletteCommand::Model {
                    provider: "local".into(),
                    model: "model-2".into(),
                },
            ),
            (
                "model model-2",
                PaletteCommand::Model {
                    provider: "local".into(),
                    model: "model-2".into(),
                },
            ),
        ];
        for (input, expected) in cases {
            let mut app = app();
            assert_eq!(app.on_key(ctrl('p')), None);
            assert!(matches!(app.overlay, Some(Overlay::Palette { .. })));
            type_text(&mut app, input);
            assert_eq!(
                app.on_key(key(KeyCode::Enter)),
                Some(Action::Reconfigure(expected))
            );
            assert!(app.overlay.is_none());
        }
    }

    #[test]
    fn a_selector_without_a_value_opens_a_local_picker_and_escape_restores_the_draft() {
        let mut app = app();
        app.on_key(ctrl('p'));
        type_text(&mut app, "resume");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(app.composer.is_empty());
        assert!(matches!(
            app.overlay,
            Some(Overlay::ResourcePicker {
                target: ResourceTarget::Resume,
                ..
            })
        ));
        assert_eq!(app.on_key(key(KeyCode::Esc)), None);
        assert_eq!(app.composer.text(), "/resume");
        assert!(app.overlay.is_none());
    }

    #[test]
    fn model_picker_applies_a_cross_provider_pair_atomically() {
        let mut app = app();
        app.resources.providers.push(ResourceEntry::new(
            "openrouter",
            "openrouter",
            "openai-compatible · 1 model",
        ));
        app.resources.models.push(ResourceEntry::new(
            "openrouter/openai/gpt-4o-mini",
            "openrouter/openai/gpt-4o-mini",
            "configured limits",
        ));
        type_text(&mut app, "/model");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::ResourcePicker {
                target: ResourceTarget::Model,
                ..
            })
        ));
        app.on_key(key(KeyCode::Down));
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(PaletteCommand::Model {
                provider: "openrouter".into(),
                model: "openai/gpt-4o-mini".into(),
            }))
        );
    }

    #[test]
    fn provider_with_several_models_cascades_to_a_scoped_model_picker() {
        let mut app = app();
        app.resources.providers.push(ResourceEntry::new(
            "router",
            "router",
            "openai-compatible · 2 models",
        ));
        app.resources.models.extend([
            ResourceEntry::new("router/alpha", "router/alpha", "configured limits"),
            ResourceEntry::new("router/beta", "router/beta", "configured limits"),
        ]);
        type_text(&mut app, "/provider router");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        let picker = match &app.overlay {
            Some(Overlay::ResourcePicker {
                picker,
                target: ResourceTarget::Model,
                ..
            }) => picker,
            other => panic!("expected a model cascade, got {other:?}"),
        };
        assert_eq!(picker.entries.len(), 2);
        assert!(
            picker
                .entries
                .iter()
                .all(|entry| entry.id.starts_with("router/"))
        );
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(PaletteCommand::Model {
                provider: "router".into(),
                model: "alpha".into(),
            }))
        );
    }

    #[test]
    fn ambiguous_unqualified_model_opens_qualified_choices_without_applying_one() {
        let mut app = app();
        app.resources.providers.extend([
            ResourceEntry::new("a", "a", "one model"),
            ResourceEntry::new("b", "b", "one model"),
        ]);
        app.resources.models.extend([
            ResourceEntry::new("a/shared", "a/shared", "configured limits"),
            ResourceEntry::new("b/shared", "b/shared", "configured limits"),
        ]);
        type_text(&mut app, "/model shared");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        let picker = match &app.overlay {
            Some(Overlay::ResourcePicker {
                picker,
                target: ResourceTarget::Model,
                ..
            }) => picker,
            other => panic!("expected qualified choices, got {other:?}"),
        };
        assert_eq!(picker.filtered_indices().len(), 2);
        assert!(
            app.transcript.blocks().iter().any(
                |block| matches!(block, Block::Error { message } if message.contains("multiple providers"))
            )
        );
    }

    #[test]
    fn empty_model_picker_is_non_effectful_and_points_to_setup() {
        let mut app = app();
        app.resources.models.clear();
        type_text(&mut app, "/model");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        let picker = match &app.overlay {
            Some(Overlay::ResourcePicker { picker, .. }) => picker,
            other => panic!("expected an empty picker, got {other:?}"),
        };
        assert!(picker.entries.is_empty());
        assert!(picker.empty_guidance.contains("smith setup add-model"));
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
    }

    #[test]
    fn a_busy_turn_can_discover_but_cannot_run_an_idle_command() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(app.on_key(ctrl('p')), None);
        assert!(matches!(app.overlay, Some(Overlay::Palette { .. })));
        type_text(&mut app, "model next");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(app.composer.text(), "/model next");
        assert!(app.overlay.is_none());
        assert!(
            app.transcript.blocks().iter().any(|block| {
                matches!(
                    block, Block::Notice { text, .. }
                    if text.contains("requires an idle turn") && text.contains("draft preserved")
                )
            }),
            "the rejected switch was invisible"
        );
    }

    #[tokio::test]
    async fn cancelling_an_explicit_exit_restores_the_pending_approval() {
        let mut app = app();
        app.present_approval(prompt("shell").await);

        assert_eq!(app.request_exit(), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::ExitConfirm {
                approval: Some(_),
                ..
            })
        ));
        assert_eq!(app.on_key(key(KeyCode::Char('n'))), None);
        assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
    }

    #[tokio::test]
    async fn approval_is_never_granted_by_enter() {
        let mut app = app();
        app.present_approval(prompt("shell").await);

        // Enter, Tab, and an ordinary character must all leave the modal open.
        for code in [KeyCode::Enter, KeyCode::Tab, KeyCode::Char('x')] {
            assert_eq!(app.on_key(key(code)), None);
            assert!(
                matches!(app.overlay, Some(Overlay::Approval { .. })),
                "{code:?} must not answer an approval"
            );
        }

        app.on_key(key(KeyCode::Char('y')));
        assert!(app.overlay.is_none());
    }

    #[tokio::test]
    async fn denying_marks_the_tool_row_denied() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("c1"),
            name: "shell".into(),
            argument_keys: vec!["command".into()],
            argument_fingerprint: fingerprint("arguments"),
            arguments: None,
        }));
        app.present_approval(prompt("shell").await);
        app.on_key(key(KeyCode::Char('n')));

        match &app.transcript.blocks()[0] {
            Block::Tool {
                status,
                display,
                protected_summary,
                ..
            } => {
                assert_eq!(*status, ToolStatus::Denied);
                assert!(display.is_none());
                assert_eq!(protected_summary, "command · details unavailable");
            }
            other => panic!("expected a tool block, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn parallel_approval_requests_are_presented_in_fifo_order() {
        let mut app = app();
        let (shell, shell_decision) =
            pending_prompt_with("shell", serde_json::json!({"command": "build"})).await;
        let (patch, patch_decision) =
            pending_prompt_with("patch", serde_json::json!({"path": "src/lib.rs"})).await;
        app.present_approval(shell);
        app.present_approval(patch);

        match &app.overlay {
            Some(Overlay::Approval { prompt, .. }) => assert_eq!(prompt.tool(), "shell"),
            other => panic!("expected the first approval, got {other:?}"),
        }
        assert_eq!(app.pending_approval_count(), 2);

        app.on_key(key(KeyCode::Char('n')));
        match &app.overlay {
            Some(Overlay::Approval { prompt, .. }) => assert_eq!(prompt.tool(), "patch"),
            other => panic!("expected the queued approval, got {other:?}"),
        }
        assert_eq!(app.pending_approval_count(), 1);

        app.on_key(key(KeyCode::Char('y')));
        assert!(app.overlay.is_none());
        assert_eq!(app.pending_approval_count(), 0);
        assert!(matches!(
            shell_decision.await.expect("shell decision"),
            ApprovalDecision::Deny { .. }
        ));
        assert_eq!(
            patch_decision.await.expect("patch decision"),
            ApprovalDecision::Allow
        );
    }

    #[test]
    fn questionnaire_requires_explicit_submit_and_resolves_once() {
        let mut app = app();
        app.present_questionnaire(questionnaire_form("question-1", Deadline::never()));

        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(
            app.take_questionnaire_resolution().is_none(),
            "Enter on an answer stages it; it must not submit the form"
        );
        assert!(matches!(app.overlay, Some(Overlay::Questionnaire { .. })));

        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        let (request_id, resolution) = app
            .take_questionnaire_resolution()
            .expect("explicit Submit resolves the request");
        assert_eq!(request_id, "question-1");
        assert!(matches!(resolution, QuestionnaireResolution::Submitted(_)));
        assert!(app.take_questionnaire_resolution().is_none());
        assert!(app.overlay.is_none());
    }

    #[tokio::test]
    async fn questionnaire_and_approval_prompts_share_one_fifo() {
        let mut approval_first = app();
        let (approval, decision) =
            pending_prompt_with("shell", serde_json::json!({"command": "build"})).await;
        approval_first.present_approval(approval);
        approval_first
            .present_questionnaire(questionnaire_form("queued-question", Deadline::never()));
        approval_first.on_key(key(KeyCode::Char('n')));
        assert!(matches!(
            approval_first.overlay,
            Some(Overlay::Questionnaire { .. })
        ));
        assert!(matches!(
            decision.await.expect("approval decision"),
            ApprovalDecision::Deny { .. }
        ));

        let mut question_first = app();
        let (approval, decision) =
            pending_prompt_with("edit", serde_json::json!({"path": "src/lib.rs"})).await;
        question_first
            .present_questionnaire(questionnaire_form("first-question", Deadline::never()));
        question_first.present_approval(approval);
        question_first.on_key(key(KeyCode::Tab));
        question_first.on_key(key(KeyCode::Tab));
        question_first.on_key(key(KeyCode::Enter));
        assert!(matches!(
            question_first.overlay,
            Some(Overlay::Approval { .. })
        ));
        assert!(matches!(
            question_first.take_questionnaire_resolution(),
            Some((
                request_id,
                QuestionnaireResolution::Declined
            )) if request_id == "first-question"
        ));
        question_first.on_key(key(KeyCode::Char('n')));
        assert!(matches!(
            decision.await.expect("approval decision"),
            ApprovalDecision::Deny { .. }
        ));
    }

    #[test]
    fn forced_exit_cancels_visible_and_queued_questionnaires_exactly_once() {
        let mut app = app();
        app.present_questionnaire(questionnaire_form("visible", Deadline::never()));
        app.present_questionnaire(questionnaire_form("queued", Deadline::never()).restored(true));

        assert_eq!(app.on_key(ctrl('c')), None);
        assert!(matches!(app.overlay, Some(Overlay::Questionnaire { .. })));
        assert_eq!(app.on_key(ctrl('c')), Some(Action::Quit));

        for expected in ["visible", "queued"] {
            assert!(matches!(
                app.take_questionnaire_resolution(),
                Some((request_id, QuestionnaireResolution::Cancelled))
                    if request_id == expected
            ));
        }
        assert!(app.take_questionnaire_resolution().is_none());
        assert!(app.overlay.is_none());
    }

    #[tokio::test]
    async fn questionnaire_deadlines_remove_queued_requests_without_answering_them() {
        let mut app = app();
        app.present_questionnaire(questionnaire_form("visible", Deadline::never()));
        app.present_questionnaire(questionnaire_form(
            "expired",
            Deadline::after(&SystemClock, 1),
        ));
        tokio::time::sleep(Duration::from_millis(5)).await;

        app.tick();

        assert!(matches!(
            app.take_questionnaire_resolution(),
            Some((request_id, QuestionnaireResolution::TimedOut))
                if request_id == "expired"
        ));
        assert!(matches!(app.overlay, Some(Overlay::Questionnaire { .. })));
        app.on_key(key(KeyCode::Esc));
        assert!(matches!(
            app.take_questionnaire_resolution(),
            Some((request_id, QuestionnaireResolution::Cancelled))
                if request_id == "visible"
        ));
        assert!(app.take_questionnaire_resolution().is_none());
    }

    #[test]
    fn session_shutdown_cancels_every_questionnaire_responder_once() {
        let mut app = app();
        app.present_questionnaire(questionnaire_form("visible", Deadline::never()));
        app.present_questionnaire(questionnaire_form("queued", Deadline::never()));

        app.apply(&event(RuntimeEvent::SessionShutdown));

        for expected in ["visible", "queued"] {
            assert!(matches!(
                app.take_questionnaire_resolution(),
                Some((request_id, QuestionnaireResolution::Cancelled))
                    if request_id == expected
            ));
        }
        assert!(app.take_questionnaire_resolution().is_none());
        assert!(app.overlay.is_none());
    }

    #[test]
    fn runtime_close_removes_visible_or_queued_questionnaires_idempotently() {
        let mut app = app();
        app.present_questionnaire(questionnaire_form("visible", Deadline::never()));
        app.present_questionnaire(questionnaire_form("queued", Deadline::never()));

        app.dismiss_questionnaire("visible");
        assert!(matches!(
            &app.overlay,
            Some(Overlay::Questionnaire { state })
                if state.form().request_id == "queued"
        ));
        app.dismiss_questionnaire("visible");
        assert!(app.take_questionnaire_resolution().is_none());

        app.dismiss_questionnaire("queued");
        app.dismiss_questionnaire("queued");
        assert!(app.overlay.is_none());
        assert_eq!(app.pending_questionnaire_count(), 0);
        assert!(app.take_questionnaire_resolution().is_none());
    }

    #[tokio::test]
    async fn terminal_exit_cancels_visible_and_queued_approval_responders() {
        let mut app = app();
        let (shell, shell_decision) =
            pending_prompt_with("shell", serde_json::json!({"command": "build"})).await;
        let (patch, patch_decision) =
            pending_prompt_with("patch", serde_json::json!({"path": "src/lib.rs"})).await;
        app.present_approval(shell);
        app.present_approval(patch);

        assert_eq!(app.on_key(ctrl('c')), None);
        assert_eq!(app.on_key(ctrl('c')), Some(Action::Quit));

        assert_eq!(
            shell_decision.await.expect("shell cancellation"),
            ApprovalDecision::Cancelled
        );
        assert_eq!(
            patch_decision.await.expect("patch cancellation"),
            ApprovalDecision::Cancelled
        );
    }

    #[tokio::test]
    async fn approval_deadlines_close_the_prompt_without_selecting_a_default() {
        let mut app = app();
        let (prompt, decision) = pending_prompt_with_deadline(
            "shell",
            serde_json::json!({"command": "build"}),
            Deadline::after(&SystemClock, 1),
        )
        .await;
        app.present_approval(prompt);
        tokio::time::sleep(Duration::from_millis(5)).await;

        app.tick();

        assert!(app.overlay.is_none());
        assert_eq!(
            decision.await.expect("timeout decision"),
            ApprovalDecision::TimedOut
        );
        assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Notice { source, text }
                if source == "approval" && text.contains("timed out")
        )));
    }

    #[tokio::test]
    async fn a_prompt_delivered_before_turn_started_is_not_mistaken_for_stale_work() {
        let mut app = app();
        let (prompt, decision) =
            pending_prompt_with("shell", serde_json::json!({"command": "build"})).await;
        app.present_approval(prompt);

        // Runtime events and approval prompts use independent channels. The
        // host loop may receive the prompt first even though TurnStarted was
        // emitted first on the event stream.
        app.apply(&event(RuntimeEvent::TurnStarted));

        assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
        app.on_key(key(KeyCode::Char('y')));
        assert_eq!(
            decision.await.expect("approval decision"),
            ApprovalDecision::Allow
        );
    }

    #[tokio::test]
    async fn an_edit_approval_carries_its_review_but_a_shell_one_does_not() {
        let mut edit = app();
        edit.present_approval(
            prompt_with(
                "edit",
                serde_json::json!({
                    "path": "src/retry.rs",
                    "old_string": "once();\n",
                    "new_string": "twice();\n",
                }),
            )
            .await,
        );
        match &edit.overlay {
            Some(Overlay::Approval { review, .. }) => {
                let review = review.as_ref().expect("an edit call is reviewable");
                assert_eq!(review.path, "src/retry.rs");
                assert_eq!(review.added, 1);
            }
            other => panic!("expected an approval, got {other:?}"),
        }

        let mut shell = app();
        shell.present_approval(prompt("shell").await);
        match &shell.overlay {
            Some(Overlay::Approval { review, .. }) => assert!(
                review.is_none(),
                "a shell call must fall back to its arguments"
            ),
            other => panic!("expected an approval, got {other:?}"),
        }
    }

    #[test]
    fn streaming_text_lands_in_the_transcript_with_usage_in_the_header() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(text_delta("The retry ")));
        app.apply(&event(text_delta("policy")));
        assert_eq!(app.speculative_text(), Some("The retry policy"));
        assert!(app.transcript.is_empty());
        app.apply(&event(commit_output()));
        app.apply(&event(RuntimeEvent::Usage {
            record: UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance::default(),
                delta: UsageDelta::new().with(CounterKind::InputUncached, 12_400),
            },
        }));
        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        }));

        assert_eq!(app.transcript.len(), 2);
        assert_eq!(
            app.transcript.blocks()[0],
            Block::Assistant {
                text: "The retry policy".into(),
                open: false
            }
        );
        assert_eq!(
            app.transcript.blocks()[1],
            Block::Notice {
                source: "turn".into(),
                text: "completed".into(),
            }
        );
        assert!(
            !app.transcript
                .blocks()
                .iter()
                .any(|block| matches!(block, Block::Notice { source, .. } if source == "work"))
        );
        assert_eq!(app.status.context.render(), "12.4k");
        assert_eq!(app.status.activity, Activity::Idle);
    }

    #[test]
    fn a_provider_change_warns_that_the_cache_does_not_transfer() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::ModelProfileResolved {
            provider: "openai".into(),
            model: ModelId::new("gpt-5.3"),
            profile: fingerprint("profile"),
        }));
        // The first resolution is not a change, so it must not add a notice.
        assert!(app.transcript.is_empty());

        app.apply(&event(RuntimeEvent::Usage {
            record: UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance::default(),
                delta: UsageDelta::new().with(CounterKind::InputUncached, 9_000),
            },
        }));
        app.apply(&event(RuntimeEvent::ModelProfileResolved {
            provider: "anthropic".into(),
            model: ModelId::new("claude-opus-5"),
            profile: fingerprint("profile"),
        }));

        match &app.transcript.blocks()[0] {
            Block::Notice { source, text } => {
                assert_eq!(source, "provider");
                assert!(text.contains("not transferable"), "{text}");
            }
            other => panic!("expected a provider notice, got {other:?}"),
        }
        assert_eq!(app.status.context.render(), "~9k");
    }

    #[test]
    fn an_interrupted_turn_says_so() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(text_delta("partial")));
        app.apply(&event(discard_output()));
        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Cancelled {
                reason: CancelReason::UserRequested,
            },
            visible_output: true,
        }));

        assert_eq!(app.status.activity, Activity::Idle);
        assert!(matches!(
            app.transcript.blocks().last(),
            Some(Block::Notice { .. })
        ));
    }

    #[test]
    fn a_completed_turn_uses_canonical_duration_and_clears_live_timing() {
        let mut app = app();
        app.apply(&event_at(Timestamp(1_000), RuntimeEvent::TurnStarted));
        app.turn_started_at = Instant::now().checked_sub(Duration::from_secs(65));
        assert!(
            app.turn_elapsed()
                .is_some_and(|elapsed| elapsed.as_secs() >= 65)
        );

        app.apply(&event_at(
            Timestamp(66_000),
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));

        assert!(app.turn_elapsed().is_none());
        assert_eq!(app.turn_started_timestamp, None);
        assert_eq!(
            app.transcript.blocks(),
            &[Block::Notice {
                source: "turn".into(),
                text: "completed in 1m 05s".into(),
            }]
        );
    }

    #[test]
    fn a_success_without_visible_text_keeps_an_honest_subsecond_notice() {
        let mut app = app();
        app.apply(&event_at(Timestamp(1_000), RuntimeEvent::TurnStarted));
        app.apply(&event_at(
            Timestamp(1_842),
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: false,
            },
        ));

        assert_eq!(app.status.activity, Activity::Idle);
        assert!(app.turn_elapsed().is_none());
        let rendered = format!("{:?}", app.transcript.blocks());
        assert!(rendered.contains("completed in 842ms"), "{rendered}");
        assert!(!rendered.contains("reasoning only"), "{rendered}");
    }

    #[test]
    fn unavailable_or_backward_canonical_timing_never_fabricates_duration() {
        for (started, completed) in [
            (Timestamp::ZERO, Timestamp(5_000)),
            (Timestamp(7_000), Timestamp(6_000)),
        ] {
            let mut app = app();
            app.apply(&event_at(started, RuntimeEvent::TurnStarted));
            app.apply(&event_at(
                completed,
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ));
            assert_eq!(
                app.transcript.blocks(),
                &[Block::Notice {
                    source: "turn".into(),
                    text: "completed".into(),
                }]
            );
        }
    }

    #[test]
    fn an_error_event_becomes_a_visible_error_block() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::Error {
            error: RuntimeError::config("no provider is configured"),
        }));
        match &app.transcript.blocks()[0] {
            Block::Error { message } => assert!(message.contains("no provider")),
            other => panic!("expected an error block, got {other:?}"),
        }
    }

    #[test]
    fn a_live_sequence_gap_is_visible_instead_of_silently_losing_output() {
        let mut app = app();
        let mut first = event(RuntimeEvent::TurnStarted);
        first.seq = 4;
        app.apply(&first);
        let mut later = event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        });
        later.seq = 7;
        app.apply(&later);

        assert!(
            app.transcript.blocks().iter().any(|block| matches!(
                block,
                Block::Error { message } if message.contains("sequence 5 through 6")
            )),
            "{:?}",
            app.transcript.blocks()
        );
    }

    #[test]
    fn reducer_event_v4_pre_attempt_scoping_fixture_is_frozen() {
        // This is deliberately JSON evidence rather than a current
        // `EventEnvelope` decode test. Event schema v4 streamed text without a
        // request/attempt identity, so it cannot safely reconstruct
        // speculative output after the v5 migration. A separate v5 fixture
        // exercises the current reducer contract.
        let fixture = include_str!("../tests/fixtures/reducer-events-v4-pre-attempt-scoping.json");
        let events: serde_json::Value =
            serde_json::from_str(fixture).expect("valid reducer fixture");
        let events = events.as_array().expect("an event array");

        assert_eq!(events.len(), 5);
        assert!(events.iter().all(|event| event["schema_version"] == 4));
        let delta = &events[2]["payload"];
        assert_eq!(delta["event"], "text_delta");
        assert_eq!(delta["text"], "fixture answer");
        assert!(delta.get("request").is_none());
        assert!(delta.get("attempt").is_none());
        assert!(
            serde_json::from_str::<Vec<EventEnvelope>>(fixture).is_err(),
            "v5 must reject unattributed v4 deltas instead of synthesizing attempt identity"
        );
    }

    #[test]
    fn reducer_event_v5_fixture_commits_only_the_successful_attempt() {
        let events: Vec<EventEnvelope> = serde_json::from_str(include_str!(
            "../tests/fixtures/reducer-events-v5-attempt-scoped.json"
        ))
        .expect("valid v5 reducer fixture");
        let mut app = app();
        for event in &events {
            app.apply(event);
        }

        assert_eq!(app.status.activity, Activity::Idle);
        assert_eq!(app.status.context.render(), "30");
        assert_eq!(app.speculative_attempt_count(), 0);
        assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Assistant { text, open: false } if text == "fixture answer"
        )));
        assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Notice { source, text }
                if source == "retry" && text.contains("attempt-failed")
        )));
        assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Tool {
                call_id,
                status: ToolStatus::Ok,
                ..
            } if call_id == "call-fixture"
        )));
        assert!(
            !format!("{:?}", app.transcript.blocks()).contains("discarded prefix"),
            "failed speculative output entered the committed transcript"
        );
    }

    #[test]
    fn live_reducer_and_journal_replay_produce_equivalent_ui_state() {
        use agent_runtime_core::interaction::InteractionOutcomeKind;
        use agent_runtime_core::provider::FinishReason;

        let session = SessionId::new("replay-session");
        let ordinary_turn = TurnId::new("turn-ordinary");
        let harness_turn = TurnId::new("turn-harness");
        let ordinary_request = RequestId::new("request-ordinary");
        let ordinary_attempt = AttemptId::new("attempt-ordinary");
        let retry_request = RequestId::new("request-retry");
        let failed_attempt = AttemptId::new("attempt-failed");
        let successful_attempt = AttemptId::new("attempt-successful");
        let final_request = RequestId::new("request-final");
        let final_attempt = AttemptId::new("attempt-final");
        let edit_call = ToolCallId::new("call-approved-edit");
        let question_call = ToolCallId::new("call-question");
        let question_request = InteractionRequestId::new("interaction-direction");
        let events = vec![
            (None, RuntimeEvent::SessionStarted),
            (
                None,
                RuntimeEvent::CapabilitiesActivated {
                    epoch: 1,
                    activation: vec![
                        ActivatedCapability::new(
                            agent_runtime_registry::RegistryId::tool("read"),
                            agent_runtime_registry::RegistryRevision::new("read-1"),
                        ),
                        ActivatedCapability::new(
                            agent_runtime_registry::RegistryId::tool("edit"),
                            agent_runtime_registry::RegistryRevision::new("edit-1"),
                        ),
                    ],
                },
            ),
            (Some(ordinary_turn.clone()), RuntimeEvent::TurnStarted),
            (
                Some(ordinary_turn.clone()),
                RuntimeEvent::ProviderAttemptStarted {
                    request: ordinary_request.clone(),
                    attempt: ordinary_attempt.clone(),
                    index: 0,
                    model: "fixture-model".to_owned(),
                },
            ),
            (
                Some(ordinary_turn.clone()),
                RuntimeEvent::TextDelta {
                    request: ordinary_request.clone(),
                    attempt: ordinary_attempt.clone(),
                    text: "ordinary committed answer".to_owned(),
                },
            ),
            (
                Some(ordinary_turn.clone()),
                RuntimeEvent::ProviderAttemptOutputCommitted {
                    request: ordinary_request,
                    attempt: ordinary_attempt.clone(),
                },
            ),
            (
                Some(ordinary_turn.clone()),
                RuntimeEvent::ProviderAttemptFinished {
                    attempt: ordinary_attempt,
                    finish: FinishReason::Stop,
                    retryable: false,
                },
            ),
            (
                Some(ordinary_turn),
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ),
            (Some(harness_turn.clone()), RuntimeEvent::TurnStarted),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptStarted {
                    request: retry_request.clone(),
                    attempt: failed_attempt.clone(),
                    index: 0,
                    model: "fixture-model".to_owned(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::TextDelta {
                    request: retry_request.clone(),
                    attempt: failed_attempt.clone(),
                    text: "discarded speculative prefix".to_owned(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptOutputDiscarded {
                    request: retry_request.clone(),
                    attempt: failed_attempt.clone(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptFinished {
                    attempt: failed_attempt,
                    finish: FinishReason::Error,
                    retryable: true,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptStarted {
                    request: retry_request.clone(),
                    attempt: successful_attempt.clone(),
                    index: 1,
                    model: "fixture-model".to_owned(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::TextDelta {
                    request: retry_request.clone(),
                    attempt: successful_attempt.clone(),
                    text: "retry committed answer".to_owned(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptOutputCommitted {
                    request: retry_request,
                    attempt: successful_attempt.clone(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptFinished {
                    attempt: successful_attempt,
                    finish: FinishReason::ToolCalls,
                    retryable: false,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ToolCallRequested {
                    call: edit_call.clone(),
                    name: "edit".to_owned(),
                    argument_keys: vec![
                        "new_string".to_owned(),
                        "old_string".to_owned(),
                        "path".to_owned(),
                    ],
                    argument_fingerprint: fingerprint("approved-edit"),
                    arguments: None,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ToolCallCompleted {
                    call: edit_call,
                    name: "edit".to_owned(),
                    is_error: false,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ToolCallRequested {
                    call: question_call.clone(),
                    name: "ask_user".to_owned(),
                    argument_keys: vec!["questions".to_owned()],
                    argument_fingerprint: fingerprint("question"),
                    arguments: None,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::InteractionRequested {
                    request: question_request.clone(),
                    call: question_call.clone(),
                    question_count: 1,
                    sensitivity: InteractionSensitivity::Sensitive,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::InteractionResolved {
                    request: question_request,
                    call: question_call.clone(),
                    outcome: InteractionOutcomeKind::Answered,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ToolCallCompleted {
                    call: question_call,
                    name: "ask_user".to_owned(),
                    is_error: false,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::PlanUpdated {
                    revision: 2,
                    sensitivity: PlanSensitivity::Public,
                    counts: BTreeMap::from([
                        ("cancelled".to_owned(), 0),
                        ("completed".to_owned(), 1),
                        ("in_progress".to_owned(), 1),
                        ("pending".to_owned(), 0),
                    ]),
                    items: Some(vec![
                        PlanItemProjection {
                            id: "inspect".to_owned(),
                            text: "Inspect state".to_owned(),
                            status: PlanItemStatus::Completed,
                            reason: None,
                        },
                        PlanItemProjection {
                            id: "verify".to_owned(),
                            text: "Verify replay".to_owned(),
                            status: PlanItemStatus::InProgress,
                            reason: None,
                        },
                    ]),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptStarted {
                    request: final_request.clone(),
                    attempt: final_attempt.clone(),
                    index: 0,
                    model: "fixture-model".to_owned(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::TextDelta {
                    request: final_request.clone(),
                    attempt: final_attempt.clone(),
                    text: "tools and question completed".to_owned(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptOutputCommitted {
                    request: final_request,
                    attempt: final_attempt.clone(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptFinished {
                    attempt: final_attempt,
                    finish: FinishReason::Stop,
                    retryable: false,
                },
            ),
            (
                Some(harness_turn),
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(seq, (turn, payload))| {
            EventEnvelope::new(
                u64::try_from(seq).expect("fixture sequence"),
                EventId::new(format!("event-{seq}")),
                session.clone(),
                turn,
                Timestamp(10_000 + u64::try_from(seq).expect("fixture time") * 100),
                payload,
            )
        })
        .collect::<Vec<_>>();
        let journal_bytes = serde_json::to_vec(&events).expect("serializable journal events");
        let replayed_events: Vec<EventEnvelope> =
            serde_json::from_slice(&journal_bytes).expect("replayable journal events");

        let mut live = app();
        live.present_recovered_ephemeral_work(1, 1);
        for event in &events {
            live.apply(event);
        }
        let mut replayed = app();
        replayed.present_recovered_ephemeral_work(1, 1);
        for event in &replayed_events {
            replayed.apply(event);
        }

        assert_eq!(live.transcript.blocks(), replayed.transcript.blocks());
        assert_eq!(live.status.activity, replayed.status.activity);
        assert_eq!(live.status.context, replayed.status.context);
        assert_eq!(live.status.context_plan, replayed.status.context_plan);
        assert_eq!(live.status.cache_read, replayed.status.cache_read);
        assert_eq!(live.status.capabilities, replayed.status.capabilities);
        assert_eq!(live.plan, replayed.plan);
        assert_eq!(live.children, replayed.children);
        assert_eq!(live.pending_approval_count(), 0);
        assert_eq!(replayed.pending_approval_count(), 0);
        assert_eq!(live.pending_questionnaire_count(), 0);
        assert_eq!(replayed.pending_questionnaire_count(), 0);
        assert_eq!(live.speculative_attempt_count(), 0);
        assert_eq!(replayed.speculative_attempt_count(), 0);
        let rendered = format!("{:?}", live.transcript.blocks());
        assert!(rendered.contains("ordinary committed answer"));
        assert!(rendered.contains("retry committed answer"));
        assert!(rendered.contains("call-approved-edit"));
        assert!(rendered.contains("call-question"));
        assert!(rendered.contains("not restarted"));
        assert!(rendered.contains("completed in"));
        assert!(!rendered.contains("reasoning only"));
        assert!(
            !rendered.contains("discarded speculative prefix"),
            "failed attempt output entered the committed transcript"
        );
    }

    #[test]
    fn unterminated_speculative_output_is_discarded_at_the_turn_boundary() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(text_delta("orphaned draft")));
        assert_eq!(app.speculative_attempt_count(), 1);

        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Failed,
            visible_output: false,
        }));

        assert_eq!(app.speculative_attempt_count(), 0);
        assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Notice { source, text }
                if source == "integrity" && text.contains("unterminated")
        )));
        assert!(!format!("{:?}", app.transcript.blocks()).contains("orphaned draft"));
    }

    #[test]
    fn scrolling_up_pauses_following_until_the_user_returns() {
        let mut app = app();
        app.sync_scroll_limit(30);
        assert!(app.following);

        app.on_key(key(KeyCode::PageUp));
        assert!(!app.following);
        assert_eq!(app.scroll_back, 10);

        app.on_key(key(KeyCode::End));
        assert!(app.following);
        assert_eq!(app.scroll_back, 0);

        app.on_key(key(KeyCode::Home));
        assert!(!app.following);
        assert_eq!(app.scroll_back, 30);

        app.on_key(ctrl('l'));
        assert!(app.following);
        assert_eq!(app.scroll_back, 0);
    }

    #[test]
    fn scrolling_without_overflow_keeps_following_newest() {
        let mut app = app();

        app.on_key(key(KeyCode::PageUp));
        app.on_key(key(KeyCode::Home));

        assert!(app.following);
        assert_eq!(app.scroll_back, 0);
    }

    #[test]
    fn paused_scrolling_keeps_the_visible_offset_when_content_grows() {
        let mut app = app();
        app.sync_scroll_limit(20);
        app.scroll_up(5);

        app.sync_scroll_limit(30);

        assert!(!app.following);
        assert_eq!(app.scroll_back, 15);
    }

    #[test]
    fn sending_a_message_resumes_following() {
        let mut app = app();
        app.sync_scroll_limit(20);
        app.scroll_up(5);
        assert!(!app.following);

        type_text(&mut app, "next");
        app.on_key(key(KeyCode::Enter));
        assert!(app.following);
    }

    #[test]
    fn tab_never_changes_regions_and_completes_without_execution() {
        let mut app = app();
        app.on_key(key(KeyCode::Tab));
        assert!(app.composer.is_empty());

        type_text(&mut app, "/sta");
        assert!(matches!(app.overlay, Some(Overlay::Palette { .. })));
        assert_eq!(app.on_key(key(KeyCode::Tab)), None);
        assert_eq!(app.composer.text(), "/status");
        assert!(app.overlay.is_none());
        assert!(app.transcript.is_empty(), "completion must not execute");

        app.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(app.composer.text(), "/status");
    }

    fn agent_first_app() -> App {
        let mut app = app();
        app.status.switch_model(Some("zai".to_owned()), "glm-5.2");
        app.status.set_agent("build");
        app.set_resources(RuntimeResources {
            files: vec![ResourceEntry::new(
                "file:src/lib.rs",
                "@src/lib.rs",
                "file · 42 bytes",
            )],
            child_agents: vec![ResourceEntry::new(
                "agent:review",
                "@review",
                "agent · read-only child preset",
            )],
            agent_modes: vec![
                ResourceEntry::new("build", "build", "coding").active(true),
                ResourceEntry::new("plan", "plan", "read-only planning"),
                ResourceEntry::new("review", "review", "read-only review"),
            ],
            ..app.resources.clone()
        });
        app
    }

    #[test]
    fn tab_cycles_only_an_empty_idle_root_agent() {
        let mut app = agent_first_app();
        assert_eq!(
            app.on_key(key(KeyCode::Tab)),
            Some(Action::Reconfigure(PaletteCommand::Agent(
                "plan".to_owned()
            )))
        );

        type_text(&mut app, "draft");
        assert_eq!(app.on_key(key(KeyCode::Tab)), None);
        assert_eq!(app.composer.text(), "draft");

        app.composer.clear();
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(Action::Reconfigure(PaletteCommand::Agent(
                "review".to_owned()
            )))
        );

        app.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(app.on_key(key(KeyCode::Tab)), None);
    }

    #[test]
    fn file_reference_submission_is_typed_and_unresolved_drafts_fail_locally() {
        let mut app = agent_first_app();
        app.composer.replace("inspect @src/lib.rs");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::SendWithFiles {
                text: "inspect @src/lib.rs".to_owned(),
                files: vec!["src/lib.rs".to_owned()],
            })
        );

        let mut unresolved = agent_first_app();
        unresolved.composer.replace("inspect @missing.rs");
        assert_eq!(unresolved.on_key(key(KeyCode::Enter)), None);
        assert_eq!(unresolved.composer.text(), "inspect @missing.rs");
        assert!(unresolved.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Error { message } if message.contains("unresolved reference")
        )));
    }

    #[test]
    fn at_escape_and_shell_escape_are_provider_prompts() {
        let mut at = agent_first_app();
        at.on_key(key(KeyCode::Char('@')));
        assert!(matches!(at.overlay, Some(Overlay::ResourcePicker { .. })));
        at.on_key(key(KeyCode::Char('@')));
        type_text(&mut at, "owner please");
        assert_eq!(
            at.on_key(key(KeyCode::Enter)),
            Some(Action::Send("@owner please".to_owned()))
        );

        let mut shell = agent_first_app();
        shell.composer.replace("!cargo test --workspace");
        assert_eq!(
            shell.on_key(key(KeyCode::Enter)),
            Some(Action::RunShell {
                command: "cargo test --workspace".to_owned(),
            })
        );

        let mut literal_shell = agent_first_app();
        literal_shell.composer.replace("!!explain shell syntax");
        assert_eq!(
            literal_shell.on_key(key(KeyCode::Enter)),
            Some(Action::Send("!explain shell syntax".to_owned()))
        );
    }

    #[test]
    fn explicit_child_requires_non_default_spend_confirmation() {
        let mut app = agent_first_app();
        app.composer.replace("@review inspect the diff");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::AgentConfirm { ref preset, ref task, ref content })
                if preset == "review"
                    && task == "inspect the diff"
                    && content.contains("zai/glm-5.2")
                    && content.contains("read-only")
                    && content.contains("provider spend: yes")
        ));
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(app.overlay, Some(Overlay::AgentConfirm { .. })));
        assert_eq!(
            app.on_key(key(KeyCode::Char('y'))),
            Some(Action::StartAgent {
                preset: "review".to_owned(),
                task: "inspect the diff".to_owned(),
            })
        );
        assert!(app.composer.is_empty());
    }

    #[test]
    fn existing_child_reference_confirms_a_follow_up_instead_of_spawning() {
        let mut app = agent_first_app();
        app.restore_child(
            "child-1",
            "idle",
            Some("durable · session child-session-1 · 1/4 turns".to_owned()),
        );
        app.composer.replace("@child-1 check the parser edge case");

        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::AgentFollowUpConfirm {
                ref child_id,
                ref task,
                ref content,
            }) if child_id == "child-1"
                && task == "check the parser edge case"
                && content.contains("new follow-up turn")
                && content.contains("reuse prior child history")
        ));
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(
            app.on_key(key(KeyCode::Char('y'))),
            Some(Action::FollowUpAgent {
                child_id: "child-1".to_owned(),
                task: "check the parser edge case".to_owned(),
            })
        );
    }

    #[test]
    fn interrupted_child_resume_is_explicit_and_has_no_enter_default() {
        let mut app = agent_first_app();
        app.restore_child(
            "child-2",
            "interrupted",
            Some("durable · session child-session-2 · resumable".to_owned()),
        );
        app.composer.replace("/agent resume child-2");

        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::AgentResumeConfirm { ref child_id, ref content })
                if child_id == "child-2"
                    && content.contains("exact interrupted checkpoint")
                    && content.contains("turn slot consumed: no")
        ));
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(
            app.on_key(key(KeyCode::Char('y'))),
            Some(Action::ResumeAgent {
                child_id: "child-2".to_owned(),
            })
        );
    }

    #[test]
    fn slash_and_ctrl_p_open_the_same_filtered_registry() {
        let mut slash = app();
        type_text(&mut slash, "/rev");
        let slash_matches = commands::matches(slash.composer.text());

        let mut palette = app();
        palette.on_key(ctrl('p'));
        type_text(&mut palette, "rev");
        let palette_matches = commands::matches(palette.composer.text());

        assert_eq!(slash_matches, palette_matches);
        assert_eq!(
            slash_matches
                .iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            ["review", "revert"]
        );
    }

    #[test]
    fn dismissing_ctrl_p_restores_the_original_draft() {
        let mut app = app();
        type_text(&mut app, "keep this");
        app.on_key(ctrl('p'));
        assert!(app.composer.text().starts_with('/'));
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.composer.text(), "keep this");
        assert!(app.overlay.is_none());
    }

    #[test]
    fn local_status_and_agent_commands_never_become_provider_sends() {
        for command in ["/status", "/context", "/agent", "/diff"] {
            let mut app = app();
            type_text(&mut app, command);
            assert!(matches!(
                app.on_key(key(KeyCode::Enter)),
                Some(Action::Command(_))
            ));
            assert!(
                !app.transcript
                    .blocks()
                    .iter()
                    .any(|block| matches!(block, Block::User { .. }))
            );
        }
    }

    #[test]
    fn context_planning_telemetry_becomes_bounded_status_state() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::ContextPlanned {
            context: fingerprint("context"),
            cache_plan: fingerprint("cache"),
            segment_count: 2,
            totals: BTreeMap::from([
                (SegmentKind::new("history"), 1_500),
                (SegmentKind::new("tool_schema"), 500),
            ]),
            input_tokens: 2_000,
            input_budget_tokens: 10_000,
            reserved_tokens: 2_000,
            confidence: EstimationConfidence::Estimated,
        }));

        let plan = app.status.context_plan.expect("context plan");
        assert_eq!(plan.fingerprint, fingerprint("context").as_str());
        assert_eq!(plan.cache_fingerprint, fingerprint("cache").as_str());
        assert_eq!(plan.input_tokens, 2_000);
        assert_eq!(plan.input_budget_tokens, 10_000);
        assert_eq!(plan.reserved_tokens, 2_000);
        assert_eq!(plan.segment_count, 2);
        assert_eq!(plan.totals["history"], 1_500);
        assert_eq!(plan.render_footer(), "~80% ctx");
    }

    #[test]
    fn capability_lifecycle_becomes_bounded_status_and_a_concise_notice() {
        let mut app = app();
        let snapshot = fingerprint("registry");
        let view = fingerprint("view");
        app.apply(&event(RuntimeEvent::RegistrySnapshotSealed {
            snapshot: snapshot.clone(),
            entries: 6,
        }));
        app.apply(&event(RuntimeEvent::ScopedViewDerived {
            snapshot,
            view: view.clone(),
            visible_entries: 4,
        }));
        app.apply(&event(RuntimeEvent::CapabilityRetrievalPerformed {
            resolver_revision: agent_runtime_registry::RegistryRevision::new("resolver-1"),
            index_revision: None,
            candidates: vec![
                agent_runtime_registry::RegistryId::tool("read"),
                agent_runtime_registry::RegistryId::tool("search"),
            ],
        }));
        app.apply(&event(RuntimeEvent::CapabilitiesActivated {
            epoch: 2,
            activation: vec![
                ActivatedCapability::new(
                    agent_runtime_registry::RegistryId::tool("read"),
                    agent_runtime_registry::RegistryRevision::new("read-1"),
                ),
                ActivatedCapability::new(
                    agent_runtime_registry::RegistryId::tool("search"),
                    agent_runtime_registry::RegistryRevision::new("search-1"),
                ),
            ],
        }));

        assert_eq!(
            app.status.capabilities.registry,
            Some((fingerprint("registry").as_str().to_owned(), 6))
        );
        assert_eq!(
            app.status.capabilities.view,
            Some((view.as_str().to_owned(), 4))
        );
        assert_eq!(
            app.status.capabilities.retrieval,
            Some((
                "resolver-1".to_owned(),
                vec!["tool:read".to_owned(), "tool:search".to_owned()]
            ))
        );
        assert_eq!(
            app.status.capabilities.activation,
            Some((2, vec!["tool:read".to_owned(), "tool:search".to_owned()]))
        );
        assert!(app.transcript.blocks().iter().any(|block| {
            matches!(
                block,
                Block::Notice { source, text }
                    if source == "capabilities"
                        && text == "activation epoch 2: tool:read, tool:search"
            )
        }));
    }

    #[test]
    fn public_todo_updates_remain_replaceable_and_replay_equivalent() {
        let update = event(RuntimeEvent::PlanUpdated {
            revision: 3,
            sensitivity: PlanSensitivity::Public,
            counts: BTreeMap::from([
                ("cancelled".to_owned(), 0),
                ("completed".to_owned(), 1),
                ("in_progress".to_owned(), 1),
                ("pending".to_owned(), 1),
            ]),
            items: Some(vec![
                PlanItemProjection {
                    id: "inspect".to_owned(),
                    text: "Inspect\nrelevant code".to_owned(),
                    status: PlanItemStatus::Completed,
                    reason: None,
                },
                PlanItemProjection {
                    id: "change".to_owned(),
                    text: "Implement the change".to_owned(),
                    status: PlanItemStatus::InProgress,
                    reason: None,
                },
                PlanItemProjection {
                    id: "verify".to_owned(),
                    text: "Run focused tests".to_owned(),
                    status: PlanItemStatus::Pending,
                    reason: None,
                },
            ]),
        });
        let mut live = app();
        live.apply(&event(RuntimeEvent::TurnStarted));
        live.apply(&update);
        let mut replayed = app();
        replayed.apply(&event(RuntimeEvent::TurnStarted));
        replayed.apply(&update);

        assert_eq!(live.plan, replayed.plan);
        assert_eq!(live.transcript.blocks(), replayed.transcript.blocks());
        assert!(live.work_detail_lines().is_empty());
        assert!(
            !live
                .transcript
                .blocks()
                .iter()
                .any(|block| matches!(block, Block::Notice { source, .. } if source == "plan")),
            "plan updates must replace one work row instead of appending notices"
        );
    }

    #[test]
    fn sensitive_todo_update_displays_counts_without_item_text() {
        const PROTECTED_ITEM: &str = "PROTECTED PLAN CONTENT";
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::PlanUpdated {
            revision: 1,
            sensitivity: PlanSensitivity::Sensitive,
            counts: BTreeMap::from([
                ("cancelled".to_owned(), 1),
                ("completed".to_owned(), 0),
                ("in_progress".to_owned(), 0),
                ("pending".to_owned(), 2),
            ]),
            items: Some(vec![PlanItemProjection {
                id: "protected".to_owned(),
                text: PROTECTED_ITEM.to_owned(),
                status: PlanItemStatus::Pending,
                reason: None,
            }]),
        }));

        let plan = app.plan.as_ref().expect("latest plan");
        assert_eq!(plan.sensitivity, PlanSensitivity::Sensitive);
        assert!(
            plan.items.is_none(),
            "sensitive item text survived the reducer seam"
        );
        assert!(app.work_detail_lines().is_empty());
        assert!(!format!("{:?}", app.transcript.blocks()).contains(PROTECTED_ITEM));
    }

    #[test]
    fn local_results_append_without_stealing_the_composer() {
        let mut app = app();
        type_text(&mut app, "keep drafting");
        app.scroll_up(4);

        app.show_local_result("status", "model: example");
        app.show_local_empty("agents", "No child agents in this session.");

        assert_eq!(app.composer.text(), "keep drafting");
        assert!(app.overlay.is_none());
        assert!(app.following);
        let results = app
            .transcript
            .blocks()
            .iter()
            .filter_map(|block| match block {
                Block::LocalResult { title, .. } => Some(title.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(results, ["status", "agents"]);
        assert!(
            !app.transcript
                .blocks()
                .iter()
                .any(|block| matches!(block, Block::User { text } if text == "keep drafting")),
            "a local result must not turn the draft into provider input"
        );
    }

    #[test]
    fn recovery_and_review_confirmations_have_no_enter_default() {
        let mut undo = app();
        undo.confirm_undo("--- current\n+++ restore\n-old\n+new\n");
        assert_eq!(undo.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(undo.overlay, Some(Overlay::UndoConfirm { .. })));
        assert_eq!(undo.on_key(key(KeyCode::Esc)), Some(Action::CancelUndo));

        let mut review = app();
        review.confirm_review("all", "provider-backed: yes");
        assert_eq!(review.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            review.overlay,
            Some(Overlay::ReviewConfirm { .. })
        ));

        let mut revert = app();
        revert.confirm_revert("file.txt", "fingerprint", "reverse patch");
        assert_eq!(
            revert.on_key(key(KeyCode::Char('n'))),
            Some(Action::CancelRevert {
                scope: "file.txt".to_owned(),
                fingerprint: "fingerprint".to_owned(),
            })
        );
    }

    #[test]
    fn child_lifecycle_is_inline_and_available_for_agent_detail() {
        let mut app = app();
        let child = ChildId::new("child-1");
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        }));
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: child.clone(),
            result: "No findings.".to_owned(),
        }));
        assert_eq!(app.children[child.as_str()].state, "completed");
        assert!(app.transcript.blocks().iter().any(|block| {
            matches!(block, Block::Notice { text, .. } if text.contains("No findings"))
        }));
    }

    #[test]
    fn durable_child_recovery_and_resume_replay_match_live_projection() {
        let child = ChildId::new("child-9");
        let session = SessionId::new("child-session-9");
        let payloads = vec![
            RuntimeEvent::ChildProgress {
                child: child.clone(),
                phase: ChildPhase::Recovered {
                    child_session: session.clone(),
                    state: ChildRecoveryState::Interrupted,
                    resumable: true,
                },
            },
            RuntimeEvent::ChildProgress {
                child: child.clone(),
                phase: ChildPhase::ResumeStarted {
                    child_session: session.clone(),
                },
            },
            RuntimeEvent::ChildProgress {
                child: child.clone(),
                phase: ChildPhase::Interrupted {
                    child_session: session,
                    resumable: false,
                },
            },
        ];
        let events = payloads
            .into_iter()
            .enumerate()
            .map(|(sequence, payload)| {
                EventEnvelope::new(
                    u64::try_from(sequence).expect("bounded sequence"),
                    EventId::new(format!("child-event-{sequence}")),
                    SessionId::new("parent-session"),
                    None,
                    Timestamp::ZERO,
                    payload,
                )
            })
            .collect::<Vec<_>>();
        let encoded = serde_json::to_vec(&events).expect("events serialize");
        let replayed: Vec<EventEnvelope> =
            serde_json::from_slice(&encoded).expect("events deserialize");

        let mut live = app();
        for event in &events {
            live.apply(event);
        }
        let mut replay = app();
        for event in &replayed {
            replay.apply(event);
        }

        assert_eq!(live.children, replay.children);
        assert_eq!(live.transcript.blocks(), replay.transcript.blocks());
        assert_eq!(live.children[child.as_str()].state, "interrupted");
        assert!(
            live.children[child.as_str()]
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("no compatible checkpoint"))
        );
    }

    #[test]
    fn child_needs_input_is_metadata_only_and_does_not_open_a_questionnaire() {
        let mut app = app();
        let child = ChildId::new("child-ask");
        app.apply(&event(RuntimeEvent::ChildNeedsInput {
            child: child.clone(),
            child_session: SessionId::new("child-session"),
            turn: TurnId::new("child-turn"),
            call: ToolCallId::new("ask-call"),
            request: InteractionRequestId::new("child-request"),
            question_ids: vec![
                QuestionId::new("question-one"),
                QuestionId::new("question-two"),
            ],
            sensitivity: InteractionSensitivity::Sensitive,
        }));

        let summary = &app.children[child.as_str()];
        assert_eq!(summary.state, "needs input");
        assert!(
            summary
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("2 questions"))
        );
        assert!(
            app.overlay.is_none(),
            "a child request must not seize the root questionnaire overlay"
        );
        assert!(app.transcript.blocks().iter().any(|block| {
            matches!(
                block,
                Block::Notice { source, text }
                    if source == "sub-agent"
                        && text.contains("child-request")
                        && text.contains("2 questions")
            )
        }));
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut app = app();
        let mut release = key(KeyCode::Char('x'));
        release.kind = KeyEventKind::Release;
        app.on_key(release);
        assert!(app.composer.is_empty());
    }

    #[test]
    fn notifications_render_inline_without_a_focusable_inbox() {
        let mut app = app();
        app.notify(Notification {
            source: "monitor:build".into(),
            text: "stopped".into(),
            terminal: true,
        });
        assert!(matches!(
            app.transcript.blocks().last(),
            Some(Block::Notice { source, text })
                if source == "monitor:build" && text.contains("finished")
        ));
    }
}
