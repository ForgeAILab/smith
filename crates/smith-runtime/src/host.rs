//! Standard Smith session composition shared by interactive and headless hosts.
//!
//! [`crate::factory`] maps resolved product policy onto Agent Runtime. This
//! module adds the host-owned lifecycle around that immutable runtime:
//! project-scoped paths, an optional snapshot store, a canonical event journal,
//! explicit create/resume identity, and ordered shutdown. It deliberately does
//! not render a terminal or choose an output format, so `smith` and `smith -p`
//! cannot drift in their persistence behavior.

use std::collections::{BTreeSet, VecDeque};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use agent_runtime::delegation::ChildDurability;
use agent_runtime::harness::{
    SEMANTIC_SUMMARY_COMPONENT_ID, SEMANTIC_SUMMARY_PURPOSE, protected_semantic_summary_from_state,
};
use agent_runtime::registry::Fingerprint;
use agent_runtime::runtime::{
    CheckpointRecoveryPolicy, GoalAdmissionGate, GoalController, GoalControllerConfig,
    SessionHandle, StartSession,
};
use agent_runtime_core::artifact::{
    ArtifactProvenance, ArtifactRef, ArtifactRetention, ArtifactSensitivity, ArtifactStore,
    ArtifactWrite,
};
use agent_runtime_core::cancel::CancelReason;
use agent_runtime_core::checkpoint::{TurnCheckpoint, TurnState};
use agent_runtime_core::clock::{Clock, SystemClock};
use agent_runtime_core::content::{ContentPart, Message};
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::event::EventEnvelope;
use agent_runtime_core::event::RuntimeEvent;
use agent_runtime_core::goal::{GoalCommand, GoalCommandResult, GoalProjection};
use agent_runtime_core::ids::{ChildId, InteractionRequestId, SessionId, ToolCallId, TurnId};
use agent_runtime_core::observer::EventObserver;
use agent_runtime_core::store::{SessionSnapshot, SessionStateSensitivity, SessionStore};
use agent_runtime_core::usage::{CounterKind, UsageDelta, UsageSource};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use smith_config::model::ApprovalMode;
use smith_config::resolve::{Layer, ResolvedConfig, Source};
use smith_tools::{ToolCallDisplay, project_tool_call_display};

use crate::artifact::SmithArtifactStore;
use crate::background_tasks::{BackgroundTaskRegistry, TaskStatus};
use crate::cache_controller::{
    CacheControllerConfig, CacheControllerResolvedInputs, CacheLifecycleController,
};
use crate::checkpoint::{
    CheckpointBarrier, CheckpointKeyProvider, ConfiguredCheckpointKeyProvider,
    CredentialCheckpointKeyProvider, SmithCheckpointSetup, with_resume_capsule,
};
use crate::delegation::{DelegationLifecycle, DelegationWaitPolicy};
use crate::factory::{FactoryError, RuntimeRequest, SmithRuntime};
use crate::journal::{
    DefaultRedactor, EphemeralWorkInterruption, EventJournal, JournalConfig, JournalRecord,
    JournalRecovery, JournalStats, Redactor, read_journal, reconcile_nonterminal_journal,
};
use crate::private_storage::{PrivateFileLock, try_acquire_private_lock};
use crate::project_instructions::discover as discover_project_instructions;
use crate::reasoning::{PersistedReasoningOverride, SESSION_STATE_NAMESPACE};
use crate::resume_capsule::{
    ArtifactProjection, MAX_ARTIFACTS, MAX_SERIALIZED_CAPSULE_BYTES, MAX_SUMMARY_BYTES,
    RESUME_CAPSULE_STATE_NAMESPACE, RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
    RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE, RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE,
    RESUME_SUMMARY_MEDIA_TYPE, RecoverySource, ResumeCapsule, ResumeCapsuleError,
    ResumeCapsuleSlot, SummaryCoverage, SummaryUsage, restore_runtime_summary_state,
    restore_summary_artifact,
};
use crate::session::{FileSessionStore, ProjectId, SessionListing, SessionPaths};

