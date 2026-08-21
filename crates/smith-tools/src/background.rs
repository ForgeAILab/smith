//! The seam between Smith's built-in tools and a session-owned background
//! task registry.
//!
//! `smith-tools` has no business owning process lifetime beyond one
//! invocation — everything past "spawn it and wait" belongs to whatever host
//! composes a runtime around these tools. `smith-runtime` is that host today,
//! but the dependency graph only runs one way (`smith-runtime` depends on
//! `smith-tools`, never the reverse), so this module cannot import the real
//! registry. Instead it declares the operations `shell`, `task_output`, and
//! `task_stop` need. The composing host injects one explicit implementation
//! into those tool instances; there is no process-global fallback.
//!
//! Tests that do not need process lifetime use [`unavailable`], a deliberate
//! fail-closed adapter rather than ambient state.

use std::path::PathBuf;
use std::sync::Arc;

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::ids::SessionId;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};

/// Terminal state or running status of a background task, as seen from the
/// tool side of the seam.
///
/// Mirrors `smith_runtime::background_tasks::TaskStatus` field for field; the
/// two types cannot be the same one without `smith-tools` depending on
/// `smith-runtime`, which would invert the crate graph. The runtime-side
/// adapter converts between them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTaskStatus {
    /// Still executing.
    Running,
    /// Exited on its own; `code` is `None` when it was killed by a signal.
    Exited {
        /// The process exit code, when the process exited normally.
        code: Option<i32>,
    },
    /// Stopped by `task_stop` or a manual-backgrounding takeover shutting
    /// down.
    Stopped,
    /// Killed because its explicit `timeout_ms` deadline elapsed.
    DeadlineKill,
    /// Killed because the owning session shut down.
    Shutdown,
    /// Recovered on resume: a start marker with no terminal marker, so the
    /// process is presumed gone rather than respawned.
    InterruptedByProcessExit,
}

impl BackgroundTaskStatus {
    /// Whether the task has reached a final state.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Running)
    }

    /// A stable, model-facing status word.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Exited { .. } => "exited",
            Self::Stopped => "stopped",
            Self::DeadlineKill => "deadline_kill",
            Self::Shutdown => "shutdown",
            Self::InterruptedByProcessExit => "interrupted_by_process_exit",
        }
    }

    /// The process exit code, when the terminal state is a plain exit.
    pub fn exit_code(&self) -> Option<i32> {
        match self {
            Self::Exited { code } => *code,
            _ => None,
        }
    }
}

/// Identity returned by a successful spawn or adoption.
#[derive(Debug, Clone)]
pub struct SpawnedTask {
    /// The session-scoped task ID, stable for the life of the task.
    pub task_id: String,
    /// A reference to the task's output spool, for display and diagnostics.
    pub spool_ref: String,
}

/// A bounded, offset-addressed slice of a task's spooled output.
#[derive(Debug, Clone)]
pub struct BackgroundTaskOutput {
    /// The task's current status.
    pub status: BackgroundTaskStatus,
    /// The process exit code, once `status` is terminal and it exited
    /// normally.
    pub exit_code: Option<i32>,
    /// The byte offset this slice actually starts at (clamped to the
    /// spool's length).
    pub offset: usize,
    /// The offset to pass as `offset` on the next poll to read only what
    /// arrived since.
    pub next_offset: usize,
    /// The output between `offset` and `next_offset`.
    pub output: String,
    /// Whether the task's spool has been truncated at its byte cap.
    pub truncated: bool,
}

/// The host operations the `shell`, `task_output`, and `task_stop` tools need
/// from a session-owned background task registry.
///
/// Implemented once, over the real registry, and installed process-wide
/// during runtime composition (see [`install`]). Every method takes the
/// owning session explicitly rather than assuming a single ambient one,
/// because one process may compose more than one session's runtime.
#[async_trait]
pub trait BackgroundTaskHost: Send + Sync + std::fmt::Debug {
    /// Spawns `command` as a new session-owned background task.
    ///
    /// `timeout_ms` is `None` for "no deadline" — the ordinary case for an
    /// explicit `run_in_background` call — and `Some` to bound it, mirroring
    /// the foreground tool's own `timeout_ms`.
    async fn spawn(
        &self,
        session: SessionId,
        command: String,
        cwd: PathBuf,
        timeout_ms: Option<u64>,
    ) -> Result<SpawnedTask, RuntimeError>;

