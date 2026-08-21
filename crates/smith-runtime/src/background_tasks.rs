//! Session-owned background task registry and lifecycle management.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use agent_runtime::runtime::{InjectedContent, SessionHandle};
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::ids::SessionId;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, watch};

use crate::journal::EventJournal;

/// Default grace period before SIGKILLing a process group.
const GRACE: Duration = Duration::from_millis(500);

/// Bound for an acknowledged stop, including process cleanup and terminal
/// lifecycle publication.
const STOP_ACK_TIMEOUT: Duration = Duration::from_secs(2);

/// Hard cap on task output spool file size (8 MiB).
pub const MAX_SPOOL_BYTES: usize = 8 * 1024 * 1024;

/// Terminal state or running status of a background task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// The process is still running under the session's ownership.
    Running,
    /// The process ended on its own, with its exit code when one was
    /// observable.
    Exited {
        /// The process exit code; absent when it was ended by a signal.
        code: Option<i32>,
    },
    /// `task_stop` (or its host action) terminated the owned group.
    Stopped,
    /// The explicit `timeout_ms` deadline elapsed and the group was killed.
    DeadlineKill,
    /// Session shutdown terminated the group within the grace period.
    Shutdown,
    /// A previous Smith process exited while the task was running; the work
    /// did not survive and was never respawned.
    InterruptedByProcessExit,
}

impl TaskStatus {
    /// Whether the task can no longer produce output or change state.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, TaskStatus::Running)
    }

    /// The snake_case status word used in notifications and machine output.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Running => "running",
            TaskStatus::Exited { .. } => "exited",
            TaskStatus::Stopped => "stopped",
            TaskStatus::DeadlineKill => "deadline_kill",
            TaskStatus::Shutdown => "shutdown",
            TaskStatus::InterruptedByProcessExit => "interrupted_by_process_exit",
        }
    }

    /// The exit code, present only for a normally exited task.
    pub fn exit_code(&self) -> Option<i32> {
        match self {
            TaskStatus::Exited { code } => *code,
            _ => None,
        }
    }
}

/// Metadata describing a background task.
#[derive(Debug, Clone)]
pub struct BackgroundTaskInfo {
    /// Session-scoped identity, shape `task:<n>`.
    pub task_id: String,
    /// The command line the task runs.
    pub command: String,
    /// The working directory it was spawned in.
    pub cwd: PathBuf,
    /// Where its bounded output spool lives.
    pub spool_path: PathBuf,
    /// Running, or the terminal state it reached.
    pub status: TaskStatus,
    /// The explicit deadline, when the caller bounded the task.
    pub timeout_ms: Option<u64>,
}

/// Outcome of reading a slice of spooled task output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutputResult {
    /// The task the slice belongs to.
    pub task_id: String,
    /// Status observed when the slice was read.
    pub status: TaskStatus,
    /// The exit code, once the task exited normally.
    pub exit_code: Option<i32>,
    /// Where this slice actually started, clamped to the spool length.
    pub offset: usize,
    /// Pass this back as `offset` to continue reading incrementally.
    pub next_offset: usize,
    /// The decoded slice contents.
    pub output: String,
    /// Whether the spool hit its byte cap and dropped later output.
    pub truncated: bool,
}

/// An entry in the task registry.
struct TaskEntry {
    task_id: String,
    command: String,
    cwd: PathBuf,
    spool_path: PathBuf,
    status: TaskStatus,
    timeout_ms: Option<u64>,
    stop_tx: Option<oneshot::Sender<StopRequest>>,
    completion: watch::Receiver<TaskStatus>,
}

/// One terminal request and the acknowledgement returned after all terminal
/// lifecycle side effects have been published.
struct StopRequest {
    reason: TaskStatus,
    completed: oneshot::Sender<TaskStatus>,
}

/// Session-scoped state in the task registry.
struct SessionTaskState {
    session_handle: Option<SessionHandle>,
    journal: Option<Arc<EventJournal>>,
    spool_dir: PathBuf,
    counter: u64,
    tasks: HashMap<String, TaskEntry>,
    foreground_background_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl SessionTaskState {
    fn new(spool_dir: PathBuf) -> Self {
        Self {
            session_handle: None,
            journal: None,
            spool_dir,
            counter: 0,
            tasks: HashMap::new(),
            foreground_background_tx: None,
        }
    }

    fn next_task_id(&mut self) -> String {
        self.counter += 1;
        format!("task:{}", self.counter)
    }
}

/// One host-owned registry managing background shell tasks.
#[derive(Default)]
pub struct BackgroundTaskRegistry {
    sessions: RwLock<HashMap<SessionId, Arc<Mutex<SessionTaskState>>>>,
}

impl fmt::Debug for BackgroundTaskRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sessions = self.sessions.read().map(|sessions| sessions.len()).ok();
        f.debug_struct("BackgroundTaskRegistry")
            .field("session_count", &sessions)
            .finish()
    }
}

