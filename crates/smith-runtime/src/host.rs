//! Standard Smith session composition shared by interactive and headless hosts.
//!
//! [`crate::factory`] maps resolved product policy onto Agent Runtime. This
//! module adds the host-owned lifecycle around that immutable runtime:
//! project-scoped paths, an optional snapshot store, a canonical event journal,
//! explicit create/resume identity, and ordered shutdown. It deliberately does
//! not render a terminal or choose an output format, so `smith` and `smith -p`
//! cannot drift in their persistence behavior.

use std::collections::{BTreeSet, VecDeque};
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
use crate::summary::SmithSemanticSummaryConfig;

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
}

impl HostSessionRequest {
    /// Creates a request for a fresh session rooted at `project_root`.
    pub fn new(mut runtime: RuntimeRequest, project_root: impl Into<PathBuf>) -> Self {
        if runtime.background_services.is_none() {
            runtime.background_services = Some(crate::background_tasks::BackgroundServices::new());
        }
        if runtime.config.persistence.enabled.value && runtime.semantic_summary.is_none() {
            runtime.semantic_summary = Some(SmithSemanticSummaryConfig::standard());
        }
        Self {
            runtime,
            project_root: project_root.into(),
            session_id: None,
            journal: JournalConfig::default(),
            checkpoint_keys: None,
            reasoning_reset_enabled: false,
            reasoning_reset_effort: false,
            reasoning_effort_shadowed: false,
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
}

/// A running Smith session and the host resources that must shut down with it.
#[derive(Debug)]
pub struct HostSession {
    runtime: SmithRuntime,
    session: SessionHandle,
    client: crate::client::SmithSession,
    display_redactor: DefaultRedactor,
    journal: Option<Arc<EventJournal>>,
    paths: Option<SessionPaths>,
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

    /// Stops host schedulers, performs Runtime's final save, then drains and
    /// syncs the journal.
    ///
    /// The journal is attempted even when snapshot persistence fails so a
    /// storage error cannot strand the writer task or silently lose events
    /// already accepted by its bounded queue.
    pub async fn shutdown(&self) -> Result<Option<JournalStats>, RuntimeError> {
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
        // projection while the worker drains; the final Runtime save must be
        // the last session-store write.
        let session = self.session.shutdown().await;

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
        goal_controller?;
        delegation?;
        session?;
        journal
    }
}

fn tool_call_display_from_history(
    history: &[Message],
    call_id: &ToolCallId,
    redactor: &DefaultRedactor,
) -> Option<ToolCallDisplay> {
    history.iter().rev().find_map(|message| {
        message.content.iter().rev().find_map(|part| {
            let ContentPart::ToolCall(call) = part else {
                return None;
            };
            (call.id == *call_id)
                .then(|| {
                    let arguments = redactor.redacted_clone(&call.arguments);
                    project_tool_call_display(&call.name, &arguments)
                })
                .flatten()
        })
    })
}

fn tool_result_text_from_history(
    history: &[Message],
    call_id: &ToolCallId,
    redactor: &DefaultRedactor,
) -> Option<String> {
    history.iter().rev().find_map(|message| {
        message.content.iter().rev().find_map(|part| {
            let ContentPart::ToolResult(result) = part else {
                return None;
            };
            (result.call_id == *call_id)
                .then(|| redacted_result_text(result, redactor))
                .flatten()
        })
    })
}

fn redacted_result_text(
    result: &agent_runtime_core::content::ToolResultBlock,
    redactor: &DefaultRedactor,
) -> Option<String> {
    let text = result
        .content
        .iter()
        .filter_map(ContentPart::as_text)
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        return None;
    }
    match redactor.redacted_clone(&serde_json::Value::String(text)) {
        serde_json::Value::String(redacted) => Some(redacted),
        _ => None,
    }
}

fn tool_call_displays_from_history(
    history: &[Message],
    redactor: &DefaultRedactor,
) -> Vec<(ToolCallId, ToolCallDisplay)> {
    history
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|part| {
            let ContentPart::ToolCall(call) = part else {
                return None;
            };
            let arguments = redactor.redacted_clone(&call.arguments);
            project_tool_call_display(&call.name, &arguments)
                .map(|display| (call.id.clone(), display))
        })
        .collect()
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

/// Starts a standard Smith session through the one runtime factory.
///
/// The journal observer is attached to the builder through a deferred seam,
/// then opened only after provider/runtime preflight succeeds and before the
/// first session event is emitted. A bad provider configuration therefore
/// cannot leave an empty session journal behind.
pub async fn start(mut request: HostSessionRequest) -> Result<HostSession, HostSessionError> {
    if request.runtime.system_prompt.is_none() && request.runtime.project_instructions.is_none() {
        request.runtime.project_instructions =
            discover_project_instructions(&request.project_root)?;
    }
    let mut config = request.runtime.config.clone();
    let summary_provider = request.runtime.semantic_summary.as_ref().map(|summary| {
        summary
            .provider
            .clone()
            .unwrap_or_else(|| config.provider.name.value.clone())
    });
    let surface = request.runtime.surface;
    reject_project_granted_authority(&config, &request.project_root)?;
    reject_project_controlled_persistence(&config, &request.project_root)?;
    let persistence = config.persistence.enabled.value;
    let session_id = request.session_id.clone().unwrap_or_else(mint_session_id);
    let host_clock: Arc<dyn Clock> = request
        .runtime
        .clock
        .clone()
        .unwrap_or_else(|| Arc::new(SystemClock));
    request.runtime.clock = Some(host_clock.clone());
    let resume_capsule = config
        .context
        .cache
        .resume_capsule
        .value
        .then(|| Arc::new(ResumeCapsuleSlot::new(session_id.clone(), host_clock.now())));

    if request.session_id.is_some() && !persistence {
        return Err(HostSessionError::ResumeDisabled {
            session: session_id,
        });
    }

    let persistence_redactor = request
        .runtime
        .persistence_redactor
        .clone()
        .unwrap_or_default();
    // The standard factory also registers provider credentials with this
    // shared redactor, even when durable session persistence is disabled.
    request.runtime.persistence_redactor = Some(persistence_redactor.clone());

    let mut resume_snapshot_exists = false;
    let (paths, journal_slot, checkpoint_barrier, ring) = if persistence {
        let paths = paths(&config, &request.project_root)?;
        if request.runtime.artifact_store.is_none() {
            request.runtime.artifact_store = Some(Arc::new(SmithArtifactStore::new(paths.clone())));
        }
        let inner = FileSessionStore::new(paths.clone());
        // A saved effort a higher layer answered for this run, kept so the
        // run's own selection cannot erase the session's choice on save.
        let mut shadowed_effort = None;
        if request.session_id.is_some() {
            let snapshot = inner.load(&session_id).await?;
            resume_snapshot_exists = snapshot.is_some();
            // Prime the persistence projection before Runtime startup so a
            // recovered non-terminal checkpoint cannot save an empty capsule
            // while the final recovery selection is still pending.  This is
            // only a write-safety baseline; the authoritative candidate
            // selection is repeated after start_session below.
            if let (Some(slot), Some(persisted)) = (
                resume_capsule.as_ref(),
                snapshot.as_ref().and_then(|snapshot| {
                    snapshot.extension_state.get(RESUME_CAPSULE_STATE_NAMESPACE)
                }),
            ) {
                slot.restore_versioned_state(persisted, RecoverySource::CanonicalSnapshot)
                    .map_err(|error| RuntimeError::conflict(error.to_string()))?;
            }
            if let Some(state) = snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.extension_state.get(SESSION_STATE_NAMESPACE))
            {
                let restored = PersistedReasoningOverride::restore(state)?;
                restored.apply(
                    &mut config,
                    request.reasoning_reset_enabled,
                    request.reasoning_reset_effort || request.reasoning_effort_shadowed,
                );
                if request.reasoning_effort_shadowed && !request.reasoning_reset_effort {
                    shadowed_effort = restored.effort.clone();
                }
                request.runtime.config = config.clone();
            }
        }
        let mut reasoning_override = PersistedReasoningOverride::from_config(&config);
        if reasoning_override.effort.is_none() {
            reasoning_override.effort = shadowed_effort;
        }
        let store = Arc::new(RedactingSessionStore::new(
            inner,
            persistence_redactor.clone(),
            reasoning_override,
            resume_capsule.clone(),
            request.runtime.artifact_store.clone(),
            summary_provider.clone(),
        ));
        request.runtime.session_store = Some(store);
        if let Some(store) = request.runtime.checkpoint_store.take() {
            request.runtime.checkpoint_store =
                Some(with_resume_capsule(store, resume_capsule.clone()));
        }
        if let Some(setup) = request.runtime.checkpoint_setup.take() {
            request.runtime.checkpoint_setup =
                Some(setup.with_resume_capsule(resume_capsule.clone()));
        }
        if request.runtime.checkpoint_store.is_none() && request.runtime.checkpoint_setup.is_none()
        {
            let provider = match request.checkpoint_keys.clone() {
                Some(provider) => Some(provider),
                None => {
                    if let Some(key) = &config.persistence.checkpoint_key {
                        Some(Arc::new(
                            ConfiguredCheckpointKeyProvider::new(&key.value)
                                .map_err(RuntimeError::from)?,
                        ) as Arc<dyn CheckpointKeyProvider>)
                    } else if let Some(reference) = &config.persistence.checkpoint_key_credential {
                        let resolver = request.runtime.credentials.clone().ok_or_else(|| {
                            RuntimeError::config(
                                "a checkpoint key credential reference requires a credential resolver",
                            )
                        })?;
                        Some(Arc::new(
                            CredentialCheckpointKeyProvider::new(resolver, &reference.value)
                                .map_err(RuntimeError::from)?,
                        ) as Arc<dyn CheckpointKeyProvider>)
                    } else {
                        None
                    }
                }
            };
            request.runtime.checkpoint_setup = Some(
                match provider {
                    Some(provider) => SmithCheckpointSetup::with_provider(paths.clone(), provider),
                    None => SmithCheckpointSetup::platform(paths.clone()),
                }
                .with_resume_capsule(resume_capsule.clone()),
            );
        }

        let (slot, barrier, ring) = if config.persistence.journal_events.value {
            let slot = Arc::new(DeferredObserver::default());
            request.runtime.observers.push(slot.clone());
            let barrier = Arc::new(JournalCheckpointBarrier::default());
            request.runtime.checkpoint_barrier =
                Some(barrier.clone() as Arc<dyn CheckpointBarrier>);
            // Unlike the journal, the ring has no later resource to bind: it
            // is its own complete observer from the moment it is built, so it
            // attaches directly instead of through the deferred slot above.
            let ring = Arc::new(EventRing::default());
            request.runtime.observers.push(ring.clone());
            (Some(slot), Some(barrier), Some(ring))
        } else {
            (None, None, None)
        };
        (Some(paths), slot, barrier, ring)
    } else {
        (None, None, None, None)
    };

