//! Standard Smith session composition shared by interactive and headless hosts.
//!
//! [`crate::factory`] maps resolved product policy onto Agent Runtime. This
//! module adds the host-owned lifecycle around that immutable runtime:
//! project-scoped paths, an optional snapshot store, a canonical event journal,
//! explicit create/resume identity, and ordered shutdown. It deliberately does
//! not render a terminal or choose an output format, so `smith` and `smith -p`
//! cannot drift in their persistence behavior.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use agent_runtime::runtime::{SessionHandle, StartSession};
use agent_runtime_core::content::{ContentPart, Message};
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::event::EventEnvelope;
use agent_runtime_core::event::RuntimeEvent;
use agent_runtime_core::ids::{SessionId, ToolCallId};
use agent_runtime_core::observer::EventObserver;
use agent_runtime_core::store::{SessionSnapshot, SessionStore};
use async_trait::async_trait;
use smith_config::model::ApprovalMode;
use smith_config::resolve::{Layer, ResolvedConfig, Source};
use smith_tools::{ToolCallDisplay, project_tool_call_display};

use crate::factory::{FactoryError, RuntimeRequest, SmithRuntime};
use crate::journal::{DefaultRedactor, EventJournal, JournalConfig, JournalStats, Redactor};
use crate::session::{FileSessionStore, ProjectId, SessionListing, SessionPaths};

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
}

impl HostSessionRequest {
    /// Creates a request for a fresh session rooted at `project_root`.
    pub fn new(runtime: RuntimeRequest, project_root: impl Into<PathBuf>) -> Self {
        Self {
            runtime,
            project_root: project_root.into(),
            session_id: None,
            journal: JournalConfig::default(),
        }
    }

    /// Resumes `session_id` instead of minting a fresh identity.
    #[must_use]
    pub fn resume(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }
}

/// A running Smith session and the host resources that must shut down with it.
#[derive(Debug)]
pub struct HostSession {
    runtime: SmithRuntime,
    session: SessionHandle,
    journal: Option<Arc<EventJournal>>,
    paths: Option<SessionPaths>,
    changes: Arc<smith_tools::ChangeRecorder>,
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

    /// Resolves a protected live event to reviewed display metadata.
    ///
    /// Agent Runtime appends the canonical assistant tool call before emitting
    /// `ToolCallRequested`, so this lookup does not require raw arguments in
    /// the event or journal.
    pub fn tool_call_display(&self, call_id: &ToolCallId) -> Option<ToolCallDisplay> {
        tool_call_display_from_history(&self.session.history(), call_id)
    }

    /// Shuts down the runtime first, then drains and syncs its journal.
    ///
    /// The journal is attempted even when snapshot persistence fails so a
    /// storage error cannot strand the writer task or silently lose events
    /// already accepted by its bounded queue.
    pub async fn shutdown(&self) -> Result<Option<JournalStats>, RuntimeError> {
        let session = self.session.shutdown().await;
        let journal = match &self.journal {
            Some(journal) => journal.shutdown().await.map(Some),
            None => Ok(None),
        };
        session?;
        journal
    }
}

fn tool_call_display_from_history(
    history: &[Message],
    call_id: &ToolCallId,
) -> Option<ToolCallDisplay> {
    history.iter().rev().find_map(|message| {
        message.content.iter().rev().find_map(|part| {
            let ContentPart::ToolCall(call) = part else {
                return None;
            };
            (call.id == *call_id)
                .then(|| project_tool_call_display(&call.name, &call.arguments))
                .flatten()
        })
    })
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
    let config = request.runtime.config.clone();
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
    if persistence {
        request.runtime.persistence_redactor = Some(persistence_redactor.clone());
    }

    let (paths, journal_slot) = if persistence {
        let paths = paths(&config, &request.project_root)?;
        let store = Arc::new(RedactingSessionStore::new(
            FileSessionStore::new(paths.clone()),
            persistence_redactor.clone(),
        ));
        if request.session_id.is_some() && store.load(&session_id).await?.is_none() {
            return Err(HostSessionError::SessionNotFound {
                session: session_id,
            });
        }
        request.runtime.session_store = Some(store);

        let slot = if config.persistence.journal_events.value {
            let slot = Arc::new(DeferredObserver::default());
            request.runtime.observers.push(slot.clone());
            Some(slot)
        } else {
            None
        };
        (Some(paths), slot)
    } else {
        (None, None)
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

    let journal = match (&paths, journal_slot) {
        (Some(paths), Some(slot)) => {
            let journal = Arc::new(
                EventJournal::for_session(
                    paths,
                    &session_id,
                    request.journal,
                    Arc::new(persistence_redactor.clone()),
                )
                .await?,
            );
            slot.install(journal.clone())?;
            Some(journal)
        }
        _ => None,
    };

    let session = match runtime
        .runtime()
        .start_session(StartSession::new().with_id(session_id))
        .await
    {
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
        crate::delegation::wire_delegation(&session, delegation)?;
    }

    Ok(HostSession {
        runtime,
        session,
        journal,
        paths,
        changes,
    })
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
}

impl RedactingSessionStore {
    fn new(inner: FileSessionStore, redactor: DefaultRedactor) -> Self {
        Self { inner, redactor }
    }
}

#[async_trait]
impl SessionStore for RedactingSessionStore {
    async fn load(&self, id: &SessionId) -> Result<Option<SessionSnapshot>, RuntimeError> {
        self.inner.load(id).await
    }

    async fn save(&self, snapshot: &SessionSnapshot) -> Result<(), RuntimeError> {
        let mut value = serde_json::to_value(snapshot).map_err(|error| {
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
    use agent_runtime_core::content::ToolCall;
    use serde_json::json;

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

        let display = tool_call_display_from_history(&history, &ToolCallId::new("call-shell"))
            .expect("matching canonical call");
        assert_eq!(display.invocation(), "Shell(crates/smith-cli)");
        assert!(!display.invocation().contains("TOP_SECRET_COMMAND"));
        assert!(tool_call_display_from_history(&history, &ToolCallId::new("missing")).is_none());
    }
}