mod history;
mod observers;
mod persistence;
mod policy;
mod recovery;
mod startup;

use history::{
    append_shell_shortcut, redacted_result_text, tool_call_display_from_history,
    tool_call_displays_from_history, tool_result_text_from_history,
};
use observers::{ChangeTurnObserver, DeferredObserver, EventRing, JournalCheckpointBarrier};
#[cfg(test)]
use persistence::project_runtime_summary_persistence;
use policy::{reject_project_controlled_persistence, reject_project_granted_authority};
use recovery::{unresolved_ephemeral_work, wait_for_background_tasks_to_stop};

pub use policy::{list, mint_session_id, paths, project_id, validate_host_policy};
pub use startup::{start, start_with_modules};

#[cfg(test)]
mod tests;

/// A request to start one standard Smith-hosted session.
#[derive(Debug)]
pub struct HostSessionRequest {
    /// The already-resolved runtime request and injected host policy.
    pub runtime: RuntimeRequest,
    /// The canonical project root used to partition user session state.
    pub project_root: PathBuf,
    /// A prior session to resume. `None` creates a fresh identity.
    pub session_id: Option<SessionId>,
    /// Bounds for the canonical event journal.
    pub journal: JournalConfig,
    /// Protected-key provider. `None` selects the operating-system credential
    /// service; deterministic tests inject a provider so they never access the
    /// developer's keychain.
    pub checkpoint_keys: Option<Arc<dyn CheckpointKeyProvider>>,
    reasoning_reset_enabled: bool,
    reasoning_reset_effort: bool,
    reasoning_effort_shadowed: bool,
    context_window_reset: bool,
    context_window_shadowed: bool,
}

impl HostSessionRequest {
    /// Creates a request for a fresh session rooted at `project_root`.
    pub fn new(mut runtime: RuntimeRequest, project_root: impl Into<PathBuf>) -> Self {
        if runtime.background_services.is_none() {
            runtime.background_services = Some(crate::background_tasks::BackgroundServices::new());
        }
        // Semantic summarization is off unless a caller asks for it. Turning it
        // on with persistence meant every ordinary session carried a second
        // model route and an idle summarization budget for a projection almost
        // nothing read; structural compaction, which is what actually keeps a
        // long session inside its budget, is installed separately and is
        // unaffected.
        Self {
            runtime,
            project_root: project_root.into(),
            session_id: None,
            journal: JournalConfig::default(),
            checkpoint_keys: None,
            reasoning_reset_enabled: false,
            reasoning_reset_effort: false,
            reasoning_effort_shadowed: false,
            context_window_reset: false,
            context_window_shadowed: false,
        }
    }