    let change_journal = paths
        .as_ref()
        .map(|paths| paths.changes(&session_id))
        .transpose()?;
    let changes = Arc::new(smith_tools::ChangeRecorder::new(change_journal));
    request.runtime.change_recorder = Some(changes.clone());
    request
        .runtime
        .observers
        .push(Arc::new(ChangeTurnObserver(changes.clone())));

    let harness = crate::harness::resolve(crate::harness::HarnessSpec::trusted(request.runtime))
        .map_err(FactoryError::from)?;
    let runtime = crate::factory::build(harness).await?;

    // Probe existence before creating the lifecycle lock file. The reads are
    // atomic and side-effect free; an arbitrary missing resume id must not
    // leave a user-state directory or lock artifact behind.
    let checkpoint_probe = if request.session_id.is_some() {
        match runtime.checkpoint_store() {
            Some(store) => store.load_latest(&session_id).await?,
            None => None,
        }
    } else {
        None
    };
    if request.session_id.is_some() && !resume_snapshot_exists && checkpoint_probe.is_none() {
        return Err(HostSessionError::SessionNotFound {
            session: session_id,
        });
    }

    let lifecycle_lease = match &paths {
        Some(paths) => {
            let path = paths.lifecycle_lock(&session_id)?;
            Some(try_acquire_private_lock(&path).await?)
        }
        None => None,
    };

    // The prior owner may have advanced after the existence probe but before
    // releasing its lifecycle lease. Reload both durable records under our
    // lease and reconcile only against this fresh checkpoint watermark.
    let checkpoint = if request.session_id.is_some() {
        resume_snapshot_exists = match &paths {
            Some(paths) => FileSessionStore::new(paths.clone())
                .load(&session_id)
                .await?
                .is_some(),
            None => false,
        };
        let checkpoint = match runtime.checkpoint_store() {
            Some(store) => store.load_latest(&session_id).await?,
            None => None,
        };
        if !resume_snapshot_exists && checkpoint.is_none() {
            return Err(HostSessionError::SessionNotFound {
                session: session_id,
            });
        }
        if let (Some(slot), Some(persisted)) = (
            resume_capsule.as_ref(),
            checkpoint.as_ref().and_then(|checkpoint| {
                checkpoint
                    .snapshot
                    .extension_state
                    .get(RESUME_CAPSULE_STATE_NAMESPACE)
            }),
        ) {
            slot.restore_versioned_state(persisted, RecoverySource::ProtectedCheckpoint)
                .map_err(|error| RuntimeError::conflict(error.to_string()))?;
        }
        checkpoint
    } else {
        None
    };

    let mut resume_identity_floor = None;
    let mut recovered_ephemeral_work = None;
    let restored_interaction = checkpoint.as_ref().and_then(|checkpoint| {
        if let TurnState::AwaitingInteraction {
            request,
            response: None,
            ..
        } = &checkpoint.state
        {
            Some(RestoredInteraction {
                request_id: request.id().clone(),
                turn_id: checkpoint.turn.clone(),
                question_count: request.questionnaire_payload().questions().len(),
            })
        } else {
            None
        }
    });
    let journal = match (&paths, journal_slot, checkpoint_barrier) {
        (Some(paths), Some(slot), Some(barrier)) => {
            let journal_path = paths.journal(&session_id)?;
            if request.session_id.is_some() {
                let recovery = read_journal(&journal_path).await?;
                recovered_ephemeral_work = unresolved_ephemeral_work(&recovery);
                match checkpoint.as_ref() {
                    Some(checkpoint) if !matches!(checkpoint.state, TurnState::Terminal { .. }) => {
                        let reconciled = reconcile_nonterminal_journal(
                            &journal_path,
                            checkpoint.watermark.event_sequence,
                        )
                        .await?;
                        if reconciled.retained_gap {
                            tracing::warn!(
                                session = %session_id,
                                "the retained journal prefix contains an explicit gap; exact presentation replay is unavailable"
                            );
                        }
                        resume_identity_floor = Some(reconciled.identity_floor);
                        if reconciled.truncated_records > 0 {
                            tracing::info!(
                                session = %session_id,
                                records = reconciled.truncated_records,
                                boundary = checkpoint.watermark.event_sequence,
                                "discarded presentation-only journal tail before checkpoint resume"
                            );
                        }
                    }
                    _ => {
                        if recovery.records.iter().any(|line| {
                            matches!(
                                line.record,
                                crate::journal::JournalRecord::Dropped { .. }
                                    | crate::journal::JournalRecord::Oversized { .. }
                            )
                        }) {
                            tracing::warn!(
                                session = %session_id,
                                "the journal contains an explicit gap; exact presentation replay is unavailable"
                            );
                        }
                        resume_identity_floor = Some(recovery.identity_floor());
                    }
                };
            }
            let journal = Arc::new(
                EventJournal::for_session(
                    paths,
                    &session_id,
                    request.journal,
                    Arc::new(persistence_redactor.clone()),
                )
                .await?,
            );
            barrier.install(journal.clone())?;
            slot.install(journal.clone())?;
            Some(journal)
        }
        (None, None, None) | (Some(_), None, None) => None,
        _ => {
            return Err(RuntimeError::config(
                "journal observer, checkpoint barrier, and session paths must be configured together",
            )
            .into());
        }
    };

