//! Session-owned background task registry and lifecycle management.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use agent_runtime::runtime::{InjectedContent, SessionHandle};
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::ids::SessionId;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::journal::EventJournal;

/// Default grace period before SIGKILLing a process group.
const GRACE: Duration = Duration::from_millis(500);

/// Hard cap on task output spool file size (8 MiB).
pub const MAX_SPOOL_BYTES: usize = 8 * 1024 * 1024;

/// Terminal state or running status of a background task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    Exited { code: Option<i32> },
    Stopped,
    DeadlineKill,
    Shutdown,
    InterruptedByProcessExit,
}

impl TaskStatus {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, TaskStatus::Running)
    }

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
    pub task_id: String,
    pub command: String,
    pub cwd: PathBuf,
    pub spool_path: PathBuf,
    pub status: TaskStatus,
    pub timeout_ms: Option<u64>,
}

/// Outcome of reading a slice of spooled task output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutputResult {
    pub task_id: String,
    pub status: TaskStatus,
    pub exit_code: Option<i32>,
    pub offset: usize,
    pub next_offset: usize,
    pub output: String,
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
    group_pid: Option<u32>,
    stop_tx: Option<tokio::sync::oneshot::Sender<TaskStatus>>,
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

/// Global registry managing background shell tasks.
#[derive(Default)]
pub struct BackgroundTaskRegistry {
    sessions: RwLock<HashMap<SessionId, Arc<Mutex<SessionTaskState>>>>,
}

static REGISTRY: OnceLock<BackgroundTaskRegistry> = OnceLock::new();

impl BackgroundTaskRegistry {
    pub fn global() -> &'static BackgroundTaskRegistry {
        REGISTRY.get_or_init(BackgroundTaskRegistry::default)
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

        if let Some(j) = &journal {
            let _ = j.record_task_started(&task_id).await;
        }

        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<TaskStatus>();

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
                    group_pid,
                    stop_tx: Some(stop_tx),
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
                stdout,
                stderr,
                spool_path,
                0,
                timeout_ms,
                stop_rx,
                journal,
                session_handle,
            )
            .await;
        });

        let spool_ref = format!("tasks/{sanitized_id}.log");
        Ok((task_id, spool_ref))
    }

    /// Adopts an already-running child process as a background task.
    pub async fn adopt_foreground_task(
        &self,
        session_id: &SessionId,
        command: String,
        cwd: PathBuf,
        child: Child,
        group_pid: Option<u32>,
        captured_so_far: &str,
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
                RuntimeError::new(ErrorKind::Internal, format!("cannot write spool header: {e}"))
            })?;

        let mut child = child;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        if let Some(j) = &journal {
            let _ = j.record_task_started(&task_id).await;
        }

        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<TaskStatus>();

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
                    group_pid,
                    stop_tx: Some(stop_tx),
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
                stdout,
                stderr,
                spool_path,
                initial_bytes,
                timeout_ms,
                stop_rx,
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
        let stop_tx = {
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

            entry.stop_tx.take()
        };

        if let Some(tx) = stop_tx {
            let _ = tx.send(TaskStatus::Stopped);
        }

        let state = session.lock().unwrap();
        if let Some(entry) = state.tasks.get(task_id) {
            Ok(entry.status.clone())
        } else {
            Ok(TaskStatus::Stopped)
        }
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
        let txs: Vec<tokio::sync::oneshot::Sender<TaskStatus>> = {
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
            let _ = tx.send(reason.clone());
        }
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
pub async fn stop_process_group(child: &mut Child, group: Option<u32>) {
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
pub async fn stop_process_group(child: &mut Child, _group: Option<u32>) {
    let _ = child.kill().await;
}

async fn run_task_worker_append(
    session: Arc<Mutex<SessionTaskState>>,
    task_id: String,
    mut child: Child,
    group_pid: Option<u32>,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    spool_path: PathBuf,
    initial_bytes: usize,
    timeout_ms: Option<u64>,
    mut stop_rx: tokio::sync::oneshot::Receiver<TaskStatus>,
    journal: Option<Arc<EventJournal>>,
    session_handle: Option<SessionHandle>,
) {
    let (lines_tx, mut lines_rx) = mpsc::channel::<String>(256);

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

    let terminal_status = tokio::select! {
        status = child.wait() => {
            let code = status.ok().and_then(|s| s.code());
            TaskStatus::Exited { code }
        }
        requested_status = &mut stop_rx => {
            stop_process_group(&mut child, group_pid).await;
            requested_status.unwrap_or(TaskStatus::Stopped)
        }
        _ = timeout_fut => {
            stop_process_group(&mut child, group_pid).await;
            TaskStatus::DeadlineKill
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
