//! The seam between Smith's built-in tools and a session-owned background
//! task registry.
//!
//! `smith-tools` has no business owning process lifetime beyond one
//! invocation — everything past "spawn it and wait" belongs to whatever host
//! composes a runtime around these tools. `smith-runtime` is that host today,
//! but the dependency graph only runs one way (`smith-runtime` depends on
//! `smith-tools`, never the reverse), so this module cannot import the real
//! registry. Instead it declares the operations `shell`, `task_output`, and
//! `task_stop` need, and a process-global slot a host installs an
//! implementation into — mirroring the `OnceLock` singleton pattern the
//! registry itself uses for its per-session state.
//!
//! A test that exercises these tools without installing a host (every unit
//! test in this crate) must see [`installed`] return `None` and assert the
//! resulting graceful error, not a panic — background-task support is a
//! runtime capability, not a crate invariant.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

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
pub trait BackgroundTaskHost: Send + Sync {
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
    fn register_foreground_signal(&self, session: SessionId) -> oneshot::Receiver<()>;
}

static HOST: OnceLock<Arc<dyn BackgroundTaskHost>> = OnceLock::new();

/// Installs the process-wide background task host.
///
/// Idempotent by design, mirroring the registry's own `OnceLock`: composing
/// more than one runtime in a process (tests, a host embedding multiple
/// sessions) must not panic on a second install, and every session reaches
/// the same underlying registry regardless, so the first installation wins.
pub fn install(host: Arc<dyn BackgroundTaskHost>) {
    let _ = HOST.set(host);
}

/// The installed host, if any.
///
/// `None` in every unit test in this crate, which compose tools directly
/// without a runtime — that is the graceful path under test, not a gap to
/// work around.
pub fn installed() -> Option<Arc<dyn BackgroundTaskHost>> {
    HOST.get().cloned()
}

/// The error every tool in this seam returns when no host is installed.
pub fn host_unavailable() -> RuntimeError {
    RuntimeError::new(
        ErrorKind::Tool,
        "background tasks are not available in this environment: no background task host is installed",
    )
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

    #[test]
    fn no_host_is_installed_by_default_in_this_crates_unit_tests() {
        // Not a hard guarantee across the whole test binary (another test may
        // install one first), but documents the assumption every graceful-path
        // test in this crate relies on: never install a host from a `src/`
        // unit test.
        if installed().is_none() {
            let err = host_unavailable();
            assert_eq!(err.kind, ErrorKind::Tool);
            assert!(err.message.contains("no background task host"));
        }
    }
}