    let mut start = StartSession::new().with_id(session_id.clone());
    if let Some(floor) = resume_identity_floor {
        start = start.with_resume_identity_floor(floor);
    }
    if surface == crate::factory::HostSurface::Headless && restored_interaction.is_some() {
        start = start.with_checkpoint_recovery(CheckpointRecoveryPolicy::DeferPendingInteraction);
    }
    let session = match runtime.runtime().start_session(start).await {
        Ok(session) => session,
        Err(error) => {
            if let Some(journal) = &journal {
                let _ = journal.shutdown().await;
            }
            return Err(error.into());
        }
    };

    // Agent Runtime has now loaded its canonical and protected startup
    // candidates through the wrapped stores.  Select the capsule candidates
    // only after that boundary, then restore the optional protected summary
    // body and apply cold-process reconciliation last.  In particular, the
    // RedactingSessionStore load above must never run after cold_resume and
    // restore a pre-cold canonical projection back into the live slot.
    if request.session_id.is_some() {
        let canonical = match &paths {
            Some(paths) => {
                FileSessionStore::new(paths.clone())
                    .load(&session_id)
                    .await?
            }
            None => None,
        };
        let protected = match runtime.checkpoint_store() {
            Some(store) => store.load_latest(&session_id).await?,
            None => None,
        };
        if let Some(slot) = &resume_capsule {
            if let Some(persisted) = canonical
                .as_ref()
                .and_then(|snapshot| snapshot.extension_state.get(RESUME_CAPSULE_STATE_NAMESPACE))
            {
                slot.restore_versioned_state(persisted, RecoverySource::CanonicalSnapshot)
                    .map_err(|error| RuntimeError::conflict(error.to_string()))?;
            }
            if let Some(persisted) = protected.as_ref().and_then(|checkpoint| {
                checkpoint
                    .snapshot
                    .extension_state
                    .get(RESUME_CAPSULE_STATE_NAMESPACE)
            }) {
                slot.restore_versioned_state(persisted, RecoverySource::ProtectedCheckpoint)
                    .map_err(|error| RuntimeError::conflict(error.to_string()))?;
            }
            if let Some(store) = runtime.artifact_store() {
                if let Some(summary_state) = restore_runtime_summary_state(slot, store.as_ref())
                    .await
                    .map_err(|error| RuntimeError::conflict(error.to_string()))?
                    && session
                        .restore_semantic_summary_if_absent(summary_state)
                        .is_err()
                {
                    // Summary state is optional recovery acceleration. A
                    // changed route, incompatible revision, or stale source
                    // prefix must never block canonical cold continuation.
                    slot.update(|capsule| capsule.latest_summary_state_artifact = None);
                }
                restore_summary_artifact(slot, store.as_ref())
                    .await
                    .map_err(|error| RuntimeError::conflict(error.to_string()))?;
            }
            let _ = slot.cold_resume();
        }
    }

    // Root sessions get their delegation coordinator now that the session
    // exists: the `agent` tool starts answering, and completed child results
    // are routed into the session's safe-boundary inbox.
    let mut delegation_lifecycle = None;
    if let Some(delegation) = runtime.delegation() {
        let wait_policy = DelegationWaitPolicy::new(
            config.child_agents.wait_default_timeout_ms.value,
            config.child_agents.wait_max_timeout_ms.value,
        )?;
        delegation_lifecycle = Some(
            crate::delegation::wire_delegation_with_wait_policy(&session, delegation, wait_policy)
                .await?,
        );
        let durable_children = delegation
            .coordinator()
            .expect("a successfully wired delegation has a coordinator")
            .list()
            .into_iter()
            .filter(|status| status.durability == ChildDurability::Durable)
            .map(|status| status.child)
            .collect::<BTreeSet<_>>();
        if let Some(interruption) = &mut recovered_ephemeral_work {
            interruption
                .children
                .retain(|child| !durable_children.contains(child));
            if interruption.is_empty() {
                recovered_ephemeral_work = None;
            }
        }
    }
    if let (Some(journal), Some(interruption)) = (&journal, recovered_ephemeral_work.clone()) {
        journal.record_ephemeral_interruption(interruption).await?;
    }

    // Every session — persisted or not — gets a background-task context so a
    // `run_in_background` shell call always has somewhere to notify and spool
    // to. Without persistence there is no session directory to spool under,
    // so a process-scoped temp directory stands in; it is still cleaned up by
    // shutdown killing every task before the process exits.
    let task_spool_dir = match &paths {
        Some(paths) => paths.tasks_dir(session.id())?,
        None => std::env::temp_dir().join(format!("smith-tasks-{}", session.id())),
    };
    runtime
        .background_services()
        .expect("a standard HostSession always resolves background services")
        .registry()
        .register_session_context(
            session.id(),
            Some(session.clone()),
            journal.clone(),
            task_spool_dir,
        );

    let goal_admission_gate = runtime
        .goal_component()
        .map(|_| GoalAdmissionGate::new(true));
    let goal_controller = runtime
        .goal_component()
        .zip(goal_admission_gate.clone())
        .map(|(component, admission_gate)| {
            session.start_goal_controller(
                (**component).clone(),
                GoalControllerConfig::new(
                    "Continue the current persistent goal from its canonical state. Stop only by completing it or recording a genuine blocker.",
                )
                .with_sensitivity(agent_runtime_core::content::InternalTurnSensitivity::Public)
                .with_admission_gate(admission_gate),
            )
        })
        .transpose()?;

    let cache_config = CacheControllerConfig::from_resolved(
        &config.context.cache,
        CacheControllerResolvedInputs {
            synthetic_spend: config.synthetic_cache_spend,
            contract: runtime.policy().model_profile.capabilities.cache_contract(),
            model_input_limit: runtime.policy().model_profile.limits.max_input_tokens,
            model_output_limit: runtime.policy().model_profile.limits.max_output_tokens,
            provider: runtime.policy().provider_name.clone(),
            model: runtime.policy().model.as_str().to_owned(),
            endpoint_identity: runtime.policy().cache_endpoint_identity.clone(),
            profile_identity: runtime.policy().model_profile.fingerprint(),
            semantic_summary_provider: runtime
                .policy()
                .semantic_summary
                .as_ref()
                .map(|summary| summary.provider.clone()),
            semantic_summary_model: runtime
                .policy()
                .semantic_summary
                .as_ref()
                .map(|summary| summary.model.clone()),
            attempt_marker_available: resume_capsule.is_some(),
        },
    )
    .map_err(RuntimeError::config)?;
    let parking_monitor = delegation_lifecycle
        .as_ref()
        .map(DelegationLifecycle::monitor);
    let cache_controller = CacheLifecycleController::start(
        session.clone(),
        cache_config,
        host_clock,
        parking_monitor,
        resume_capsule.clone(),
        runtime.artifact_store().cloned(),
        changes.clone(),
    );

    let client = crate::client::SmithSession::new(session.clone());
    Ok(HostSession {
        runtime,
        session,
        client,
        display_redactor: persistence_redactor,
        journal,
        paths,
        ring,
        changes,
        lifecycle_lease: Mutex::new(lifecycle_lease),
        restored_interaction,
        recovered_ephemeral_work,
        goal_controller: Mutex::new(goal_controller),
        goal_admission_gate,
        delegation_lifecycle: Mutex::new(delegation_lifecycle),
        cache_controller: Mutex::new(Some(cache_controller)),
        final_cache_lifecycle: Mutex::new(None),
        resume_capsule,
    })
}