    /// Resumes `session_id` instead of minting a fresh identity.
    #[must_use]
    pub fn resume(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// Uses an injected protected-key provider.
    #[must_use]
    pub fn checkpoint_keys(mut self, provider: Arc<dyn CheckpointKeyProvider>) -> Self {
        self.checkpoint_keys = Some(provider);
        self
    }

    /// Clears selected fields from a compatible persisted reasoning override.
    #[must_use]
    pub fn reasoning_reset(mut self, enabled: bool, effort: bool) -> Self {
        self.reasoning_reset_enabled = enabled;
        self.reasoning_reset_effort = effort;
        self
    }

    /// Suppresses a persisted effort for this run without discarding it.
    ///
    /// The distinction from [`Self::reasoning_reset`] is what the session
    /// keeps. A reset is the user saying "forget my saved effort", so it is
    /// forgotten. A shadow is a higher layer — an invocation flag — answering
    /// for this run only: the saved value is neither applied nor overwritten,
    /// so the next run without the flag resumes onto the session's own choice.
    #[must_use]
    pub fn reasoning_effort_shadowed(mut self, shadowed: bool) -> Self {
        self.reasoning_effort_shadowed = shadowed;
        self
    }

    /// Clears a persisted context-window override when the user chose default.
    #[must_use]
    pub fn context_window_reset(mut self, reset: bool) -> Self {
        self.context_window_reset = reset;
        self
    }

    /// Suppresses a persisted context window for this run without discarding it.
    #[must_use]
    pub fn context_window_shadowed(mut self, shadowed: bool) -> Self {
        self.context_window_shadowed = shadowed;
        self
    }
}

/// A running Smith session and the host resources that must shut down with it.
#[derive(Debug)]
pub struct HostSession {
    runtime: SmithRuntime,
    session: SessionHandle,
    image_history_registration: crate::image_history::SessionImageRegistration,
    client: crate::client::SmithSession,
    display_redactor: DefaultRedactor,
    journal: Option<Arc<EventJournal>>,
    paths: Option<SessionPaths>,
    snapshot_store: Option<Arc<RedactingSessionStore>>,
    shutdown_result: tokio::sync::Mutex<Option<Result<Option<JournalStats>, RuntimeError>>>,
    ring: Option<Arc<EventRing>>,
    changes: Arc<smith_tools::ChangeRecorder>,
    lifecycle_lease: Mutex<Option<PrivateFileLock>>,
    restored_interaction: Option<RestoredInteraction>,
    recovered_ephemeral_work: Option<EphemeralWorkInterruption>,
    goal_controller: Mutex<Option<GoalController>>,
    goal_admission_gate: Option<GoalAdmissionGate>,
    delegation_lifecycle: Mutex<Option<DelegationLifecycle>>,
    cache_controller: Mutex<Option<CacheLifecycleController>>,
    final_cache_lifecycle: Mutex<Option<crate::cache_controller::CacheControllerSnapshot>>,
    resume_capsule: Option<Arc<ResumeCapsuleSlot>>,
}

/// A finished user shell shortcut retained only for transcript display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedShellShortcut {
    /// Version of this sidecar record's shape.
    pub schema_version: u32,
    /// Canonical history length when the shortcut was dispatched.
    pub anchor: usize,
    /// Runtime call identity, absent when admission was rejected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call: Option<String>,
    /// Credential-redacted command echoed by the transcript.
    pub command: String,
    /// Whether the shortcut failed.
    pub is_error: bool,
    /// Credential-redacted bounded text retained by the live row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

/// Redaction-safe identity of an exact pending interaction restored from a
/// protected checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoredInteraction {
    request_id: InteractionRequestId,
    turn_id: TurnId,
    question_count: usize,
}

impl RestoredInteraction {
    /// Exact interaction request identity.
    pub fn request_id(&self) -> &InteractionRequestId {
        &self.request_id
    }

    /// Turn that owns the pending interaction.
    pub fn turn_id(&self) -> &TurnId {
        &self.turn_id
    }

    /// Number of questions, without exposing prompt or answer content.
    pub fn question_count(&self) -> usize {
        self.question_count
    }
}

impl HostSession {
    /// The shared Agent Runtime session handle.
    pub fn session(&self) -> &SessionHandle {
        &self.session
    }

    /// Session state with all reported advisor usage included, even when an
    /// interrupted tool-output phase bypassed Runtime's terminal hooks.
    pub fn snapshot(&self) -> SessionSnapshot {
        self.runtime.accounted_snapshot(&self.session)
    }

    /// Versioned Smith-owned client session used by presentation surfaces.
    pub fn client(&self) -> &crate::client::SmithSession {
        &self.client
    }

    /// The Smith runtime composition record.
    pub fn runtime(&self) -> &SmithRuntime {
        &self.runtime
    }

    /// The on-disk paths when persistence is enabled.
    pub fn paths(&self) -> Option<&SessionPaths> {
        self.paths.as_ref()
    }

