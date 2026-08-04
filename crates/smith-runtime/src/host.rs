//! Standard Smith session composition shared by interactive and headless hosts.
//!
//! [`crate::factory`] maps resolved product policy onto Agent Runtime. This
//! module adds the host-owned lifecycle around that immutable runtime:
//! project-scoped paths, an optional snapshot store, a canonical event journal,
//! explicit create/resume identity, and ordered shutdown. It deliberately does
//! not render a terminal or choose an output format, so `smith` and `smith -p`
//! cannot drift in their persistence behavior.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use agent_runtime::delegation::ChildDurability;
use agent_runtime::runtime::{
    CheckpointRecoveryPolicy, GoalAdmissionGate, GoalController, GoalControllerConfig,
    SessionHandle, StartSession,
};
use agent_runtime_core::checkpoint::{TurnCheckpoint, TurnState};
use agent_runtime_core::content::{ContentPart, Message};
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::event::EventEnvelope;
use agent_runtime_core::event::RuntimeEvent;
use agent_runtime_core::goal::{GoalCommand, GoalCommandResult, GoalProjection};
use agent_runtime_core::ids::{ChildId, InteractionRequestId, SessionId, ToolCallId, TurnId};
use agent_runtime_core::observer::EventObserver;
use agent_runtime_core::store::{SessionSnapshot, SessionStore};
use async_trait::async_trait;
use smith_config::model::ApprovalMode;
use smith_config::resolve::{Layer, ResolvedConfig, Source};
use smith_tools::{ToolCallDisplay, project_tool_call_display};

use crate::artifact::SmithArtifactStore;
use crate::checkpoint::{
    CheckpointBarrier, CheckpointKeyProvider, ConfiguredCheckpointKeyProvider,
    CredentialCheckpointKeyProvider, SmithCheckpointSetup,
};
use crate::factory::{FactoryError, RuntimeRequest, SmithRuntime};
use crate::journal::{
    DefaultRedactor, EphemeralWorkInterruption, EventJournal, JournalConfig, JournalRecord,
    JournalRecovery, JournalStats, Redactor, read_journal, reconcile_nonterminal_journal,
};
use crate::private_storage::{PrivateFileLock, try_acquire_private_lock};
use crate::project_instructions::discover as discover_project_instructions;
use crate::reasoning::{PersistedReasoningOverride, SESSION_STATE_NAMESPACE};
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
}

impl HostSessionRequest {
    /// Creates a request for a fresh session rooted at `project_root`.
    pub fn new(mut runtime: RuntimeRequest, project_root: impl Into<PathBuf>) -> Self {
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
}

/// A running Smith session and the host resources that must shut down with it.
#[derive(Debug)]
pub struct HostSession {
    runtime: SmithRuntime,
    session: SessionHandle,
    display_redactor: DefaultRedactor,
    journal: Option<Arc<EventJournal>>,
    paths: Option<SessionPaths>,
    changes: Arc<smith_tools::ChangeRecorder>,
    lifecycle_lease: Mutex<Option<PrivateFileLock>>,
    restored_interaction: Option<RestoredInteraction>,
    recovered_ephemeral_work: Option<EphemeralWorkInterruption>,
    goal_controller: Mutex<Option<GoalController>>,
    goal_admission_gate: Option<GoalAdmissionGate>,
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

    /// Flushes and returns the canonical redacted events with sequence numbers
    /// in `first..=last`.
    ///
    /// This is the healing path for a lagged live subscriber: the journal
    /// observes every event synchronously before broadcast, so anything the
    /// stream skipped is already queued here. A non-persistent session, or a
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
        journal.flush().await?;
        let path = paths.journal(self.session.id())?;
        let recovery = read_journal(path).await?;
        Ok(recovery
            .events()
            .into_iter()
            .filter(|event| event.seq >= first && event.seq <= last)
            .cloned()
            .collect())
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
        self.session
            .with_history(|history| tool_call_displays_from_history(history, &self.display_redactor))
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