/// Finds process-owned child and monitor work whose latest journal lifecycle
/// has no terminal resolution. Recovery markers participate so a later resume
/// never reports the same interrupted work twice.
///
/// Monitor start/stop records are metadata-only lifecycle seams. This scanner
/// never creates or restarts monitor execution.
fn unresolved_ephemeral_work(recovery: &JournalRecovery) -> Option<EphemeralWorkInterruption> {
    let mut children = BTreeSet::<ChildId>::new();
    let mut monitors = BTreeSet::<String>::new();
    let mut tasks = BTreeSet::<String>::new();
    for line in &recovery.records {
        match &line.record {
            JournalRecord::Event { event } => match &event.payload {
                RuntimeEvent::ChildSpawned { child, .. } => {
                    children.insert(child.clone());
                }
                // Completed and needs-input children remain live coordinator
                // entries that can accept a follow-up. Their in-memory state
                // is lost across process exit, so only truly terminal
                // lifecycle events resolve ephemeral work.
                RuntimeEvent::ChildStopped { child, .. }
                | RuntimeEvent::ChildFailed { child, .. } => {
                    children.remove(child);
                }
                _ => {}
            },
            JournalRecord::EphemeralWorkInterrupted { interruption } => {
                for child in &interruption.children {
                    children.remove(child);
                }
                for monitor in &interruption.monitors {
                    monitors.remove(monitor);
                }
                for task in &interruption.tasks {
                    tasks.remove(task);
                }
            }
            JournalRecord::MonitorStarted { monitor } => {
                monitors.insert(monitor.clone());
            }
            JournalRecord::MonitorStopped { monitor } => {
                monitors.remove(monitor);
            }
            JournalRecord::TaskStarted { task } => {
                tasks.insert(task.clone());
            }
            JournalRecord::TaskExited { task } => {
                tasks.remove(task);
            }
            JournalRecord::Oversized { .. } | JournalRecord::Dropped { .. } => {}
        }
    }
    let interruption = EphemeralWorkInterruption::process_exit(children, monitors, tasks);
    (!interruption.is_empty()).then_some(interruption)
}

/// Connects the checkpoint wrapper built during factory preflight to the
/// journal opened immediately before session start.
#[derive(Debug, Default)]
struct JournalCheckpointBarrier {
    journal: RwLock<Option<Arc<EventJournal>>>,
}

impl JournalCheckpointBarrier {
    fn install(&self, journal: Arc<EventJournal>) -> Result<(), RuntimeError> {
        let mut target = self
            .journal
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if target.is_some() {
            return Err(RuntimeError::conflict(
                "checkpoint journal barrier was already installed",
            ));
        }
        *target = Some(journal);
        Ok(())
    }
}

#[async_trait]
impl CheckpointBarrier for JournalCheckpointBarrier {
    async fn before_checkpoint(&self, checkpoint: &TurnCheckpoint) -> Result<(), RuntimeError> {
        let journal = self
            .journal
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(|| {
                RuntimeError::internal(
                    "checkpoint journal barrier is unavailable before session start",
                )
            })?;
        journal
            .flush_before(checkpoint.watermark.event_sequence)
            .await
    }
}

#[derive(Debug)]
struct ChangeTurnObserver(Arc<smith_tools::ChangeRecorder>);

impl EventObserver for ChangeTurnObserver {
    fn observe(&self, event: &EventEnvelope) {
        match event.payload {
            RuntimeEvent::TurnStarted => self.0.start_turn(),
            RuntimeEvent::TurnCompleted { .. } => {
                let _ = self.0.finish_turn();
            }
            _ => {}
        }
    }
}

/// A snapshot adapter that applies the same redaction registry as the event
/// journal before bytes reach user state.
///
/// Agent Runtime keeps the live canonical snapshot unchanged for the current
/// turn. Persistence receives a clone with known credential literals removed,
/// so a provider reflecting its own authorization value cannot turn a clean
/// shutdown into a secret-bearing resume file.
#[derive(Debug)]
struct RedactingSessionStore {
    inner: FileSessionStore,
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

fn project_runtime_summary_persistence(
    capsule: &mut ResumeCapsule,
    persistence: RuntimeSummaryPersistence,
    summary_provider: Option<&str>,
    updated: agent_runtime_core::clock::Timestamp,
) -> Result<(), ResumeCapsuleError> {
    let runtime_state_unchanged = capsule
        .latest_summary_state_artifact
        .as_ref()
        .is_some_and(|current| current == &persistence.state_artifact);
    capsule.attach_runtime_summary_state_artifact(persistence.state_artifact)?;

    // Runtime's canonical snapshot can still contain the successful ordinary
    // summary that preceded a Smith handoff or a later failed idle attempt.
    // An identical protected-state artifact is exact evidence that Runtime
    // has not committed a newer summary, so retain the newer Smith projection
    // regardless of its purpose/outcome. A different state artifact is a
    // genuinely newer Runtime summary and replaces it below.
    if let Some(ordinary) = persistence.ordinary
        && !runtime_state_unchanged
    {
        let provider = summary_provider.ok_or(ResumeCapsuleError::InvalidSerializedForm)?;
        capsule.record_ordinary_summary(
            provider,
            ordinary.model,
            ordinary.revision,
            updated,
            ordinary.body,
            ordinary.coverage,
        )?;
        if let Some(summary) = capsule.semantic_summary.as_mut() {
            summary.provenance.usage = ordinary.usage;
        }
        let artifact_id = ordinary.artifact.id.to_string();
        capsule.attach_summary_artifact(ordinary.artifact.clone())?;
        if capsule.exact_state.artifacts.len() < MAX_ARTIFACTS
            && !capsule
                .exact_state
                .artifacts
                .iter()
                .any(|artifact| artifact.artifact == artifact_id)
        {
            capsule.exact_state.artifacts.push(ArtifactProjection {
                artifact: artifact_id,
                digest: Some(Fingerprint::of(ordinary.artifact.digest.hex.as_bytes())),
            });
        }
    }
    Ok(())
}

impl RedactingSessionStore {
    fn new(
        inner: FileSessionStore,
        redactor: DefaultRedactor,
        reasoning: PersistedReasoningOverride,
        resume_capsule: Option<Arc<ResumeCapsuleSlot>>,
        artifact_store: Option<Arc<dyn ArtifactStore>>,
        summary_provider: Option<String>,
    ) -> Self {
        Self {
            inner,
            redactor,
            reasoning,
            resume_capsule,
            artifact_store,
            summary_provider,
        }
    }