    /// Appends a finished shortcut without changing canonical session state.
    /// The caller supplies the exact bounded result retained by its live row.
    pub fn record_shell_shortcut(
        &self,
        anchor: usize,
        call: Option<&str>,
        command: &str,
        is_error: bool,
        result: Option<&str>,
    ) {
        let Some(paths) = &self.paths else {
            return;
        };
        let path = match paths.shell(self.session.id()) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(%error, "shell shortcut path unavailable");
                return;
            }
        };
        let redact = |text: &str| match self
            .display_redactor
            .redacted_clone(&serde_json::Value::String(text.to_owned()))
        {
            serde_json::Value::String(redacted) => Some(redacted),
            _ => {
                tracing::warn!("shell shortcut redaction returned a non-string; record skipped");
                None
            }
        };
        let Some(command) = redact(command) else {
            return;
        };
        let result = match result {
            Some(text) => {
                let Some(redacted) = redact(text) else {
                    return;
                };
                Some(redacted)
            }
            None => None,
        };
        let record = SavedShellShortcut {
            schema_version: 1,
            anchor,
            call: call.map(str::to_owned),
            command,
            is_error,
            result,
        };
        if let Err(error) = append_shell_shortcut(&path, &record) {
            tracing::warn!(%error, "shell shortcut could not be saved");
        }
    }

    /// Reads complete, supported shortcut records in their sidecar file order.
    pub fn saved_shell_shortcuts(&self) -> Vec<SavedShellShortcut> {
        let Some(paths) = &self.paths else {
            return Vec::new();
        };
        let path = match paths.shell(self.session.id()) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(%error, "shell shortcut path unavailable");
                return Vec::new();
            }
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => {
                tracing::warn!(%error, "shell shortcuts could not be read");
                return Vec::new();
            }
        };
        bytes
            .split_inclusive(|byte| *byte == b'\n')
            .filter(|line| line.last() == Some(&b'\n'))
            .filter_map(|line| serde_json::from_slice::<SavedShellShortcut>(line).ok())
            .filter(|record| record.schema_version == 1)
            .collect()
    }

    /// In-session exact/ambiguous mutation attribution.
    pub fn changes(&self) -> &Arc<smith_tools::ChangeRecorder> {
        &self.changes
    }

    /// Background-task registry owned by this exact composed host.
    pub fn background_tasks(&self) -> &Arc<BackgroundTaskRegistry> {
        self.runtime
            .background_services()
            .expect("a standard HostSession always resolves background services")
            .registry()
    }

    /// Exact pending interaction restored from the protected checkpoint, when
    /// startup resumed before the host had accepted a response.
    pub fn restored_interaction(&self) -> Option<&RestoredInteraction> {
        self.restored_interaction.as_ref()
    }

    /// Process-owned work found unresolved and explicitly interrupted during
    /// this resume.
    pub fn recovered_ephemeral_work(&self) -> Option<&EphemeralWorkInterruption> {
        self.recovered_ephemeral_work.as_ref()
    }

    /// Current identity-only parent parking projection, when delegation is
    /// enabled for this root session.
    pub fn delegation_parking(&self) -> Option<crate::delegation::ParkingSnapshot> {
        self.delegation_lifecycle
            .lock()
            .expect("delegation lifecycle lock poisoned")
            .as_ref()
            .map(DelegationLifecycle::snapshot)
    }

    /// Current redaction-safe cold-continuation projection, when enabled.
    pub fn resume_capsule(&self) -> Option<crate::resume_capsule::RedactedResumeCapsule> {
        self.resume_capsule
            .as_ref()
            .map(|slot| slot.snapshot().redacted_projection())
    }

    /// Current redaction-safe adaptive cache controller projection.
    pub fn cache_lifecycle(&self) -> Option<crate::cache_controller::CacheControllerSnapshot> {
        let live = self
            .cache_controller
            .lock()
            .expect("cache controller lock poisoned")
            .as_ref()
            .map(CacheLifecycleController::snapshot);
        live.or_else(|| {
            self.final_cache_lifecycle
                .lock()
                .expect("final cache lifecycle lock poisoned")
                .clone()
        })
    }

    /// Current bounded persistent-goal projection for this eligible root
    /// session. Ephemeral and child sessions return `None`.
    pub fn goal(&self) -> Result<Option<GoalProjection>, RuntimeError> {
        self.runtime
            .goal_component()
            .map(|component| self.session.goal(component))
            .transpose()
            .map(Option::flatten)
    }

    /// Enables or defers idle-only goal continuation admission. Interactive
    /// pending input uses this narrow gate; it does not pause or interrupt an
    /// already-serving goal turn.
    pub fn set_goal_continuation_enabled(&self, enabled: bool) {
        if let Some(gate) = &self.goal_admission_gate {
            gate.set_enabled(enabled);
        }
    }

    /// Applies one typed local goal control through Agent Runtime's serialized
    /// canonical state path without provider I/O.
    pub async fn control_goal(
        &self,
        command: GoalCommand,
    ) -> Result<GoalCommandResult, RuntimeError> {
        let component = self.runtime.goal_component().ok_or_else(|| {
            RuntimeError::conflict("persistent goals require an eligible persisted root session")
        })?;
        self.session.control_goal(component, command).await
    }

    /// Flushes and returns the redaction-safe canonical events available for
    /// local timeline projection.
    ///
    /// A non-persistent session has no replayable timeline and returns an
    /// empty vector. The protected checkpoint is deliberately not consulted:
    /// local presentation must never reconstruct raw prepared arguments or
    /// sensitive interaction content.
    pub async fn timeline_events(&self) -> Result<Vec<EventEnvelope>, RuntimeError> {
        let (Some(journal), Some(paths)) = (&self.journal, &self.paths) else {
            return Ok(Vec::new());
        };
        journal.flush().await?;
        let path = paths.journal(self.session.id())?;
        let recovery = read_journal(path).await?;
        Ok(recovery.events().into_iter().cloned().collect())
    }

    /// Returns replayable events projected through the versioned Smith client
    /// protocol. Presentation clients should prefer this over the canonical
    /// journal vocabulary.
    pub async fn client_timeline_events(
        &self,
    ) -> Result<Vec<crate::client::SmithEvent>, RuntimeError> {
        Ok(self
            .timeline_events()
            .await?
            .iter()
            .map(crate::client::SmithEvent::project_or_unknown)
            .collect())
    }

    /// Returns the canonical redacted events with sequence numbers in
    /// `first..=last`.
    ///
    /// This is the healing path for a lagged live subscriber: the ring and
    /// the journal both observe every event synchronously before broadcast,
    /// so anything the stream skipped is already captured by one of them. The
    /// bounded in-memory ring — populated the same way as the journal, see
    /// [`EventRing`] — serves the common case without touching disk; a range
    /// it cannot cover (a resumed process, or a gap wider than
    /// [`EVENT_RING_CAPACITY`]) falls back to a flushed full journal read,
    /// exactly as before this ring existed. A non-persistent session, or a
    /// journal that dropped the range under its own backpressure, returns
    /// fewer events than the range names — the caller reports the remainder
    /// honestly rather than inventing it.
    pub async fn journal_events_between(
        &self,
        first: u64,
        last: u64,
    ) -> Result<Vec<EventEnvelope>, RuntimeError> {
        let (Some(journal), Some(paths)) = (&self.journal, &self.paths) else {
            return Ok(Vec::new());
        };
        let events = match self
            .ring
            .as_ref()
            .and_then(|ring| ring.events_between(first, last))
        {
            Some(events) => events
                .into_iter()
                .map(|event| self.redact_ring_event(event))
                .collect::<Result<Vec<_>, _>>()?,
            None => {
                // Only the fallback needs the disk at all: a presentation-only
                // replay does not need the durability an fsync buys, and this
                // is exactly the await that used to starve the subscriber and
                // the journal writer on every gap, producing the next one.
                journal.flush().await?;
                let path = paths.journal(self.session.id())?;
                let recovery = read_journal(path).await?;
                recovery
                    .events()
                    .into_iter()
                    .filter(|event| event.seq >= first && event.seq <= last)
                    .cloned()
                    .collect()
            }
        };
        let requested = last.saturating_sub(first).saturating_add(1);
        if (events.len() as u64) < requested {
            // The UI collapses a run of these into one line for the person
            // looking at the transcript; this is the full, ungrouped detail
            // for whoever has to find out why, in the log the TUI can never
            // corrupt by writing to stdout/stderr itself.
            tracing::warn!(
                session = %self.session.id(),
                first,
                last,
                requested,
                recovered = events.len(),
                "a live-stream gap could not be fully replayed from durable history; the \
                 missing events are permanently gone"
            );
        }
        Ok(events)
    }

    /// Returns a replay gap projected through the Smith client protocol.
    pub async fn client_events_between(
        &self,
        first: u64,
        last: u64,
    ) -> Result<Vec<crate::client::SmithEvent>, RuntimeError> {
        Ok(self
            .journal_events_between(first, last)
            .await?
            .iter()
            .map(crate::client::SmithEvent::project_or_unknown)
            .collect())
    }

    /// Applies the same credential redaction the journal writer applies
    /// before a record reaches disk, so an event served from the in-memory
    /// ring is exactly as safe to display as one read back from the journal
    /// file.
    ///
    /// The ring stores raw envelopes (see [`EventRing::observe`]) precisely
    /// so its hot path never pays for this; the cost lands here instead, on
    /// the rare gap-replay call rather than every event's emission.
    fn redact_ring_event(&self, event: EventEnvelope) -> Result<EventEnvelope, RuntimeError> {
        let value = serde_json::to_value(&event).map_err(|err| {
            RuntimeError::new(
                ErrorKind::Serialization,
                format!("a ring-served event could not be serialized for redaction: {err}"),
            )
        })?;
        let redacted = self.display_redactor.redacted_clone(&value);
        serde_json::from_value(redacted).map_err(|err| {
            RuntimeError::new(
                ErrorKind::Serialization,
                format!("a redacted ring event could not be parsed back: {err}"),
            )
        })
    }

    /// Resolves a protected live event to reviewed display metadata.
    ///
    /// Agent Runtime appends the canonical assistant tool call before emitting
    /// `ToolCallRequested`, so this lookup does not require raw arguments in
    /// the event or journal.
    pub fn tool_call_display(&self, call_id: &ToolCallId) -> Option<ToolCallDisplay> {
        // Borrow the history under its lock instead of cloning it: this runs
        // for every live tool event, and a deep clone of a long session here
        // is what let the broadcast stream lap the TUI subscriber.
        self.session.with_history(|history| {
            tool_call_display_from_history(history, call_id, &self.display_redactor)
        })
    }

    /// Reviewed display projections for every canonical built-in tool call.
    ///
    /// Used when rebuilding a local transcript from resumed history. Unknown
    /// tools and malformed calls remain on their honest fallback rows.
    pub fn tool_call_displays(&self) -> Vec<(ToolCallId, ToolCallDisplay)> {
        self.session.with_history(|history| {
            tool_call_displays_from_history(history, &self.display_redactor)
        })
    }

    /// Credential-redacted text of one canonical tool result.
    ///
    /// The protected event stream never carries result content; the client
    /// asks for it after `ToolCallCompleted` and bounds it before display.
    /// Results are model-visible text, so unlike arbitrary tool arguments
    /// they only need the literal-secret scrub before local presentation.
    pub fn tool_result_text(&self, call_id: &ToolCallId) -> Option<String> {
        self.session.with_history(|history| {
            tool_result_text_from_history(history, call_id, &self.display_redactor)
        })
    }

    /// Reviewed display metadata for one tool call a delegated child made.
    ///
    /// A child's lifecycle events carry identifiers only, exactly as the
    /// parent's own do, so the same resolution applies — only against the
    /// child's canonical history instead of this session's. `None` once the
    /// child is dormant: its history is no longer in this process, and the
    /// row keeps the honest fallback it was built with.
    pub fn child_tool_call_display(
        &self,
        child: &ChildId,
        call: &ToolCallId,
    ) -> Option<ToolCallDisplay> {
        self.with_child_history(child, |history| {
            tool_call_display_from_history(history, call, &self.display_redactor)
        })
        .flatten()
    }

    /// Credential-redacted text of one tool result a delegated child received.
    pub fn child_tool_result_text(&self, child: &ChildId, call: &ToolCallId) -> Option<String> {
        self.with_child_history(child, |history| {
            tool_result_text_from_history(history, call, &self.display_redactor)
        })
        .flatten()
    }

    fn with_child_history<R>(&self, child: &ChildId, f: impl FnOnce(&[Message]) -> R) -> Option<R> {
        self.runtime
            .delegation()
            .and_then(|delegation| delegation.coordinator())
            .and_then(|coordinator| coordinator.with_child_history(child, f))
    }

    /// Credential-redacted result text for every canonical tool call, used
    /// when rebuilding a local transcript from resumed history.
    pub fn tool_result_texts(&self) -> Vec<(ToolCallId, String)> {
        self.session.with_history(|history| {
            history
                .iter()
                .flat_map(|message| message.content.iter())
                .filter_map(|part| {
                    let ContentPart::ToolResult(result) = part else {
                        return None;
                    };
                    redacted_result_text(result, &self.display_redactor)
                        .map(|text| (result.call_id.clone(), text))
                })
                .collect()
        })
    }

    /// Stops host schedulers, performs Runtime's final save, closes snapshot
    /// writes, then drains and syncs the journal. Closing prevents detached
    /// Runtime watchers from recreating files after host cleanup.
    ///
    /// The journal is attempted even when snapshot persistence fails so a
    /// storage error cannot strand the writer task or silently lose events
    /// already accepted by its bounded queue.
    pub async fn shutdown(&self) -> Result<Option<JournalStats>, RuntimeError> {
        // Cleanup may follow a driver shutdown, and concurrent callers must
        // not close the store while another caller is still draining workers.
        let mut shutdown_result = self.shutdown_result.lock().await;
        if let Some(result) = shutdown_result.as_ref() {
            return result.clone();
        }
        let cache_controller = self
            .cache_controller
            .lock()
            .expect("cache controller lock poisoned")
            .take();
        if let Some(controller) = cache_controller.as_ref() {
            controller.stop_scheduling();
        }
        let delegation_lifecycle = self
            .delegation_lifecycle
            .lock()
            .expect("delegation lifecycle lock poisoned")
            .take();
        if let Some(lifecycle) = delegation_lifecycle {
            lifecycle.shutdown().await;
        }
        let goal_controller = self
            .goal_controller
            .lock()
            .expect("goal controller lock poisoned")
            .take();
        let goal_controller = match goal_controller {
            Some(controller) => controller.shutdown().await,
            None => Ok(()),
        };
        let delegation = match self
            .runtime
            .delegation()
            .and_then(|delegation| delegation.coordinator())
        {
            Some(coordinator) => coordinator.shutdown(CancelReason::Shutdown).await,
            None => Ok(()),
        };
        if let Some(controller) = cache_controller {
            controller.shutdown().await;
            *self
                .final_cache_lifecycle
                .lock()
                .expect("final cache lifecycle lock poisoned") = Some(controller.snapshot());
        }
        // Drain the controller before Runtime's terminal snapshot. An idle
        // summary accepted before shutdown may still finish optional capsule
        // projection while the worker drains; the final Runtime save must
        // include that projection before the snapshot store closes.
        let session = self.session.shutdown().await;
        // Runtime emits SessionShutdown before its final save. Its detached
        // delegation watcher can persist the catalog afterwards, so close the
        // host store before releasing ownership or allowing file cleanup.
        if let Some(store) = &self.snapshot_store {
            store.close().await;
        }
        self.image_history_registration.unregister();

        // Background tasks are session-owned, process-group work, not runtime
        // state: nothing else stops them. Signal every running task before
        // the journal closes, then wait — bounded, never for the task's own
        // duration — so each worker's kill and terminal journal marker have
        // a chance to land. A task still running past the bound is abandoned
        // to `kill_on_drop` rather than allowed to hold up exit.
        self.background_tasks()
            .stop_all_session_tasks(self.session.id(), TaskStatus::Shutdown);
        wait_for_background_tasks_to_stop(self.background_tasks(), self.session.id()).await;

        let journal = match &self.journal {
            Some(journal) => journal.shutdown().await.map(Some),
            None => Ok(None),
        };
        self.lifecycle_lease
            .lock()
            .expect("session lifecycle lease poisoned")
            .take();
        let result = goal_controller.and(delegation).and(session).and(journal);
        *shutdown_result = Some(result.clone());
        result
    }
}