    /// Shuts down the runtime first, then drains and syncs its journal.
    ///
    /// The journal is attempted even when snapshot persistence fails so a
    /// storage error cannot strand the writer task or silently lose events
    /// already accepted by its bounded queue.
    pub async fn shutdown(&self) -> Result<Option<JournalStats>, RuntimeError> {
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
            Some(coordinator) => coordinator.flush().await,
            None => Ok(()),
        };
        let session = self.session.shutdown().await;
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
    let surface = request.runtime.surface;
    reject_project_granted_authority(&config, &request.project_root)?;
    reject_project_controlled_persistence(&config, &request.project_root)?;
    let persistence = config.persistence.enabled.value;
    let session_id = request.session_id.clone().unwrap_or_else(mint_session_id);

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
    let (paths, journal_slot, checkpoint_barrier) = if persistence {
        let paths = paths(&config, &request.project_root)?;
        if request.runtime.artifact_store.is_none() {
            request.runtime.artifact_store = Some(Arc::new(SmithArtifactStore::new(paths.clone())));
        }
        let inner = FileSessionStore::new(paths.clone());
        if request.session_id.is_some() {
            let snapshot = inner.load(&session_id).await?;
            resume_snapshot_exists = snapshot.is_some();
            if let Some(state) = snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.extension_state.get(SESSION_STATE_NAMESPACE))
            {
                PersistedReasoningOverride::restore(state)?.apply(
                    &mut config,
                    request.reasoning_reset_enabled,
                    request.reasoning_reset_effort,
                );
                request.runtime.config = config.clone();
            }
        }
        let store = Arc::new(RedactingSessionStore::new(
            inner,
            persistence_redactor.clone(),
            PersistedReasoningOverride::from_config(&config),
        ));
        request.runtime.session_store = Some(store);
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
            request.runtime.checkpoint_setup = Some(match provider {
                Some(provider) => SmithCheckpointSetup::with_provider(paths.clone(), provider),
                None => SmithCheckpointSetup::platform(paths.clone()),
            });
        }

        let (slot, barrier) = if config.persistence.journal_events.value {
            let slot = Arc::new(DeferredObserver::default());
            request.runtime.observers.push(slot.clone());
            let barrier = Arc::new(JournalCheckpointBarrier::default());
            request.runtime.checkpoint_barrier =
                Some(barrier.clone() as Arc<dyn CheckpointBarrier>);
            (Some(slot), Some(barrier))
        } else {
            (None, None)
        };
        (Some(paths), slot, barrier)
    } else {
        (None, None, None)
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

    let runtime = crate::factory::build(request.runtime).await?;

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

    let mut start = StartSession::new().with_id(session_id);
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

    // Root sessions get their delegation coordinator now that the session
    // exists: the `agent` tool starts answering, and completed child results
    // are routed into the session's safe-boundary inbox.
    if let Some(delegation) = runtime.delegation() {
        crate::delegation::wire_delegation(&session, delegation).await?;
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

    Ok(HostSession {
        runtime,
        session,
        display_redactor: persistence_redactor,
        journal,
        paths,
        changes,
        lifecycle_lease: Mutex::new(lifecycle_lease),
        restored_interaction,
        recovered_ephemeral_work,
        goal_controller: Mutex::new(goal_controller),
        goal_admission_gate,
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
            }
            JournalRecord::MonitorStarted { monitor } => {
                monitors.insert(monitor.clone());
            }
            JournalRecord::MonitorStopped { monitor } => {
                monitors.remove(monitor);
            }
            JournalRecord::Oversized { .. } | JournalRecord::Dropped { .. } => {}
        }
    }
    let interruption = EphemeralWorkInterruption::process_exit(children, monitors);
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
}

impl RedactingSessionStore {
    fn new(
        inner: FileSessionStore,
        redactor: DefaultRedactor,
        reasoning: PersistedReasoningOverride,
    ) -> Self {
        Self {
            inner,
            redactor,
            reasoning,
        }
    }
}

#[async_trait]
impl SessionStore for RedactingSessionStore {
    async fn load(&self, id: &SessionId) -> Result<Option<SessionSnapshot>, RuntimeError> {
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
        let mut value = serde_json::to_value(&snapshot).map_err(|error| {
            RuntimeError::new(
                ErrorKind::Serialization,
                format!(
                    "session `{}` could not be prepared for redaction: {error}",
                    snapshot.id
                ),
            )
        })?;
        self.redactor.redact(&mut value);
        let redacted = serde_json::from_value(value).map_err(|error| {
            RuntimeError::new(
                ErrorKind::Serialization,
                format!(
                    "session `{}` could not be restored after redaction: {error}",
                    snapshot.id
                ),
            )
        })?;
        self.inner.save(&redacted).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::JournalLine;
    use agent_runtime_core::cancel::CancelReason;
    use agent_runtime_core::content::ToolCall;
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::ids::{EventId, QuestionId};
    use agent_runtime_core::interaction::InteractionSensitivity;
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
}