    /// Adopts an already-running foreground child as a background task.
    ///
    /// `lines` is the live channel the foreground `shell` invocation's own
    /// reader tasks are already feeding: by the time a manual-backgrounding
    /// signal fires, `child`'s stdout/stderr have already been taken, so the
    /// host must keep draining `lines` rather than taking the child's streams
    /// a second time. `captured_so_far` is the output collected before the
    /// handoff and becomes the spool's opening content.
    // Every parameter here is independently meaningful to the handoff; a
    // wrapper struct would just relocate the same fields, not simplify them.
    #[allow(clippy::too_many_arguments)]
    async fn adopt(
        &self,
        session: SessionId,
        command: String,
        cwd: PathBuf,
        child: Child,
        group_pid: Option<u32>,
        captured_so_far: String,
        lines: mpsc::Receiver<String>,
    ) -> Result<SpawnedTask, RuntimeError>;

    /// Reads a bounded, offset-addressed slice of a task's spooled output.
    async fn output(
        &self,
        session: SessionId,
        task_id: String,
        offset: usize,
        limit: usize,
    ) -> Result<BackgroundTaskOutput, RuntimeError>;

    /// Stops a task's owned process group.
    ///
    /// Idempotent on an already-terminal task: returns its existing terminal
    /// status rather than erroring.
    async fn stop(
        &self,
        session: SessionId,
        task_id: String,
    ) -> Result<BackgroundTaskStatus, RuntimeError>;

    /// Registers a one-shot signal that fires when the user manually
    /// backgrounds the session's active foreground shell call (ctrl+b).
    ///
    /// Registering again for the same session replaces any prior signal —
    /// only the most recent foreground call is backgroundable at a time.
    fn register_foreground_signal(&self, session: SessionId) -> Option<oneshot::Receiver<()>>;
}

/// Deliberate fail-closed adapter for tool-only tests and embeddings that do
/// not provide background process ownership.
#[derive(Debug, Default)]
pub struct UnavailableBackgroundTaskHost;

/// Returns an explicit unavailable background host.
pub fn unavailable() -> Arc<dyn BackgroundTaskHost> {
    Arc::new(UnavailableBackgroundTaskHost)
}

/// The error every tool in this seam returns when no host is installed.
pub fn host_unavailable() -> RuntimeError {
    RuntimeError::new(
        ErrorKind::Tool,
        "background tasks are not available in this environment: the composed host does not provide them",
    )
}

#[async_trait]
impl BackgroundTaskHost for UnavailableBackgroundTaskHost {
    async fn spawn(
        &self,
        _session: SessionId,
        _command: String,
        _cwd: PathBuf,
        _timeout_ms: Option<u64>,
    ) -> Result<SpawnedTask, RuntimeError> {
        Err(host_unavailable())
    }

    async fn adopt(
        &self,
        _session: SessionId,
        _command: String,
        _cwd: PathBuf,
        _child: Child,
        _group_pid: Option<u32>,
        _captured_so_far: String,
        _lines: mpsc::Receiver<String>,
    ) -> Result<SpawnedTask, RuntimeError> {
        Err(host_unavailable())
    }

    async fn output(
        &self,
        _session: SessionId,
        _task_id: String,
        _offset: usize,
        _limit: usize,
    ) -> Result<BackgroundTaskOutput, RuntimeError> {
        Err(host_unavailable())
    }

    async fn stop(
        &self,
        _session: SessionId,
        _task_id: String,
    ) -> Result<BackgroundTaskStatus, RuntimeError> {
        Err(host_unavailable())
    }

    fn register_foreground_signal(&self, _session: SessionId) -> Option<oneshot::Receiver<()>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_words_are_stable_and_only_running_is_nonterminal() {
        assert_eq!(BackgroundTaskStatus::Running.as_str(), "running");
        assert!(!BackgroundTaskStatus::Running.is_terminal());
        for (status, word) in [
            (BackgroundTaskStatus::Exited { code: Some(0) }, "exited"),
            (BackgroundTaskStatus::Stopped, "stopped"),
            (BackgroundTaskStatus::DeadlineKill, "deadline_kill"),
            (BackgroundTaskStatus::Shutdown, "shutdown"),
            (
                BackgroundTaskStatus::InterruptedByProcessExit,
                "interrupted_by_process_exit",
            ),
        ] {
            assert_eq!(status.as_str(), word);
            assert!(status.is_terminal(), "{word}");
        }
        assert_eq!(
            BackgroundTaskStatus::Exited { code: Some(7) }.exit_code(),
            Some(7)
        );
        assert_eq!(BackgroundTaskStatus::Stopped.exit_code(), None);
    }

    #[tokio::test]
    async fn unavailable_is_an_explicit_fail_closed_adapter() {
        let host = unavailable();
        let err = host
            .stop(SessionId::new("s"), "task:1".into())
            .await
            .expect_err("unavailable host");
        assert_eq!(err.kind, ErrorKind::Tool);
        assert!(err.message.contains("does not provide"));
    }
}
