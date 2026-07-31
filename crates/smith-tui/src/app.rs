//! Application state: the reducer over runtime events and key presses.
//!
//! [`App`] is deliberately free of I/O. It folds [`EventEnvelope`]s and
//! [`KeyEvent`]s into state and returns [`Action`]s for the host loop to
//! perform. Everything the screen shows is derivable from this struct, which is
//! what makes the renderer testable against a fake terminal and the key map
//! testable with no terminal at all.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use agent_runtime_core::event::{ChildPhase, EventEnvelope, RuntimeEvent, TurnFinish};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use smith_host::approval::{ApprovalPrompt, PromptScope};
use smith_tools::ToolCallDisplay;

use crate::commands::{self, CommandAction};
use crate::composer::Composer;
use crate::diff::EditReview;
use crate::picker::{PickerOutcome, ResourceEntry, ResourcePicker};
use crate::status::{Activity, Status, render_elapsed};
use crate::transcript::{LocalResultState, ToolStatus, Transcript};

/// How long a second `Ctrl+C` still counts as a force-quit.
const FORCE_QUIT_WINDOW: Duration = Duration::from_secs(1);

/// Something the host loop must do on the app's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Submit this text as a user turn.
    Send(String),
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
}

/// A modal drawn over the transcript. At most one exists at a time.
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
    /// Select a session or immutable runtime configuration.
    Palette {
        /// Selected filtered result.
        selected: usize,
        /// A parse error kept inside the modal.
        error: Option<String>,
        /// Draft restored when `Ctrl+P` discovery is dismissed.
        restore_on_escape: Option<String>,
    },
    /// Search locally available runtime/session resources.
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
    /// Exit was requested while work is live.
    ExitConfirm {
        /// An approval hidden by the confirmation and restored if the user
        /// cancels exit.
        approval: Option<(Box<ApprovalPrompt>, Option<EditReview>)>,
    },
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
    /// Latest child states, keyed by stable child id.
    pub children: BTreeMap<String, ChildSummary>,
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
    last_ctrl_c: Option<Instant>,
    last_event_seq: Option<u64>,
}

impl App {
    /// A fresh client for `model` rooted at `project`.
    pub fn new(model: impl Into<String>, project: impl Into<String>) -> Self {
        Self {
            transcript: Transcript::new(),
            status: Status::new(model, project),
            composer: Composer::new(),
            overlay: None,
            children: BTreeMap::new(),
            resources: RuntimeResources::default(),
            following: true,
            scroll_back: 0,
            scroll_limit: 0,
            tick: 0,
            should_quit: false,
            turn_started_at: None,
            last_ctrl_c: None,
            last_event_seq: None,
        }
    }

    /// Replaces the local, credential-free picker inventory.
    pub fn set_resources(&mut self, resources: RuntimeResources) {
        self.resources = resources;
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
            || matches!(
                self.overlay,
                Some(Overlay::Approval { .. }) | Some(Overlay::ExitConfirm { approval: Some(_) })
            )
    }

