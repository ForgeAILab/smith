//! `shell`'s `run_in_background`, `task_output`, and `task_stop` exercised
//! against explicitly injected fake [`BackgroundTaskHost`] instances.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Deadline, SystemClock};
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId, TurnId};
use agent_runtime_core::tool::{InvocationContext, PreparationContext, Tool};
use async_trait::async_trait;
use serde_json::json;
use smith_host::workspace::ProjectWorkspace;
use smith_tools::background::{
    BackgroundTaskHost, BackgroundTaskOutput, BackgroundTaskStatus, SpawnedTask,
};
use smith_tools::{ShellTool, TaskOutputTool, TaskStopTool};
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};

/// The one task this fake knows about; everything else is "unknown".
const KNOWN_TASK_ID: &str = "task:1";

#[derive(Debug, Default)]
struct FakeHost {
    spawn_calls: Mutex<Vec<(String, Option<u64>)>>,
    stop_calls: Mutex<Vec<String>>,
}

fn unknown_task_error(task_id: &str) -> RuntimeError {
    // The exact stable string the real registry returns — the contract
    // `task_output`/`task_stop` promise the model, independent of which host
    // implements it.
    RuntimeError::new(
        ErrorKind::Tool,
        format!("unknown background task ID: {task_id}"),
    )
}

#[async_trait]
impl BackgroundTaskHost for FakeHost {
    async fn spawn(
        &self,
        _session: SessionId,
        command: String,
        _cwd: PathBuf,
        timeout_ms: Option<u64>,
    ) -> Result<SpawnedTask, RuntimeError> {
        self.spawn_calls.lock().unwrap().push((command, timeout_ms));
        Ok(SpawnedTask {
            task_id: KNOWN_TASK_ID.to_owned(),
            spool_ref: "tasks/task_1.log".to_owned(),
        })
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
        unimplemented!("manual backgrounding is exercised at the registry level, not here")
    }

    async fn output(
        &self,
        _session: SessionId,
        task_id: String,
        offset: usize,
        _limit: usize,
    ) -> Result<BackgroundTaskOutput, RuntimeError> {
        if task_id != KNOWN_TASK_ID {
            return Err(unknown_task_error(&task_id));
        }
        Ok(BackgroundTaskOutput {
            status: BackgroundTaskStatus::Running,
            exit_code: None,
            offset,
            next_offset: offset + 5,
            output: "hello".to_owned(),
            truncated: false,
        })
    }

    async fn stop(
        &self,
        _session: SessionId,
        task_id: String,
    ) -> Result<BackgroundTaskStatus, RuntimeError> {
        self.stop_calls.lock().unwrap().push(task_id.clone());
        if task_id != KNOWN_TASK_ID {
            return Err(unknown_task_error(&task_id));
        }
        Ok(BackgroundTaskStatus::Stopped)
    }

    fn register_foreground_signal(&self, _session: SessionId) -> Option<oneshot::Receiver<()>> {
        None
    }
}

struct Project {
    _dir: tempfile::TempDir,
    invocation: InvocationContext,
}

impl Project {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temporary project");
        let workspace = Arc::new(ProjectWorkspace::new(dir.path()).expect("a project workspace"));
        Self {
            _dir: dir,
            invocation: InvocationContext {
                session: SessionId::new("session-1"),
                turn: Some(TurnId::new("turn-1")),
                call_id: ToolCallId::new("call-1"),
                request: RequestId::new("request-1"),
                workspace,
                clock: Arc::new(SystemClock),
                cancel: Cancellation::new(),
                deadline: Deadline::never(),
                output_limit: 32_000,
            },
        }
    }

    fn preparation(&self) -> PreparationContext {
        PreparationContext {
            session: self.invocation.session.clone(),
            turn: self.invocation.turn.clone(),
            call_id: self.invocation.call_id.clone(),
            request: self.invocation.request.clone(),
            workspace: self.invocation.workspace.clone(),
            clock: self.invocation.clock.clone(),
            cancel: self.invocation.cancel.clone(),
            deadline: self.invocation.deadline,
        }
    }

    async fn invoke(
        &self,
        tool: &dyn Tool,
        arguments: serde_json::Value,
    ) -> Result<agent_runtime_core::tool::ToolOutcome, RuntimeError> {
        let prepared = tool.prepare(arguments, &self.preparation()).await?;
        tool.invoke(prepared, &self.invocation).await
    }
}

#[tokio::test]
async fn unknown_task_id_reports_the_same_stable_error_from_both_tools() {
    let host = Arc::new(FakeHost::default());
    let output = TaskOutputTool::new(host.clone());
    let stop = TaskStopTool::new(host);
    let project = Project::new();

    let output_err = project
        .invoke(&output, json!({"task_id": "task:missing"}))
        .await
        .unwrap_err();
    assert!(
        output_err
            .message
            .contains("unknown background task ID: task:missing"),
        "{output_err:?}"
    );

    let stop_err = project
        .invoke(&stop, json!({"task_id": "task:missing"}))
        .await
        .unwrap_err();
    assert!(
        stop_err
            .message
            .contains("unknown background task ID: task:missing"),
        "{stop_err:?}"
    );
}

#[tokio::test]
async fn task_output_reports_the_hosts_status_and_output_for_a_known_task() {
    let tool = TaskOutputTool::new(Arc::new(FakeHost::default()));
    let project = Project::new();

    let outcome = project
        .invoke(&tool, json!({"task_id": KNOWN_TASK_ID, "offset": 3}))
        .await
        .expect("a known task resolves");
    assert!(!outcome.is_error);
    assert_eq!(outcome.value["status"], "running");
    assert_eq!(outcome.value["offset"], 3);
    assert_eq!(outcome.value["next_offset"], 8);
}

#[tokio::test]
async fn task_stop_reports_the_hosts_terminal_state_for_a_known_task() {
    let tool = TaskStopTool::new(Arc::new(FakeHost::default()));
    let project = Project::new();

    let outcome = project
        .invoke(&tool, json!({"task_id": KNOWN_TASK_ID}))
        .await
        .expect("a known task resolves");
    assert!(!outcome.is_error);
    assert_eq!(outcome.value["status"], "stopped");
}

#[tokio::test]
async fn run_in_background_returns_promptly_with_the_hosts_task_id() {
    let tool = ShellTool::new(Arc::new(FakeHost::default()));
    let project = Project::new();

    let outcome = project
        .invoke(
            &tool,
            json!({"command": "sleep 30", "run_in_background": true}),
        )
        .await
        .expect("a background spawn resolves without waiting for the command");
    assert!(!outcome.is_error);
    assert_eq!(outcome.value["task_id"], KNOWN_TASK_ID);
    assert_eq!(outcome.value["running"], true);
}