    /// Copies the latest Sensitive Runtime summary extension into an
    /// owner-authorized artifact. Smith's ordinary JSON snapshot intentionally
    /// drops this namespace; the capsule keeps only the protected reference so
    /// a later host can hand the exact state back to Runtime.
    async fn persist_runtime_summary_state(
        &self,
        snapshot: &SessionSnapshot,
    ) -> Option<RuntimeSummaryPersistence> {
        let capsule = self.resume_capsule.as_ref()?;
        if snapshot.id != capsule.snapshot().session_id {
            return None;
        }
        let state = snapshot
            .extension_state
            .get(SEMANTIC_SUMMARY_COMPONENT_ID)?;
        if state.sensitivity != SessionStateSensitivity::Sensitive {
            return None;
        }
        let mut summary = protected_semantic_summary_from_state(state, UsageDelta::new()).ok()?;
        let summary_usage = snapshot
            .usage
            .records()
            .iter()
            .rev()
            .find(|record| {
                record.source == UsageSource::SemanticSummary
                    && record.provenance.purpose.as_deref() == Some(summary.purpose.as_str())
            })
            .map(|record| record.delta.clone())
            .unwrap_or_default();
        summary.usage = summary_usage.clone();
        if summary.source_artifact.provenance.session != snapshot.id {
            return None;
        }
        let bytes = serde_json::to_vec(state).ok()?;
        if bytes.is_empty() || bytes.len() > MAX_SERIALIZED_CAPSULE_BYTES {
            return None;
        }
        let artifacts = self.artifact_store.as_ref()?;
        let idempotency_key = Fingerprint::of(&bytes).as_str().to_owned();
        let state_artifact = artifacts
            .put(ArtifactWrite {
                bytes,
                media_type: RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE.to_owned(),
                sensitivity: ArtifactSensitivity::Sensitive,
                retention: ArtifactRetention::Session,
                provenance: ArtifactProvenance::new(
                    snapshot.id.clone(),
                    RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE,
                ),
                idempotency_key,
            })
            .await
            .ok()?;
        if state_artifact.provenance.session != snapshot.id
            || state_artifact.provenance.purpose != RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE
            || state_artifact.media_type != RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE
            || state_artifact.byte_length == 0
            || state_artifact.byte_length > MAX_SERIALIZED_CAPSULE_BYTES as u64
        {
            return None;
        }

        let ordinary = if summary.purpose == SEMANTIC_SUMMARY_PURPOSE {
            let body = summary.body.as_str();
            if body.is_empty() || body.len() > MAX_SUMMARY_BYTES {
                None
            } else {
                let artifact = artifacts
                    .put(ArtifactWrite {
                        bytes: body.as_bytes().to_vec(),
                        media_type: RESUME_SUMMARY_MEDIA_TYPE.to_owned(),
                        sensitivity: summary.source_artifact.sensitivity,
                        retention: ArtifactRetention::Session,
                        provenance: ArtifactProvenance::new(
                            snapshot.id.clone(),
                            RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
                        ),
                        idempotency_key: summary.summary_revision.as_str().to_owned(),
                    })
                    .await
                    .ok();
                artifact.and_then(|artifact| {
                    (artifact.provenance.session == snapshot.id
                        && artifact.provenance.purpose == RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE
                        && artifact.media_type == RESUME_SUMMARY_MEDIA_TYPE
                        && artifact.byte_length > 0
                        && artifact.byte_length <= MAX_SUMMARY_BYTES as u64)
                        .then(|| OrdinarySummaryPersistence {
                            model: summary.model_id,
                            revision: summary.summary_revision,
                            body: body.to_owned(),
                            usage: summary_usage_projection(&summary_usage),
                            artifact,
                            coverage: vec![SummaryCoverage::new(
                                "canonical_history",
                                0,
                                summary.omit_prefix as u64,
                            )],
                        })
                })
            }
        } else {
            None
        };
        Some(RuntimeSummaryPersistence {
            state_artifact,
            ordinary,
        })
    }
}

fn summary_usage_projection(usage: &UsageDelta) -> SummaryUsage {
    SummaryUsage {
        input_uncached: usage.get(CounterKind::InputUncached),
        input_cached: usage.get(CounterKind::InputCached),
        cache_write: usage.get(CounterKind::CacheWrite),
        output: usage.get(CounterKind::Output),
        reasoning: usage.get(CounterKind::Reasoning),
        cost_micro_usd: None,
        cost_is_estimate: false,
    }
}

#[async_trait]
impl SessionStore for RedactingSessionStore {
    async fn load(&self, id: &SessionId) -> Result<Option<SessionSnapshot>, RuntimeError> {
        // Capsule recovery is deliberately host-ordered after Runtime startup
        // has loaded canonical and protected state.  Loading must remain a
        // pure store operation here: mutating the slot would allow this call
        // to overwrite a cold-resumed projection during start_session.
        self.inner.load(id).await
    }