impl BackgroundTaskRegistry {
    /// Creates an isolated registry for one composed Smith host.
    pub fn new() -> Self {
        Self::default()
    }

    fn get_or_create_session(&self, session_id: &SessionId) -> Arc<Mutex<SessionTaskState>> {
        let mut map = self.sessions.write().unwrap();
        map.entry(session_id.clone())
            .or_insert_with(|| {
                let default_spool = std::env::temp_dir().join(format!("smith_tasks_{session_id}"));
                Arc::new(Mutex::new(SessionTaskState::new(default_spool)))
            })
            .clone()
    }

    /// Binds a session's inbox handle, journal, and spool directory so tasks
    /// started later can notify and persist; hosts call this at session start,
    /// before any task can exist.
    pub fn register_session_context(
        &self,
        session_id: &SessionId,
        session_handle: Option<SessionHandle>,
        journal: Option<Arc<EventJournal>>,
        spool_dir: PathBuf,
    ) {
        let session = self.get_or_create_session(session_id);
        let mut state = session.lock().unwrap();
        state.session_handle = session_handle;
        state.journal = journal;
        state.spool_dir = spool_dir;
    }

    /// Spawns a command as a background shell task.
    pub async fn spawn_background_task(
        &self,
        session_id: &SessionId,
        command: String,
        cwd: PathBuf,
        timeout_ms: Option<u64>,
    ) -> Result<(String, String), RuntimeError> {
        let session = self.get_or_create_session(session_id);

        let (task_id, spool_dir, journal, session_handle) = {
            let mut state = session.lock().unwrap();
            let id = state.next_task_id();
            (
                id,
                state.spool_dir.clone(),
                state.journal.clone(),
                state.session_handle.clone(),
            )
        };

        tokio::fs::create_dir_all(&spool_dir).await.map_err(|e| {
            RuntimeError::new(ErrorKind::Internal, format!("cannot create spool dir: {e}"))
        })?;

        let sanitized_id = task_id.replace(':', "_");
        let spool_path = spool_dir.join(format!("{sanitized_id}.log"));

        let mut child = spawn_shell_cmd(&command, &cwd)?;
        let group_pid = child.id();

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let source = OutputSource::Streams { stdout, stderr };

        if let Some(j) = &journal {
            let _ = j.record_task_started(&task_id).await;
        }

        let (stop_tx, stop_rx) = oneshot::channel::<StopRequest>();
        let (completion_tx, completion) = watch::channel(TaskStatus::Running);

        {
            let mut state = session.lock().unwrap();
            state.tasks.insert(
                task_id.clone(),
                TaskEntry {
                    task_id: task_id.clone(),
                    command: command.clone(),
                    cwd: cwd.clone(),
                    spool_path: spool_path.clone(),
                    status: TaskStatus::Running,
                    timeout_ms,
                    stop_tx: Some(stop_tx),
                    completion,
                },
            );
        }

        let task_id_clone = task_id.clone();
        let session_clone = session.clone();

        tokio::spawn(async move {
            run_task_worker_append(
                session_clone,
                task_id_clone,
                child,
                group_pid,
                source,
                spool_path,
                0,
                timeout_ms,
                stop_rx,
                completion_tx,
                journal,
                session_handle,
            )
            .await;
        });

        let spool_ref = format!("tasks/{sanitized_id}.log");
        Ok((task_id, spool_ref))
    }

    /// Adopts an already-running child process as a background task.
    ///
    /// The child's stdout/stderr are *not* taken here: by the time a manual
    /// backgrounding signal fires, the foreground `shell` invocation's own
    /// reader tasks already took them, and they are still draining into
    /// `lines`. Taking them again would find nothing and the spool would go
    /// silent from this point on — this is the fix for that bug. `lines`
    /// keeps being drained by the worker exactly as if it had opened the
    /// streams itself.
    // Every parameter is independently meaningful to the handoff; a wrapper
    // struct would just relocate the same fields, not simplify them.
    #[allow(clippy::too_many_arguments)]
    pub async fn adopt_foreground_task(
        &self,
        session_id: &SessionId,
        command: String,
        cwd: PathBuf,
        child: Child,
        group_pid: Option<u32>,
        captured_so_far: &str,
        lines: mpsc::Receiver<String>,
        timeout_ms: Option<u64>,
    ) -> Result<(String, String), RuntimeError> {
        let session = self.get_or_create_session(session_id);

        let (task_id, spool_dir, journal, session_handle) = {
            let mut state = session.lock().unwrap();
            let id = state.next_task_id();
            (
                id,
                state.spool_dir.clone(),
                state.journal.clone(),
                state.session_handle.clone(),
            )
        };

        tokio::fs::create_dir_all(&spool_dir).await.map_err(|e| {
            RuntimeError::new(ErrorKind::Internal, format!("cannot create spool dir: {e}"))
        })?;

        let sanitized_id = task_id.replace(':', "_");
        let spool_path = spool_dir.join(format!("{sanitized_id}.log"));

        tokio::fs::write(&spool_path, captured_so_far)
            .await
            .map_err(|e| {
                RuntimeError::new(
                    ErrorKind::Internal,
                    format!("cannot write spool header: {e}"),
                )
            })?;

        if let Some(j) = &journal {
            let _ = j.record_task_started(&task_id).await;
        }

        let (stop_tx, stop_rx) = oneshot::channel::<StopRequest>();
        let (completion_tx, completion) = watch::channel(TaskStatus::Running);

        {
            let mut state = session.lock().unwrap();
            state.tasks.insert(
                task_id.clone(),
                TaskEntry {
                    task_id: task_id.clone(),
                    command: command.clone(),
                    cwd: cwd.clone(),
                    spool_path: spool_path.clone(),
                    status: TaskStatus::Running,
                    timeout_ms,
                    stop_tx: Some(stop_tx),
                    completion,
                },
            );
        }

        let task_id_clone = task_id.clone();
        let session_clone = session.clone();
        // Measured before the spawn: the borrow cannot outlive this method.
        let initial_bytes = captured_so_far.len();

        tokio::spawn(async move {
            run_task_worker_append(
                session_clone,
                task_id_clone,
                child,
                group_pid,
                OutputSource::Lines(lines),
                spool_path,
                initial_bytes,
                timeout_ms,
                stop_rx,
                completion_tx,
                journal,
                session_handle,
            )
            .await;
        });

        let spool_ref = format!("tasks/{sanitized_id}.log");
        Ok((task_id, spool_ref))
    }