/// A standard-session startup failure.
#[derive(Debug, thiserror::Error)]
pub enum HostSessionError {
    /// Runtime policy or provider composition failed.
    #[error(transparent)]
    Factory(#[from] FactoryError),
    /// A shared runtime or persistence operation failed.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    /// Resume was requested while persistence was disabled.
    #[error("session `{session}` cannot be resumed because persistence is disabled")]
    ResumeDisabled {
        /// The requested session.
        session: SessionId,
    },
    /// An explicit resume identity had no saved snapshot.
    #[error("session `{session}` does not exist for this project")]
    SessionNotFound {
        /// The requested session.
        session: SessionId,
    },
    /// Repository-controlled configuration attempted to grant execution
    /// authority merely by being opened.
    #[error(
        "{provenance} cannot grant tool execution authority; move `{setting}` to \
         user configuration or pass an explicit command-line policy"
    )]
    ProjectGrantedAuthority {
        /// The authority-bearing setting.
        setting: &'static str,
        /// The repository-controlled source that supplied it.
        provenance: Source,
    },
    /// Repository-controlled configuration attempted to redirect or weaken
    /// user-scoped session persistence.
    #[error(
        "{provenance} cannot control user-scoped persistence `{setting}`; move the \
         setting to user configuration or pass an explicit invocation policy"
    )]
    ProjectControlledPersistence {
        /// The persistence setting.
        setting: &'static str,
        /// The repository-controlled source that supplied it.
        provenance: Source,
    },
}