    /// Advances the animation clock.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Monotonic elapsed time for the active turn.
    pub fn turn_elapsed(&self) -> Option<Duration> {
        self.turn_started_at.map(|started| started.elapsed())
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

    /// Enriches a protected live tool event with a reviewed local projection.
    pub fn set_tool_display(&mut self, call_id: &str, display: ToolCallDisplay) {
        self.transcript.set_tool_display(call_id, display);
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
                self.turn_started_at = None;
            }
            RuntimeEvent::TurnStarted => {
                self.status.activity = Activity::Working;
                self.turn_started_at = Some(Instant::now());
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
            RuntimeEvent::TextDelta { text } => {
                self.transcript.push_text_delta(text);
            }
            RuntimeEvent::ReasoningDelta { text, redacted } => {
                self.transcript.push_reasoning_delta(text, *redacted);
            }
            RuntimeEvent::ToolCallRequested {
                call,
                name,
                argument_keys,
                arguments,
                ..
            } => {
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
                self.transcript.complete_tool_call(call.as_str(), status);
            }
            RuntimeEvent::ContextPlanned {
                segment_count,
                totals,
                input_tokens,
                input_budget_tokens,
                reserved_tokens,
                confidence,
                ..
            } => {
                self.status.record_context_plan(
                    *input_tokens,
                    *input_budget_tokens,
                    *reserved_tokens,
                    *segment_count,
                    totals,
                    *confidence,
                );
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
            RuntimeEvent::TurnCompleted {
                finish,
                visible_output,
            } => {
                let elapsed = self.turn_started_at.take().map(|started| started.elapsed());
                self.transcript.close_open();
                self.status.activity = Activity::Idle;
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
                    // A reasoning-only completion would otherwise end in
                    // silence; say so instead of showing nothing.
                    TurnFinish::Completed if !visible_output => {
                        self.transcript.push_notice(
                            "turn",
                            elapsed.map_or_else(
                                || "completed without a visible answer (reasoning only)".to_owned(),
                                |elapsed| {
                                    format!(
                                        "completed in {} without a visible answer (reasoning only)",
                                        render_elapsed(elapsed)
                                    )
                                },
                            ),
                        );
                    }
                    TurnFinish::Completed => {
                        if let Some(elapsed) = elapsed {
                            self.transcript.push_notice(
                                "turn",
                                format!("completed in {}", render_elapsed(elapsed)),
                            );
                        }
                    }
                    TurnFinish::Failed => {
                        if let Some(elapsed) = elapsed {
                            self.transcript.push_notice(
                                "turn",
                                format!("failed after {}", render_elapsed(elapsed)),
                            );
                        }
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
                ChildPhase::TurnStarted => {
                    if let Some(summary) = self.children.get_mut(&child.to_string()) {
                        summary.state = "working".to_owned();
                    }
                    self.transcript
                        .push_notice("sub-agent", format!("{child} is working"));
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
                self.transcript.close_open();
                self.status.activity = Activity::Ended;
                self.turn_started_at = None;
            }
            // Planning-lifecycle events carry diagnostics the basic TUI does not
            // surface yet; they are recorded by the session log regardless.
            _ => {}
        }
    }

    /// Presents an approval request.
    ///
    /// A pending overlay is answered — denied — rather than dropped silently,
    /// so a request never disappears without the model learning its fate.
    ///
    /// The diff is derived here rather than in the renderer: the arguments
    /// cannot change while the modal is open, and the redraw budget is 30 fps
    /// (`DESIGN.md` §6), so paying for it once per request is the whole cost.
    pub fn present_approval(&mut self, prompt: ApprovalPrompt) {
        if let Some(Overlay::Approval {
            prompt: previous, ..
        }) = self.overlay.take()
        {
            previous.deny("superseded by another approval request");
        }
        let review = EditReview::from_call(prompt.tool(), &prompt.request.arguments);
        self.overlay = Some(Overlay::Approval {
            prompt: Box::new(prompt),
            review,
        });
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
    }

    /// Handles a key press, returning an action for the host loop.
    pub fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        // Terminals that report both press and release would otherwise act on
        // every keystroke twice.
        if key.kind == KeyEventKind::Release {
            return None;
        }

        // Ctrl+C is checked before overlays: it must always be able to leave.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return self.on_ctrl_c();
        }
        self.last_ctrl_c = None;

        match &self.overlay {
            Some(Overlay::Approval { .. }) => return self.on_approval_key(key),
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
            Some(Overlay::ExitConfirm { .. }) => return self.on_exit_confirm_key(key),
            None => {}
        }

        match (key.code, key.modifiers) {
            // Tab has no region-focus meaning. Outside command completion it
            // deliberately does nothing and leaves the composer ready.
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
            (KeyCode::Esc, _) => self.on_escape(),
            (KeyCode::PageUp, _) => {
                self.scroll_up(10);
                None
            }
            (KeyCode::PageDown, _) => {
                self.scroll_down(10);
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
        let forced = self
            .last_ctrl_c
            .is_some_and(|previous| now.duration_since(previous) < FORCE_QUIT_WINDOW);

        if forced || !self.has_live_work() {
            self.deny_pending_approval("Smith is exiting");
            self.should_quit = true;
            return Some(Action::Quit);
        }

        self.last_ctrl_c = Some(now);
        let approval = match self.overlay.take() {
            Some(Overlay::Approval { prompt, review }) => Some((prompt, review)),
            _ => None,
        };
        self.overlay = Some(Overlay::ExitConfirm { approval });
        None
    }

    fn deny_pending_approval(&mut self, reason: &str) {
        match self.overlay.take() {
            Some(Overlay::Approval { prompt, .. }) => prompt.deny(reason),
            Some(Overlay::ExitConfirm {
                approval: Some((prompt, _)),
            }) => prompt.deny(reason),
            _ => {}
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

    fn on_resource_picker_key(&mut self, key: KeyEvent) -> Option<Action> {
        let outcome = match &mut self.overlay {
            Some(Overlay::ResourcePicker { picker, .. }) => picker.on_key(key),
            _ => return None,
        };
        match outcome {
            PickerOutcome::Pending => None,
            PickerOutcome::Cancelled => {
                if let Some(Overlay::ResourcePicker {
                    restore_on_escape, ..
                }) = self.overlay.take()
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
        }
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
                self.deny_pending_approval("Smith is exiting");
                self.should_quit = true;
                Some(Action::Quit)
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                let prior = match self.overlay.take() {
                    Some(Overlay::ExitConfirm { approval }) => approval,
                    _ => None,
                };
                self.overlay = prior.map(|(prompt, review)| Overlay::Approval { prompt, review });
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
                self.composer.clear();
                self.transcript.push_user(&text);
                self.follow_newest();
                Some(Action::Send(text))
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

    fn dispatch_command(&mut self, command: CommandAction) -> Option<Action> {
        let Some(spec) = commands::COMMANDS.iter().find(|spec| {
            let name = match &command {
                CommandAction::Help => "help",
                CommandAction::Status => "status",
                CommandAction::Context => "context",
                CommandAction::NewSession => "new",
                CommandAction::Resume(_) => "resume",
                CommandAction::Profile(_) => "profile",
                CommandAction::Provider(_) => "provider",
                CommandAction::Model(_) => "model",
                CommandAction::Agent(_) => "agent",
                CommandAction::Diff(_) => "diff",
                CommandAction::Review(_) => "review",
                CommandAction::Undo => "undo",
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
            CommandAction::Quit => {
                self.composer.clear();
                self.on_ctrl_c()
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

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::approval::{ApprovalPolicy, ApprovalRequest};
    use agent_runtime_core::cancel::CancelReason;
    use agent_runtime_core::clock::Timestamp;
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::error::RuntimeError;
    use agent_runtime_core::event::EstimationConfidence;
    use agent_runtime_core::ids::{ChildId, EventId, SessionId, ToolCallId};
    use agent_runtime_core::manifest::SegmentKind;
    use agent_runtime_core::provider::ModelId;
    use agent_runtime_core::tool::ToolEffects;
    use agent_runtime_core::usage::{
        CounterKind, Provenance, UsageDelta, UsageRecord, UsageSource,
    };
    use smith_host::approval::InteractiveApproval;

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
        });
        app
    }

    fn event(payload: RuntimeEvent) -> EventEnvelope {
        EventEnvelope::new(
            0,
            EventId::new("e"),
            SessionId::new("s"),
            None,
            Timestamp::ZERO,
            payload,
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
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
        // The simplest way to obtain a real prompt is to drive the policy the
        // runtime would call.
        let (policy, mut requests) = InteractiveApproval::new(1);
        let tool = tool.to_owned();
        tokio::spawn(async move {
            let request = ApprovalRequest {
                call_id: ToolCallId::new("c1"),
                tool,
                arguments,
                effects: ToolEffects::read_only().with_write("/repo"),
            };
            let _ = policy.decide(&request).await;
        });
        loop {
            if let Some(prompt) = requests.recv().await {
                return prompt;
            }
        }
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
    fn ctrl_c_quits_when_idle_but_confirms_when_busy() {
        let mut idle = app();
        assert_eq!(idle.on_key(ctrl('c')), Some(Action::Quit));
        assert!(idle.should_quit);

        let mut busy = app();
        busy.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(busy.on_key(ctrl('c')), None);
        assert!(matches!(busy.overlay, Some(Overlay::ExitConfirm { .. })));
        assert!(!busy.should_quit);
    }

    #[test]
    fn declining_the_exit_confirmation_leaves_work_running() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.on_key(ctrl('c'));
        assert_eq!(app.on_key(key(KeyCode::Char('n'))), None);
        assert!(app.overlay.is_none());
        assert!(!app.should_quit);
        assert!(app.is_busy());
    }

    #[test]
    fn a_second_ctrl_c_forces_the_exit() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(app.on_key(ctrl('c')), None);
        assert_eq!(app.on_key(ctrl('c')), Some(Action::Quit));
        assert!(app.should_quit);
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
    async fn cancelling_exit_restores_the_pending_approval() {
        let mut app = app();
        app.present_approval(prompt("shell").await);

        assert_eq!(app.on_key(ctrl('c')), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::ExitConfirm { approval: Some(_) })
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
                assert_eq!(protected_summary, "command · values protected");
            }
            other => panic!("expected a tool block, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_second_request_supersedes_rather_than_stacks() {
        let mut app = app();
        app.present_approval(prompt("shell").await);
        app.present_approval(prompt("patch").await);

        match &app.overlay {
            Some(Overlay::Approval { prompt, .. }) => assert_eq!(prompt.tool(), "patch"),
            other => panic!("expected the newer approval, got {other:?}"),
        }
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
        app.apply(&event(RuntimeEvent::TextDelta {
            text: "The retry ".into(),
        }));
        app.apply(&event(RuntimeEvent::TextDelta {
            text: "policy".into(),
        }));
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
        assert!(matches!(
            &app.transcript.blocks()[1],
            Block::Notice { source, text }
                if source == "turn" && text.starts_with("completed in ")
        ));
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
        app.apply(&event(RuntimeEvent::TextDelta {
            text: "partial".into(),
        }));
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
    fn a_completed_turn_freezes_its_monotonic_elapsed_time() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.turn_started_at = Instant::now().checked_sub(Duration::from_secs(65));
        assert!(
            app.turn_elapsed()
                .is_some_and(|elapsed| elapsed.as_secs() >= 65)
        );

        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        }));

        assert!(app.turn_elapsed().is_none());
        match app.transcript.blocks().last() {
            Some(Block::Notice { source, text }) => {
                assert_eq!(source, "turn");
                assert_eq!(text, "completed in 1m 05s");
            }
            other => panic!("expected a completed-duration notice, got {other:?}"),
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
        assert_eq!(plan.input_tokens, 2_000);
        assert_eq!(plan.input_budget_tokens, 10_000);
        assert_eq!(plan.reserved_tokens, 2_000);
        assert_eq!(plan.segment_count, 2);
        assert_eq!(plan.totals["history"], 1_500);
        assert_eq!(plan.render_footer(), "~80% ctx");
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