    async fn save(&self, snapshot: &SessionSnapshot) -> Result<(), RuntimeError> {
        let mut snapshot = snapshot.clone();
        if self.reasoning.is_empty() {
            snapshot.extension_state.remove(SESSION_STATE_NAMESPACE);
        } else {
            snapshot.extension_state.insert(
                SESSION_STATE_NAMESPACE.to_owned(),
                self.reasoning.versioned()?,
            );
        }
        let prepared_capsule = if let Some(slot) = &self.resume_capsule {
            // Protected artifact I/O happens before the atomic live-slot
            // projection, so a concurrent canonical event cannot be captured
            // in the rollback baseline and then erased by a failed save.
            let persistence = self.persist_runtime_summary_state(&snapshot).await;
            let projection = persistence.and_then(|persistence| {
                slot.try_update_atomic(|capsule| {
                    project_runtime_summary_persistence(
                        capsule,
                        persistence,
                        self.summary_provider.as_deref(),
                        snapshot.updated,
                    )
                })
                .ok()
            });
            let (prepared, state) = match slot.prepare_versioned_state(snapshot.updated) {
                Ok(prepared) => prepared,
                Err(error) => {
                    if let Some((previous, expected)) = projection {
                        let _ = slot.restore_if_current(&expected, previous);
                    }
                    return Err(RuntimeError::conflict(error.to_string()));
                }
            };
            snapshot
                .extension_state
                .insert(RESUME_CAPSULE_STATE_NAMESPACE.to_owned(), state);
            Some((slot, prepared, projection))
        } else {
            None
        };
        let mut value = match serde_json::to_value(&snapshot) {
            Ok(value) => value,
            Err(error) => {
                if let Some((slot, _, Some((previous, expected)))) = prepared_capsule.as_ref() {
                    let _ = slot.restore_if_current(expected, previous.clone());
                }
                return Err(RuntimeError::new(
                    ErrorKind::Serialization,
                    format!(
                        "session `{}` could not be prepared for redaction: {error}",
                        snapshot.id
                    ),
                ));
            }
        };
        self.redactor.redact(&mut value);
        let redacted = match serde_json::from_value(value) {
            Ok(redacted) => redacted,
            Err(error) => {
                if let Some((slot, _, Some((previous, expected)))) = prepared_capsule.as_ref() {
                    let _ = slot.restore_if_current(expected, previous.clone());
                }
                return Err(RuntimeError::new(
                    ErrorKind::Serialization,
                    format!(
                        "session `{}` could not be restored after redaction: {error}",
                        snapshot.id
                    ),
                ));
            }
        };
        let result = self.inner.save(&redacted).await;
        if let Some((slot, prepared, projection)) = prepared_capsule {
            if result.is_ok() {
                let _ = slot.commit_persisted(&prepared);
            } else if let Some((previous, expected)) = projection {
                let _ = slot.restore_if_current(&expected, previous);
            }
        }
        result
    }
}

fn reject_project_granted_authority(
    config: &ResolvedConfig,
    project_root: &Path,
) -> Result<(), HostSessionError> {
    if config.approval.mode.value == ApprovalMode::AllowAll
        && controlled_by_project(&config.approval.mode.source, project_root)
    {
        return Err(HostSessionError::ProjectGrantedAuthority {
            setting: "approval.mode",
            provenance: config.approval.mode.source.clone(),
        });
    }
    if let Some(auto_approve) = &config.approval.auto_approve
        && !auto_approve.value.is_empty()
        && controlled_by_project(&auto_approve.source, project_root)
    {
        return Err(HostSessionError::ProjectGrantedAuthority {
            setting: "approval.auto_approve",
            provenance: auto_approve.source.clone(),
        });
    }
    if let Some(rule) = config
        .approval
        .auto
        .iter()
        .find(|rule| controlled_by_project(&rule.source, project_root))
    {
        return Err(HostSessionError::ProjectGrantedAuthority {
            setting: "approval.auto",
            provenance: rule.source.clone(),
        });
    }
    Ok(())
}

fn reject_project_controlled_persistence(
    config: &ResolvedConfig,
    project_root: &Path,
) -> Result<(), HostSessionError> {
    for (setting, source) in [
        ("persistence.enabled", &config.persistence.enabled.source),
        (
            "persistence.sessions_dir",
            &config.persistence.sessions_dir.source,
        ),
        (
            "persistence.journal_events",
            &config.persistence.journal_events.source,
        ),
    ] {
        if controlled_by_project(source, project_root) {
            return Err(HostSessionError::ProjectControlledPersistence {
                setting,
                provenance: source.clone(),
            });
        }
    }
    for (setting, value) in [
        (
            "persistence.checkpoint_key",
            config
                .persistence
                .checkpoint_key
                .as_ref()
                .map(|key| &key.source),
        ),
        (
            "persistence.checkpoint_key_credential",
            config
                .persistence
                .checkpoint_key_credential
                .as_ref()
                .map(|credential| &credential.source),
        ),
    ] {
        if let Some(source) = value
            && controlled_by_project(source, project_root)
        {
            return Err(HostSessionError::ProjectControlledPersistence {
                setting,
                provenance: source.clone(),
            });
        }
    }
    Ok(())
}

fn controlled_by_project(source: &Source, project_root: &Path) -> bool {
    let project_root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let project_config = project_root.join(".smith");
    matches!(source.layer, Layer::ProjectFile | Layer::ProjectLocalFile)
        || source.file.as_ref().is_some_and(|file| {
            file.starts_with(&project_config)
                || file
                    .canonicalize()
                    .unwrap_or_else(|_| file.clone())
                    .starts_with(&project_config)
        })
}

/// Lists saved sessions for `project_root`, newest first.
pub async fn list(
    config: &ResolvedConfig,
    project_root: impl AsRef<Path>,
) -> Result<Vec<SessionListing>, HostSessionError> {
    reject_project_controlled_persistence(config, project_root.as_ref())?;
    if !config.persistence.enabled.value {
        return Ok(Vec::new());
    }
    Ok(FileSessionStore::new(paths(config, project_root.as_ref())?)
        .list()
        .await?)
}

/// Validates host-owned authority and persistence boundaries without creating
/// a runtime or session.
pub fn validate_host_policy(
    config: &ResolvedConfig,
    project_root: impl AsRef<Path>,
) -> Result<(), HostSessionError> {
    reject_project_granted_authority(config, project_root.as_ref())?;
    reject_project_controlled_persistence(config, project_root.as_ref())
}

/// Resolves the configured session directory for one canonical project.
pub fn paths(
    config: &ResolvedConfig,
    project_root: impl AsRef<Path>,
) -> Result<SessionPaths, HostSessionError> {
    reject_project_controlled_persistence(config, project_root.as_ref())?;
    let project = project_id(project_root)?;
    Ok(SessionPaths::from_sessions_dir(
        &config.persistence.sessions_dir.value,
        &project,
    ))
}

/// Derives a stable, path-safe project identity from its canonical path.
pub fn project_id(project_root: impl AsRef<Path>) -> Result<ProjectId, RuntimeError> {
    let canonical = project_root.as_ref().canonicalize().map_err(|error| {
        RuntimeError::new(
            ErrorKind::Config,
            format!(
                "cannot resolve project root `{}`: {error}",
                project_root.as_ref().display()
            ),
        )
    })?;
    let fingerprint = agent_runtime::registry::Fingerprint::of(path_bytes(&canonical));
    ProjectId::new(fingerprint.as_str())
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> &[u8] {
    // Smith's first supported hosts are macOS and Linux, where the branch
    // above hashes the exact OS bytes. This fallback keeps other targets
    // buildable until their native path encoding gets a release contract.
    path.to_str().unwrap_or_default().as_bytes()
}

/// Bounds how long shutdown waits for a session's background-task workers to
/// kill their process groups and record their terminal journal marker.
///
/// The registry drops a task from `running_tasks` as soon as its worker
/// records the terminal status, an instant before that worker appends the
/// journal marker; the trailing poll interval after the list goes empty
/// exists to close that window instead of racing the journal shutdown that
/// follows this call against an in-flight marker write.
const BACKGROUND_TASK_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const BACKGROUND_TASK_POLL_INTERVAL: Duration = Duration::from_millis(25);

async fn wait_for_background_tasks_to_stop(
    registry: &BackgroundTaskRegistry,
    session_id: &SessionId,
) {
    let deadline = Instant::now() + BACKGROUND_TASK_SHUTDOWN_GRACE;
    while !registry.running_tasks(session_id).is_empty() {
        if Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(BACKGROUND_TASK_POLL_INTERVAL).await;
    }
    tokio::time::sleep(BACKGROUND_TASK_POLL_INTERVAL).await;
}

/// Mints an explicit identity so persistence observers can be attached before
/// Agent Runtime emits `SessionStarted`.
pub fn mint_session_id() -> SessionId {
    SessionId::new(format!("session-{}", uuid::Uuid::new_v4()))
}

/// An observer installed before runtime construction and bound before start.
#[derive(Debug, Default)]
struct DeferredObserver {
    target: RwLock<Option<Arc<dyn EventObserver>>>,
}

impl DeferredObserver {
    fn install(&self, observer: Arc<dyn EventObserver>) -> Result<(), RuntimeError> {
        let mut target = self
            .target
            .write()
            .map_err(|_| RuntimeError::internal("journal observer state is poisoned"))?;
        if target.is_some() {
            return Err(RuntimeError::internal(
                "journal observer was installed more than once",
            ));
        }
        *target = Some(observer);
        Ok(())
    }
}

impl EventObserver for DeferredObserver {
    fn observe(&self, event: &EventEnvelope) {
        let target = self.target.read().ok().and_then(|target| target.clone());
        if let Some(target) = target {
            target.observe(event);
        }
    }
}

/// How many recent event envelopes [`EventRing`] retains.
///
/// Token deltas are coalesced before emission now, so a real burst is on the
/// order of tens of events per millisecond rather than the 1046-in-one-
/// millisecond burst that first overran the journal and broadcast channel
/// (see [`crate::journal::JournalConfig`]'s default). A gap is queried the
/// instant it is detected, not minutes later, so this bound only has to
/// outlast the time between a burst landing and the lagged subscriber asking
/// for it — a few thousand envelopes is generous headroom for that, without
/// keeping unbounded session history in memory. The case this bound does not
/// cover — a gap that predates process start on a resumed session, or a
/// session old enough to have scrolled the range out — is exactly what the
/// journal-read fallback in `Host::journal_events_between` exists for.
const EVENT_RING_CAPACITY: usize = 4096;

/// A bounded, in-memory record of the most recently emitted session events.
///
/// `Host::journal_events_between` used to answer every lagged-subscriber gap
/// by fsyncing and re-parsing the entire multi-megabyte journal file, on the
/// TUI's own event-loop task. That await starved the subscriber and the
/// journal writer alike, which produced the *next* gap — a self-reinforcing
/// cascade. This ring is populated by an observer installed the same way as
/// the journal's (see `start`, beside where the journal's `DeferredObserver`
/// slot is installed), so it sees the same synchronous, pre-broadcast event
/// stream the journal does, and answers the common case — a lag that just
/// happened — from memory instead of disk.
#[derive(Debug, Default)]
struct EventRing {
    events: Mutex<VecDeque<EventEnvelope>>,
}

impl EventRing {
    /// Returns the requested inclusive range when every event in it is still
    /// held, `None` when any part of the range has been evicted or was never
    /// observed — the signal for the caller to fall back to the journal.
    ///
    /// Events come back raw, not redacted: redaction is a JSON round trip,
    /// deliberately kept off the synchronous `observe` hot path (see there),
    /// so it is the caller's job to redact before this reaches a display.
    fn events_between(&self, first: u64, last: u64) -> Option<Vec<EventEnvelope>> {
        let events = self.events.lock().expect("event ring lock poisoned");
        let oldest = events.front()?.seq;
        let newest = events.back()?.seq;
        if first < oldest || last > newest {
            return None;
        }
        Some(
            events
                .iter()
                .filter(|event| event.seq >= first && event.seq <= last)
                .cloned()
                .collect(),
        )
    }
}

impl EventObserver for EventRing {
    fn observe(&self, event: &EventEnvelope) {
        // Non-blocking by construction, matching the journal observer this
        // is installed beside: a clone and a bounded push, nothing that can
        // stall the runtime's emission path. Redaction is deliberately not
        // done here — see `events_between` — so a burst of arrivals never
        // pays for a JSON round trip on this synchronous path.
        let mut events = self.events.lock().expect("event ring lock poisoned");
        events.push_back(event.clone());
        if events.len() > EVENT_RING_CAPACITY {
            events.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::JournalLine;
    use crate::resume_capsule::{ResumeSummaryOutcome, ResumeSummaryPurpose};
    use agent_runtime_core::artifact::{ArtifactDigest, ArtifactId};
    use agent_runtime_core::cancel::CancelReason;
    use agent_runtime_core::content::ToolCall;
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::ids::{EventId, QuestionId};
    use agent_runtime_core::interaction::InteractionSensitivity;
    use agent_runtime_core::provider::{
        CacheEndpointIdentity, CacheIdentity, CacheIdentityFragment, ModelId,
    };
    use serde_json::json;

    fn journal_event(seq: u64, payload: RuntimeEvent) -> JournalLine {
        JournalLine::new(JournalRecord::Event {
            event: EventEnvelope::new(
                seq,
                EventId::new(format!("evt-{seq}")),
                SessionId::new("session-recovery"),
                None,
                agent_runtime_core::clock::Timestamp(seq),
                payload,
            ),
        })
    }

    fn cache_identity() -> CacheIdentity {
        CacheIdentity::builder(
            "provider",
            ModelId::new("model"),
            CacheEndpointIdentity::from_opaque(
                "endpoint",
                agent_runtime::registry::RegistryRevision::new("endpoint-r1"),
            ),
            agent_runtime::registry::RegistryRevision::new("adapter-r1"),
            Fingerprint::of("profile"),
        )
        .provider_key(Fingerprint::of("account"))
        .stable_prefix(vec![CacheIdentityFragment::new(
            "system",
            Fingerprint::of("system"),
        )])
        .build()
    }

    fn artifact_reference(
        session: &SessionId,
        id: &str,
        purpose: &str,
        media_type: &str,
    ) -> ArtifactRef {
        ArtifactRef {
            id: ArtifactId::new(id).expect("bounded artifact id"),
            digest: ArtifactDigest::new("sha256", "aa").expect("valid digest"),
            media_type: media_type.to_owned(),
            byte_length: 8,
            sensitivity: ArtifactSensitivity::Sensitive,
            retention: ArtifactRetention::Session,
            provenance: ArtifactProvenance::new(session.clone(), purpose),
        }
    }

    #[test]
    fn stale_runtime_ordinary_summary_cannot_replace_a_handoff_during_persist_prepare() {
        let session = SessionId::new("session-handoff-persist");
        let identity = cache_identity();
        let slot = ResumeCapsuleSlot::new(session.clone(), agent_runtime_core::clock::Timestamp(1));
        let handoff_artifact = artifact_reference(
            &session,
            "handoff-artifact",
            crate::resume_capsule::RESUME_SUMMARY_ARTIFACT_PURPOSE,
            RESUME_SUMMARY_MEDIA_TYPE,
        );
        let runtime_state = artifact_reference(
            &session,
            "runtime-state",
            RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE,
            RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE,
        );
        slot.update(|capsule| {
            capsule
                .attach_runtime_summary_state_artifact(runtime_state.clone())
                .expect("prior ordinary Runtime state artifact");
            capsule.cache.prior_identity = Some(identity.clone());
            capsule
                .record_handoff_summary(
                    "provider",
                    "model",
                    agent_runtime::registry::RegistryRevision::new("handoff-r1"),
                    identity,
                    agent_runtime_core::clock::Timestamp(2),
                    "handoff body",
                    vec![SummaryCoverage::new("canonical_events", 0, 3)],
                )
                .expect("valid handoff summary");
            capsule
                .attach_summary_artifact(handoff_artifact)
                .expect("valid handoff artifact");
        });
        let handoff = slot
            .snapshot()
            .semantic_summary
            .expect("handoff projection");

        let persistence = RuntimeSummaryPersistence {
            state_artifact: runtime_state,
            ordinary: Some(OrdinarySummaryPersistence {
                model: "summary-model".to_owned(),
                revision: agent_runtime::registry::RegistryRevision::new("ordinary-r2"),
                body: "stale ordinary body".to_owned(),
                usage: SummaryUsage::default(),
                artifact: artifact_reference(
                    &session,
                    "ordinary-artifact",
                    RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
                    RESUME_SUMMARY_MEDIA_TYPE,
                ),
                coverage: vec![SummaryCoverage::new("canonical_history", 0, 2)],
            }),
        };
        slot.try_update_atomic(|capsule| {
            project_runtime_summary_persistence(
                capsule,
                persistence,
                Some("summary-provider"),
                agent_runtime_core::clock::Timestamp(3),
            )
        })
        .expect("runtime summary state reference projects atomically");

        let after = slot.snapshot();
        assert_eq!(after.semantic_summary.as_ref(), Some(&handoff));
        assert!(after.latest_summary_state_artifact.is_some());
        let (_, state) = slot
            .prepare_versioned_state(agent_runtime_core::clock::Timestamp(4))
            .expect("handoff capsule remains persistable");
        assert_eq!(
            state.value["semantic_summary"]["provenance"]["purpose"],
            "handoff_checkpoint"
        );
        assert_eq!(
            state.value["semantic_summary"]["provenance"]["provider"],
            "provider"
        );
        assert_eq!(
            state.value["semantic_summary"]["provenance"]["model"],
            "model"
        );

        let newer_runtime_state = artifact_reference(
            &session,
            "runtime-state-newer",
            RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE,
            RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE,
        );
        slot.try_update_atomic(|capsule| {
            project_runtime_summary_persistence(
                capsule,
                RuntimeSummaryPersistence {
                    state_artifact: newer_runtime_state.clone(),
                    ordinary: Some(OrdinarySummaryPersistence {
                        model: "summary-model-newer".to_owned(),
                        revision: agent_runtime::registry::RegistryRevision::new("ordinary-r3"),
                        body: "newer ordinary body".to_owned(),
                        usage: SummaryUsage::default(),
                        artifact: artifact_reference(
                            &session,
                            "ordinary-artifact-newer",
                            RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
                            RESUME_SUMMARY_MEDIA_TYPE,
                        ),
                        coverage: vec![SummaryCoverage::new("canonical_history", 0, 4)],
                    }),
                },
                Some("summary-provider"),
                agent_runtime_core::clock::Timestamp(5),
            )
        })
        .expect("newer ordinary Runtime state replaces the older handoff");
        let newer = slot.snapshot();
        let newer_summary = newer.semantic_summary.expect("newer ordinary projection");
        assert_eq!(
            newer_summary.provenance.purpose,
            ResumeSummaryPurpose::OrdinarySummary
        );
        assert_eq!(newer_summary.provenance.provider, "summary-provider");
        assert_eq!(newer_summary.provenance.model, "summary-model-newer");
        assert_eq!(newer_summary.provenance.cache_identity, None);
        assert_eq!(
            newer.latest_summary_state_artifact.as_ref(),
            Some(&newer_runtime_state)
        );
    }

    #[test]
    fn stale_runtime_success_cannot_replace_a_newer_failed_idle_projection() {
        let session = SessionId::new("session-idle-failure-persist");
        let runtime_state = artifact_reference(
            &session,
            "runtime-state-before-failure",
            RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE,
            RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE,
        );
        let slot = ResumeCapsuleSlot::new(session.clone(), agent_runtime_core::clock::Timestamp(1));
        slot.update(|capsule| {
            capsule
                .attach_runtime_summary_state_artifact(runtime_state.clone())
                .expect("prior successful Runtime summary state");
            capsule
                .record_failed_ordinary_summary(
                    "summary-provider",
                    "summary-model",
                    agent_runtime::registry::RegistryRevision::new("failed-r2"),
                    agent_runtime_core::clock::Timestamp(3),
                    vec![SummaryCoverage::new("canonical_events", 0, 4)],
                )
                .expect("failed idle projection");
        });
        let failed = slot
            .snapshot()
            .semantic_summary
            .expect("failed summary metadata");

        slot.try_update_atomic(|capsule| {
            project_runtime_summary_persistence(
                capsule,
                RuntimeSummaryPersistence {
                    state_artifact: runtime_state,
                    ordinary: Some(OrdinarySummaryPersistence {
                        model: "summary-model".to_owned(),
                        revision: agent_runtime::registry::RegistryRevision::new("successful-r1"),
                        body: "older successful body".to_owned(),
                        usage: SummaryUsage::default(),
                        artifact: artifact_reference(
                            &session,
                            "older-success-artifact",
                            RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
                            RESUME_SUMMARY_MEDIA_TYPE,
                        ),
                        coverage: vec![SummaryCoverage::new("canonical_history", 0, 2)],
                    }),
                },
                Some("summary-provider"),
                agent_runtime_core::clock::Timestamp(4),
            )
        })
        .expect("unchanged Runtime state is recognized as stale");

        let after = slot.snapshot();
        assert_eq!(after.semantic_summary.as_ref(), Some(&failed));
        assert_eq!(
            after
                .semantic_summary
                .as_ref()
                .map(|summary| summary.provenance.outcome),
            Some(ResumeSummaryOutcome::Failed)
        );
        slot.prepare_versioned_state(agent_runtime_core::clock::Timestamp(5))
            .expect("failed metadata remains persistable");
    }

    #[test]
    fn canonical_tool_calls_are_resolved_by_stable_id_without_exposing_arguments() {
        let history = vec![Message::assistant(vec![
            ContentPart::ToolCall(ToolCall {
                id: ToolCallId::new("call-read"),
                name: "read".to_owned(),
                arguments: json!({"path": "src/lib.rs"}),
            }),
            ContentPart::ToolCall(ToolCall {
                id: ToolCallId::new("call-shell"),
                name: "shell".to_owned(),
                arguments: json!({
                    "command": "printf TOP_SECRET_COMMAND",
                    "cwd": "crates/smith-cli"
                }),
            }),
        ])];

        let redactor = DefaultRedactor::new().with_secret("TOP_SECRET_COMMAND");
        assert!(
            tool_call_display_from_history(&[], &ToolCallId::new("call-shell"), &redactor)
                .is_none(),
            "request-time lookup can race canonical history visibility"
        );
        let display =
            tool_call_display_from_history(&history, &ToolCallId::new("call-shell"), &redactor)
                .expect("matching canonical call");
        assert_eq!(
            display.invocation(),
            "Shell(printf [redacted] · cwd crates/smith-cli)"
        );
        assert!(!display.invocation().contains("TOP_SECRET_COMMAND"));
        let ContentPart::ToolCall(canonical) = &history[0].content[1] else {
            panic!("expected canonical tool call");
        };
        assert_eq!(canonical.arguments["command"], "printf TOP_SECRET_COMMAND");
        let resumed = tool_call_displays_from_history(&history, &redactor);
        assert_eq!(
            resumed
                .iter()
                .find(|(call, _)| call.as_str() == "call-shell")
                .map(|(_, display)| display),
            Some(&display),
            "completion retry and resume must converge on the same projection"
        );
        assert!(
            tool_call_display_from_history(&history, &ToolCallId::new("missing"), &redactor)
                .is_none()
        );
    }

    #[test]
    fn only_terminal_children_are_removed_from_ephemeral_recovery() {
        let child = ChildId::new("child-1");
        let spawned = RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        };
        let resolutions = [
            (
                RuntimeEvent::ChildCompleted {
                    child: child.clone(),
                    result: "done".to_owned(),
                },
                true,
            ),
            (
                RuntimeEvent::ChildStopped {
                    child: child.clone(),
                    reason: CancelReason::Shutdown,
                },
                false,
            ),
            (
                RuntimeEvent::ChildFailed {
                    child: child.clone(),
                    error: RuntimeError::internal("failed"),
                },
                false,
            ),
            (
                RuntimeEvent::ChildNeedsInput {
                    child: child.clone(),
                    child_session: SessionId::new("child-session"),
                    turn: TurnId::new("turn-1"),
                    call: ToolCallId::new("call-1"),
                    request: InteractionRequestId::new("interaction-1"),
                    question_ids: vec![QuestionId::new("question-1")],
                    sensitivity: InteractionSensitivity::Sensitive,
                },
                true,
            ),
        ];

        for (index, (resolution, remains_ephemeral)) in resolutions.into_iter().enumerate() {
            let recovery = JournalRecovery {
                records: vec![
                    journal_event(0, spawned.clone()),
                    journal_event(u64::try_from(index).unwrap_or(0) + 1, resolution),
                ],
                truncated_tail: None,
            };
            let interruption = unresolved_ephemeral_work(&recovery);
            if remains_ephemeral {
                assert_eq!(
                    interruption
                        .expect("a follow-up-capable child remains ephemeral")
                        .children
                        .as_slice(),
                    std::slice::from_ref(&child)
                );
            } else {
                assert!(
                    interruption.is_none(),
                    "a terminal child was treated as live"
                );
            }
        }
    }

    #[test]
    fn monitor_lifecycle_and_prior_interruption_are_reconciled_exactly_once() {
        let running = "monitor:build".to_owned();
        let stopped = "monitor:lint".to_owned();
        let recovery = JournalRecovery {
            records: vec![
                JournalLine::new(JournalRecord::MonitorStarted {
                    monitor: running.clone(),
                }),
                JournalLine::new(JournalRecord::MonitorStarted {
                    monitor: stopped.clone(),
                }),
                JournalLine::new(JournalRecord::MonitorStopped { monitor: stopped }),
            ],
            truncated_tail: None,
        };
        let interruption = unresolved_ephemeral_work(&recovery)
            .expect("the unresolved monitor is interrupted on recovery");
        assert_eq!(
            interruption.monitors.as_slice(),
            std::slice::from_ref(&running)
        );
        assert!(interruption.children.is_empty());

        let mut reconciled = recovery;
        reconciled
            .records
            .push(JournalLine::new(JournalRecord::EphemeralWorkInterrupted {
                interruption,
            }));
        assert!(
            unresolved_ephemeral_work(&reconciled).is_none(),
            "the persisted recovery marker must prevent duplicate interruption"
        );
    }

    #[test]
    fn a_background_task_started_without_a_terminal_marker_is_reported_and_never_duplicated() {
        let running = "task:build".to_owned();
        let exited = "task:lint".to_owned();
        let recovery = JournalRecovery {
            records: vec![
                JournalLine::new(JournalRecord::TaskStarted {
                    task: running.clone(),
                }),
                JournalLine::new(JournalRecord::TaskStarted {
                    task: exited.clone(),
                }),
                JournalLine::new(JournalRecord::TaskExited { task: exited }),
            ],
            truncated_tail: None,
        };
        let interruption = unresolved_ephemeral_work(&recovery)
            .expect("a task started with no terminal marker is interrupted on recovery");
        assert_eq!(
            interruption.tasks.as_slice(),
            std::slice::from_ref(&running)
        );
        assert!(interruption.children.is_empty());
        assert!(interruption.monitors.is_empty());

        let mut reconciled = recovery;
        reconciled
            .records
            .push(JournalLine::new(JournalRecord::EphemeralWorkInterrupted {
                interruption,
            }));
        assert!(
            unresolved_ephemeral_work(&reconciled).is_none(),
            "the persisted recovery marker must prevent duplicate interruption"
        );
    }
}