/// A snapshot adapter that applies the same redaction registry as the event
/// journal before bytes reach user state.
///
/// Agent Runtime keeps the live canonical snapshot unchanged for the current
/// turn. Persistence receives a clone with known credential literals removed,
/// so a provider reflecting its own authorization value cannot turn a clean
/// shutdown into a secret-bearing resume file.
#[derive(Debug, Clone)]
struct RedactingSessionStore {
    inner: Arc<FileSessionStore>,
    writes: Arc<tokio::sync::Mutex<bool>>,
    #[cfg(test)]
    save_pause: Arc<Mutex<Option<Arc<SavePause>>>>,
    redactor: DefaultRedactor,
    reasoning: PersistedReasoningOverride,
    resume_capsule: Option<Arc<ResumeCapsuleSlot>>,
    artifact_store: Option<Arc<dyn ArtifactStore>>,
    summary_provider: Option<String>,
}

struct OrdinarySummaryPersistence {
    model: String,
    revision: agent_runtime::registry::RegistryRevision,
    body: String,
    usage: SummaryUsage,
    artifact: ArtifactRef,
    coverage: Vec<SummaryCoverage>,
}

struct RuntimeSummaryPersistence {
    state_artifact: ArtifactRef,
    ordinary: Option<OrdinarySummaryPersistence>,
}

#[cfg(test)]
#[derive(Debug, Default)]
struct SavePause {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