    /// Queries output slice for a task.
    pub async fn get_task_output(
        &self,
        session_id: &SessionId,
        task_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<TaskOutputResult, RuntimeError> {
        let session = self.get_or_create_session(session_id);
        let (status, spool_path) = {
            let state = session.lock().unwrap();
            let entry = state.tasks.get(task_id).ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::Tool,
                    format!("unknown background task ID: {task_id}"),
                )
            })?;
            (entry.status.clone(), entry.spool_path.clone())
        };

        let limit = if limit == 0 { 65536 } else { limit };

        let content = if spool_path.exists() {
            tokio::fs::read(&spool_path).await.unwrap_or_default()
        } else {
            Vec::new()
        };

        let file_len = content.len();
        let start = offset.min(file_len);
        let end = (start + limit).min(file_len);
        let slice = &content[start..end];
        let output_text = String::from_utf8_lossy(slice).to_string();
        let truncated = content.ends_with(b"[output truncated at 8388608 bytes]\n");

        Ok(TaskOutputResult {
            task_id: task_id.to_string(),
            status: status.clone(),
            exit_code: status.exit_code(),
            offset: start,
            next_offset: end,
            output: output_text,
            truncated,
        })
    }

    /// Stops a background task by ID. Idempotent if already terminal.
    pub async fn stop_task(
        &self,
        session_id: &SessionId,
        task_id: &str,
    ) -> Result<TaskStatus, RuntimeError> {
        let session = self.get_or_create_session(session_id);
        let (stop_tx, completion) = {
            let mut state = session.lock().unwrap();
            let entry = state.tasks.get_mut(task_id).ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::Tool,
                    format!("unknown background task ID: {task_id}"),
                )
            })?;

            if entry.status.is_terminal() {
                return Ok(entry.status.clone());
            }

            (entry.stop_tx.take(), entry.completion.clone())
        };

        if let Some(tx) = stop_tx {
            let (completed, acknowledgement) = oneshot::channel();
            let request = StopRequest {
                reason: TaskStatus::Stopped,
                completed,
            };
            let _ = tx.send(request);

            return await_terminal_acknowledgement(
                session_id,
                task_id,
                completion,
                Some(acknowledgement),
            )
            .await;
        }

        await_terminal_acknowledgement(session_id, task_id, completion, None).await
    }

    /// Registers a channel to signal manual backgrounding on a foreground shell call.
    pub fn register_foreground_signal(
        &self,
        session_id: &SessionId,
        tx: tokio::sync::oneshot::Sender<()>,
    ) {
        let session = self.get_or_create_session(session_id);
        let mut state = session.lock().unwrap();
        state.foreground_background_tx = Some(tx);
    }

    /// Triggers manual backgrounding on any active foreground shell call in the session.
    pub fn trigger_manual_backgrounding(&self, session_id: &SessionId) -> bool {
        let session = self.get_or_create_session(session_id);
        let tx = {
            let mut state = session.lock().unwrap();
            state.foreground_background_tx.take()
        };
        if let Some(tx) = tx {
            tx.send(()).is_ok()
        } else {
            false
        }
    }

    /// Lists running background tasks for a session.
    pub fn running_tasks(&self, session_id: &SessionId) -> Vec<BackgroundTaskInfo> {
        let session = self.get_or_create_session(session_id);
        let state = session.lock().unwrap();
        state
            .tasks
            .values()
            .filter(|t| !t.status.is_terminal())
            .map(|t| BackgroundTaskInfo {
                task_id: t.task_id.clone(),
                command: t.command.clone(),
                cwd: t.cwd.clone(),
                spool_path: t.spool_path.clone(),
                status: t.status.clone(),
                timeout_ms: t.timeout_ms,
            })
            .collect()
    }

    /// Stops all running background tasks for a session.
    pub fn stop_all_session_tasks(&self, session_id: &SessionId, reason: TaskStatus) {
        let session = self.get_or_create_session(session_id);
        let txs: Vec<oneshot::Sender<StopRequest>> = {
            let mut state = session.lock().unwrap();
            state
                .tasks
                .values_mut()
                .filter_map(|t| {
                    if !t.status.is_terminal() {
                        t.stop_tx.take()
                    } else {
                        None
                    }
                })
                .collect()
        };

        for tx in txs {
            let (completed, _acknowledgement) = oneshot::channel();
            let _ = tx.send(StopRequest {
                reason: reason.clone(),
                completed,
            });
        }
    }
}

async fn await_terminal_acknowledgement(
    session_id: &SessionId,
    task_id: &str,
    mut completion: watch::Receiver<TaskStatus>,
    acknowledgement: Option<oneshot::Receiver<TaskStatus>>,
) -> Result<TaskStatus, RuntimeError> {
    let wait = async {
        if let Some(acknowledgement) = acknowledgement
            && let Ok(status) = acknowledgement.await
        {
            return Ok(status);
        }

        loop {
            let status = completion.borrow().clone();
            if status.is_terminal() {
                return Ok(status);
            }
            completion.changed().await.map_err(|_| {
                RuntimeError::new(
                    ErrorKind::Internal,
                    format!(
                        "background task {task_id} in session {session_id} lost its completion signal"
                    ),
                )
            })?;
        }
    };

    tokio::time::timeout(STOP_ACK_TIMEOUT, wait)
        .await
        .map_err(|_| {
            RuntimeError::new(
                ErrorKind::Tool,
                format!(
                    "background task {task_id} did not confirm process cleanup within {}ms",
                    STOP_ACK_TIMEOUT.as_millis()
                ),
            )
        })?
}

impl From<TaskStatus> for smith_tools::background::BackgroundTaskStatus {
    fn from(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Running => Self::Running,
            TaskStatus::Exited { code } => Self::Exited { code },
            TaskStatus::Stopped => Self::Stopped,
            TaskStatus::DeadlineKill => Self::DeadlineKill,
            TaskStatus::Shutdown => Self::Shutdown,
            TaskStatus::InterruptedByProcessExit => Self::InterruptedByProcessExit,
        }
    }
}

/// Factory-owned background services shared by a runtime's tools and host
/// lifecycle, but isolated from every other composed runtime in the process.
#[derive(Debug, Clone)]
pub struct BackgroundServices {
    registry: Arc<BackgroundTaskRegistry>,
    host: Arc<dyn smith_tools::background::BackgroundTaskHost>,
}

impl BackgroundServices {
    /// Creates one isolated registry and its tool-facing adapter.
    pub fn new() -> Self {
        let registry = Arc::new(BackgroundTaskRegistry::new());
        let host = Arc::new(RegistryBackgroundTaskHost::new(registry.clone()));
        Self { registry, host }
    }

    /// Concrete lifecycle service used for registration, exit policy, and
    /// shutdown.
    pub fn registry(&self) -> &Arc<BackgroundTaskRegistry> {
        &self.registry
    }

    /// Narrow adapter injected into background-capable built-in tools.
    pub fn host(&self) -> Arc<dyn smith_tools::background::BackgroundTaskHost> {
        self.host.clone()
    }
}

impl Default for BackgroundServices {
    fn default() -> Self {
        Self::new()
    }
}

/// Adapts one explicitly owned [`BackgroundTaskRegistry`] to the tool seam.
#[derive(Debug)]
pub struct RegistryBackgroundTaskHost {
    registry: Arc<BackgroundTaskRegistry>,
}

impl RegistryBackgroundTaskHost {
    /// Binds the adapter to an exact host-owned registry instance.
    pub fn new(registry: Arc<BackgroundTaskRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl smith_tools::background::BackgroundTaskHost for RegistryBackgroundTaskHost {
    async fn spawn(
        &self,
        session: SessionId,
        command: String,
        cwd: PathBuf,
        timeout_ms: Option<u64>,
    ) -> Result<smith_tools::background::SpawnedTask, RuntimeError> {
        let (task_id, spool_ref) = self
            .registry
            .spawn_background_task(&session, command, cwd, timeout_ms)
            .await?;
        Ok(smith_tools::background::SpawnedTask { task_id, spool_ref })
    }

    async fn adopt(
        &self,
        session: SessionId,
        command: String,
        cwd: PathBuf,
        child: Child,
        group_pid: Option<u32>,
        captured_so_far: String,
        lines: mpsc::Receiver<String>,
    ) -> Result<smith_tools::background::SpawnedTask, RuntimeError> {
        let (task_id, spool_ref) = self
            .registry
            .adopt_foreground_task(
                &session,
                command,
                cwd,
                child,
                group_pid,
                &captured_so_far,
                lines,
                // Manual backgrounding hands off with no deadline: the
                // foreground call's own `timeout_ms` governed the call it no
                // longer is, and background tasks default to none anyway.
                None,
            )
            .await?;
        Ok(smith_tools::background::SpawnedTask { task_id, spool_ref })
    }

    async fn output(
        &self,
        session: SessionId,
        task_id: String,
        offset: usize,
        limit: usize,
    ) -> Result<smith_tools::background::BackgroundTaskOutput, RuntimeError> {
        let result = self
            .registry
            .get_task_output(&session, &task_id, offset, limit)
            .await?;
        Ok(smith_tools::background::BackgroundTaskOutput {
            status: result.status.into(),
            exit_code: result.exit_code,
            offset: result.offset,
            next_offset: result.next_offset,
            output: result.output,
            truncated: result.truncated,
        })
    }

    async fn stop(
        &self,
        session: SessionId,
        task_id: String,
    ) -> Result<smith_tools::background::BackgroundTaskStatus, RuntimeError> {
        let status = self.registry.stop_task(&session, &task_id).await?;
        Ok(status.into())
    }

    fn register_foreground_signal(&self, session: SessionId) -> Option<oneshot::Receiver<()>> {
        let (tx, rx) = oneshot::channel();
        self.registry.register_foreground_signal(&session, tx);
        Some(rx)
    }
}

fn spawn_shell_cmd(command: &str, cwd: &Path) -> Result<Child, RuntimeError> {
    let mut builder = Command::new("sh");
    builder
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    #[cfg(unix)]
    builder.process_group(0);

    builder
        .spawn()
        .map_err(|err| RuntimeError::new(ErrorKind::Tool, format!("cannot run `{command}`: {err}")))
}

#[cfg(unix)]
async fn stop_process_group(child: &mut Child, group: Option<u32>) {
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;

    let Some(group) = group.and_then(|id| i32::try_from(id).ok()) else {
        let _ = child.kill().await;
        return;
    };
    let pid = Pid::from_raw(group);

    let _ = killpg(pid, Signal::SIGTERM);
    if tokio::time::timeout(GRACE, child.wait()).await.is_ok() {
        return;
    }
    let _ = killpg(pid, Signal::SIGKILL);
    let _ = child.wait().await;
}

#[cfg(not(unix))]
async fn stop_process_group(child: &mut Child, _group: Option<u32>) {
    let _ = child.kill().await;
}

/// Where a task worker's output comes from.
///
/// `spawn_background_task` owns a freshly spawned child whose stdout/stderr
/// have never been read, so it hands over the raw streams and this function
/// starts its own reader tasks. `adopt_foreground_task` instead hands over a
/// line channel the foreground `shell` invocation's own reader tasks are
/// already feeding — the child's streams were taken before adoption, so
/// there is nothing left to read from them a second time.
enum OutputSource {
    Streams {
        stdout: Option<tokio::process::ChildStdout>,
        stderr: Option<tokio::process::ChildStderr>,
    },
    Lines(mpsc::Receiver<String>),
}

// One parameter per independently meaningful piece of a task's identity and
// lifecycle wiring; already reduced by one when `stdout`/`stderr` collapsed
// into `source` above.
#[allow(clippy::too_many_arguments)]
async fn run_task_worker_append(
    session: Arc<Mutex<SessionTaskState>>,
    task_id: String,
    mut child: Child,
    group_pid: Option<u32>,
    source: OutputSource,
    spool_path: PathBuf,
    initial_bytes: usize,
    timeout_ms: Option<u64>,
    mut stop_rx: oneshot::Receiver<StopRequest>,
    completion_tx: watch::Sender<TaskStatus>,
    journal: Option<Arc<EventJournal>>,
    session_handle: Option<SessionHandle>,
) {
    let mut lines_rx = match source {
        OutputSource::Streams { stdout, stderr } => {
            let (lines_tx, lines_rx) = mpsc::channel::<String>(256);
            if let Some(stdout) = stdout {
                let tx = lines_tx.clone();
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = reader.next_line().await {
                        if tx.send(line).await.is_err() {
                            break;
                        }
                    }
                });
            }

            if let Some(stderr) = stderr {
                let tx = lines_tx;
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = reader.next_line().await {
                        if tx.send(line).await.is_err() {
                            break;
                        }
                    }
                });
            }
            lines_rx
        }
        OutputSource::Lines(lines_rx) => lines_rx,
    };

    let spool_path_clone = spool_path.clone();
    let writer_handle = tokio::spawn(async move {
        let mut bytes_written = initial_bytes;
        let mut truncated = false;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&spool_path_clone)
            .await
            .ok();

        while let Some(line) = lines_rx.recv().await {
            if let Some(ref mut f) = file {
                let line_len = line.len() + 1;
                if bytes_written + line_len > MAX_SPOOL_BYTES {
                    if !truncated {
                        let _ = f
                            .write_all(b"\n[output truncated at 8388608 bytes]\n")
                            .await;
                        truncated = true;
                    }
                    continue;
                }
                let _ = f.write_all(line.as_bytes()).await;
                let _ = f.write_all(b"\n").await;
                bytes_written += line_len;
            }
        }
    });

    let timeout_fut = async {
        if let Some(ms) = timeout_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            true
        } else {
            futures_util::future::pending::<bool>().await
        }
    };

    let (terminal_status, stop_acknowledgement) = tokio::select! {
        status = child.wait() => {
            let code = status.ok().and_then(|s| s.code());
            (TaskStatus::Exited { code }, None)
        }
        request = &mut stop_rx => {
            stop_process_group(&mut child, group_pid).await;
            match request {
                Ok(request) => (request.reason, Some(request.completed)),
                Err(_) => (TaskStatus::Stopped, None),
            }
        }
        _ = timeout_fut => {
            stop_process_group(&mut child, group_pid).await;
            (TaskStatus::DeadlineKill, None)
        }
    };

    let _ = writer_handle.await;

    // Record terminal state in registry
    {
        let mut state = session.lock().unwrap();
        if let Some(entry) = state.tasks.get_mut(&task_id) {
            entry.status = terminal_status.clone();
        }
    }

    // Journal record
    if let Some(j) = &journal {
        let _ = j.record_task_exited(&task_id).await;
    }

    // Read spool tail for inbox notification
    let tail = read_spool_tail(&spool_path).await;

    // Inject terminal notification
    if let Some(handle) = &session_handle {
        let notification = serde_json::json!({
            "type": "background_task_terminal",
            "task_id": task_id,
            "status": terminal_status.as_str(),
            "exit_code": terminal_status.exit_code(),
            "output_tail": tail,
        });

        let content = InjectedContent::text(notification.to_string())
            .must_deliver()
            .ordered_by(format!("{task_id}/terminal"));

        let _ = handle.inject(content);
    }

    completion_tx.send_replace(terminal_status.clone());
    if let Some(completed) = stop_acknowledgement {
        let _ = completed.send(terminal_status);
    }
}

async fn read_spool_tail(path: &Path) -> String {
    let Ok(content) = tokio::fs::read(path).await else {
        return "(no output)".to_string();
    };

    if content.is_empty() {
        return "(no output)".to_string();
    }

    let text = String::from_utf8_lossy(&content);
    let trimmed = text.trim_end();
    if trimmed.len() <= 2000 {
        trimmed.to_string()
    } else {
        format!("...[truncated]\n{}", &trimmed[trimmed.len() - 2000..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh session ID per test keeps spool paths unambiguous in diagnostics.
    fn unique_session() -> SessionId {
        SessionId::new(format!("bg-registry-test-{}", uuid::Uuid::new_v4()))
    }

    /// Polls `get_task_output` until the task reaches a terminal state.
    ///
    /// Every worker transition (exit, deadline kill, stop) is asynchronous
    /// relative to the call that triggers it, so tests assert on the settled
    /// state rather than racing the registry's own bookkeeping.
    async fn wait_for_terminal(
        registry: &BackgroundTaskRegistry,
        session: &SessionId,
        task_id: &str,
    ) -> TaskStatus {
        for _ in 0..500 {
            let result = registry
                .get_task_output(session, task_id, 0, 1)
                .await
                .unwrap();
            if result.status.is_terminal() {
                return result.status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("task {task_id} did not reach a terminal state in time");
    }

    #[tokio::test]
    async fn a_background_spawn_returns_promptly_and_the_process_keeps_running() {
        let registry = BackgroundTaskRegistry::new();
        let session = unique_session();
        let started = std::time::Instant::now();
        let (task_id, spool_ref) = registry
            .spawn_background_task(&session, "sleep 0.5".into(), std::env::temp_dir(), None)
            .await
            .unwrap();
        // The call must not block for anything close to the command's own
        // runtime — that is the entire point of backgrounding it.
        assert!(
            started.elapsed() < Duration::from_millis(300),
            "{:?}",
            started.elapsed()
        );
        assert!(
            spool_ref.contains(&task_id.replace(':', "_")),
            "{spool_ref}"
        );

        let running = registry.running_tasks(&session);
        assert!(
            running
                .iter()
                .any(|t| t.task_id == task_id && t.status == TaskStatus::Running),
            "the process must still be running right after the call resolves: {running:?}"
        );

        let status = wait_for_terminal(&registry, &session, &task_id).await;
        assert_eq!(status, TaskStatus::Exited { code: Some(0) });
    }

    #[tokio::test]
    async fn an_explicit_deadline_kills_a_background_task() {
        let registry = BackgroundTaskRegistry::new();
        let session = unique_session();
        let (task_id, _) = registry
            .spawn_background_task(&session, "sleep 30".into(), std::env::temp_dir(), Some(150))
            .await
            .unwrap();

        let status = wait_for_terminal(&registry, &session, &task_id).await;
        assert_eq!(status, TaskStatus::DeadlineKill);
        // This is exactly the word `run_task_worker_append` puts in the
        // terminal notification's `status` field, so proving it here proves
        // the notification states a deadline kill too.
        assert_eq!(status.as_str(), "deadline_kill");
    }

    #[tokio::test]
    async fn stop_task_is_idempotent_and_unknown_ids_report_a_stable_error() {
        let registry = BackgroundTaskRegistry::new();
        let session = unique_session();
        let (task_id, _) = registry
            .spawn_background_task(&session, "sleep 30".into(), std::env::temp_dir(), None)
            .await
            .unwrap();

        let status = registry.stop_task(&session, &task_id).await.unwrap();
        assert_eq!(status, TaskStatus::Stopped);

        // Now that the task is terminal, stopping it again must report the
        // existing state rather than erroring or re-signaling a process that
        // is already gone.
        let again = registry.stop_task(&session, &task_id).await.unwrap();
        assert_eq!(again, TaskStatus::Stopped);

        let unknown_stop = registry
            .stop_task(&session, "task:does-not-exist")
            .await
            .unwrap_err();
        assert!(
            unknown_stop
                .message
                .contains("unknown background task ID: task:does-not-exist"),
            "{unknown_stop:?}"
        );

        let unknown_output = registry
            .get_task_output(&session, "task:does-not-exist", 0, 100)
            .await
            .unwrap_err();
        assert!(
            unknown_output
                .message
                .contains("unknown background task ID: task:does-not-exist"),
            "{unknown_output:?}"
        );
    }

    #[tokio::test]
    async fn concurrent_stops_converge_on_one_acknowledged_terminal_state() {
        let registry = Arc::new(BackgroundTaskRegistry::new());
        let session = unique_session();
        let (task_id, _) = registry
            .spawn_background_task(&session, "sleep 30".into(), std::env::temp_dir(), None)
            .await
            .unwrap();

        let first_registry = registry.clone();
        let first_session = session.clone();
        let first_task = task_id.clone();
        let first =
            tokio::spawn(
                async move { first_registry.stop_task(&first_session, &first_task).await },
            );
        let second_registry = registry.clone();
        let second_session = session.clone();
        let second_task = task_id.clone();
        let second = tokio::spawn(async move {
            second_registry
                .stop_task(&second_session, &second_task)
                .await
        });

        let first = first.await.unwrap().unwrap();
        let second = second.await.unwrap().unwrap();
        assert_eq!(first, TaskStatus::Stopped);
        assert_eq!(second, first);
    }

    #[tokio::test]
    async fn natural_exit_and_deadline_races_return_the_single_winning_terminal_state() {
        let registry = BackgroundTaskRegistry::new();
        let session = unique_session();

        let (natural, _) = registry
            .spawn_background_task(&session, "true".into(), std::env::temp_dir(), None)
            .await
            .unwrap();
        let natural_status = registry.stop_task(&session, &natural).await.unwrap();
        assert!(natural_status.is_terminal());

        let (deadline, _) = registry
            .spawn_background_task(&session, "sleep 30".into(), std::env::temp_dir(), Some(1))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1)).await;
        let deadline_status = registry.stop_task(&session, &deadline).await.unwrap();
        assert!(deadline_status.is_terminal());
        assert!(matches!(
            deadline_status,
            TaskStatus::Stopped | TaskStatus::DeadlineKill
        ));
    }

    #[tokio::test]
    async fn shutdown_and_tool_stop_share_the_worker_terminal_transition() {
        let registry = BackgroundTaskRegistry::new();
        let session = unique_session();
        let (task_id, _) = registry
            .spawn_background_task(&session, "sleep 30".into(), std::env::temp_dir(), None)
            .await
            .unwrap();

        registry.stop_all_session_tasks(&session, TaskStatus::Shutdown);
        let status = registry.stop_task(&session, &task_id).await.unwrap();
        assert_eq!(status, TaskStatus::Shutdown);
    }

    #[tokio::test]
    async fn an_unconfirmed_cleanup_times_out_instead_of_reporting_running() {
        let registry = BackgroundTaskRegistry::new();
        let session = unique_session();
        let state = registry.get_or_create_session(&session);
        let (_completion_tx, completion) = watch::channel(TaskStatus::Running);
        {
            let mut state = state.lock().unwrap();
            state.tasks.insert(
                "task:stuck".into(),
                TaskEntry {
                    task_id: "task:stuck".into(),
                    command: "stuck".into(),
                    cwd: std::env::temp_dir(),
                    spool_path: std::env::temp_dir().join("stuck.log"),
                    status: TaskStatus::Running,
                    timeout_ms: None,
                    stop_tx: None,
                    completion,
                },
            );
        }

        let error = registry
            .stop_task(&session, "task:stuck")
            .await
            .expect_err("an unconfirmed cleanup must fail");
        assert_eq!(error.kind, ErrorKind::Tool);
        assert!(error.message.contains("did not confirm process cleanup"));
    }

    #[tokio::test]
    async fn two_service_bundles_isolate_task_ids_spools_signals_and_shutdown() {
        let first = BackgroundServices::new();
        let second = BackgroundServices::new();
        let session = SessionId::new("same-session-id");
        let first_spool = tempfile::tempdir().unwrap();
        let second_spool = tempfile::tempdir().unwrap();
        first.registry().register_session_context(
            &session,
            None,
            None,
            first_spool.path().to_path_buf(),
        );
        second.registry().register_session_context(
            &session,
            None,
            None,
            second_spool.path().to_path_buf(),
        );

        let first_signal = first
            .host()
            .register_foreground_signal(session.clone())
            .unwrap();
        let mut second_signal = second
            .host()
            .register_foreground_signal(session.clone())
            .unwrap();
        assert!(first.registry().trigger_manual_backgrounding(&session));
        first_signal.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut second_signal)
                .await
                .is_err()
        );
        assert!(second.registry().trigger_manual_backgrounding(&session));
        second_signal.await.unwrap();

        let (first_task, _) = first
            .registry()
            .spawn_background_task(&session, "sleep 30".into(), std::env::temp_dir(), None)
            .await
            .unwrap();
        let (second_task, _) = second
            .registry()
            .spawn_background_task(&session, "sleep 30".into(), std::env::temp_dir(), None)
            .await
            .unwrap();
        assert_eq!(first_task, "task:1");
        assert_eq!(second_task, "task:1");
        assert!(
            first.registry().running_tasks(&session)[0]
                .spool_path
                .starts_with(first_spool.path())
        );
        assert!(
            second.registry().running_tasks(&session)[0]
                .spool_path
                .starts_with(second_spool.path())
        );

        first
            .registry()
            .stop_all_session_tasks(&session, TaskStatus::Shutdown);
        let first_status = first
            .registry()
            .stop_task(&session, &first_task)
            .await
            .unwrap();
        assert_eq!(first_status, TaskStatus::Shutdown);
        assert_eq!(second.registry().running_tasks(&session).len(), 1);
        assert_eq!(
            second
                .registry()
                .stop_task(&session, &second_task)
                .await
                .unwrap(),
            TaskStatus::Stopped
        );
    }

    #[tokio::test]
    async fn spool_output_is_capped_with_an_explicit_truncation_marker() {
        let registry = BackgroundTaskRegistry::new();
        let session = unique_session();
        let bytes = MAX_SPOOL_BYTES + 1024 * 1024;
        // The spool writer does one file write per *line* (unlike `shell`'s
        // in-memory collector), so this uses long lines rather than `yes`'s
        // usual short ones — otherwise ~9MB of ~20-byte lines means hundreds
        // of thousands of tiny disk writes and a test that times out on
        // nothing but I/O overhead.
        let long_line = "a".repeat(8_000);
        let command = format!("yes '{long_line}' | head -c {bytes}");
        let (task_id, _) = registry
            .spawn_background_task(&session, command, std::env::temp_dir(), None)
            .await
            .unwrap();

        let status = wait_for_terminal(&registry, &session, &task_id).await;
        assert!(matches!(status, TaskStatus::Exited { .. }), "{status:?}");

        let result = registry
            .get_task_output(&session, &task_id, 0, MAX_SPOOL_BYTES + 65536)
            .await
            .unwrap();
        assert!(result.truncated, "expected the spool to report truncation");
        assert!(
            result.output.len() <= MAX_SPOOL_BYTES + 4096,
            "spool grew past its cap: {} bytes",
            result.output.len()
        );
    }

    #[tokio::test]
    async fn adoption_carries_the_captured_prefix_and_keeps_draining_the_live_receiver() {
        let registry = BackgroundTaskRegistry::new();
        let session = unique_session();

        // Stands in for `shell.rs`'s own reader tasks: by the time a real
        // manual-backgrounding signal fires, the child's stdout has already
        // been taken and is being drained into a channel exactly like this
        // one — adoption must keep consuming that channel, not re-take the
        // (already-empty) stream.
        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 0.2; echo after-adopt")
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel::<String>(16);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx.send(line).await.is_err() {
                    break;
                }
            }
        });
        let group_pid = child.id();

        let (task_id, _) = registry
            .adopt_foreground_task(
                &session,
                "sleep 0.2; echo after-adopt".into(),
                std::env::temp_dir(),
                child,
                group_pid,
                "prefix output already captured\n",
                rx,
                None,
            )
            .await
            .unwrap();

        let status = wait_for_terminal(&registry, &session, &task_id).await;
        assert!(
            matches!(status, TaskStatus::Exited { code: Some(0) }),
            "{status:?}"
        );

        let result = registry
            .get_task_output(&session, &task_id, 0, 65536)
            .await
            .unwrap();
        assert!(
            result.output.contains("prefix output already captured"),
            "{}",
            result.output
        );
        assert!(result.output.contains("after-adopt"), "{}", result.output);
    }
}
