//! Background-exit policy decisions, task lifecycle handling, and reports.

use super::*;

/// One background shell task's state as the background-exit policy last
/// observed it.
#[derive(Debug, Serialize)]
pub(super) struct BackgroundTaskOutput {
    pub(super) task_id: String,
    pub(super) status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) exit_code: Option<i32>,
}

impl BackgroundTaskOutput {
    /// Projects a task the `error` policy reported without waiting on: no
    /// poll happened, so "running" is the only state actually observed.
    pub(super) fn still_running(task: &BackgroundTaskInfo) -> Self {
        Self {
            task_id: task.task_id.clone(),
            status: TaskStatus::Running.as_str(),
            exit_code: None,
        }
    }

    pub(super) fn terminal(task_id: String, status: &TaskStatus) -> Self {
        Self {
            task_id,
            status: status.as_str(),
            exit_code: status.exit_code(),
        }
    }
}

/// Report of every background shell task the caller's background-exit policy
/// acted on, and which policy applied.
#[derive(Debug, Serialize)]
pub(super) struct BackgroundExitOutput {
    pub(super) policy: &'static str,
    pub(super) tasks: Vec<BackgroundTaskOutput>,
}
/// What a background-exit policy requires when the final answer is ready but
/// background shell tasks are still running. Pure and synchronous so the
/// policy choice is unit-testable without a live task registry; the async
/// waiting/stopping itself lives in [`apply_background_exit_policy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BackgroundExitDecision {
    /// No running background shell tasks; nothing to do.
    Clear,
    /// `error` (the default): report every running task and fail rather than
    /// let `host.shutdown()` kill them without anyone having been told.
    Error(String),
    /// `wait`: block until every running task reaches a terminal state.
    Wait,
    /// `stop`: signal every running task to stop, then await termination.
    Stop,
}

pub(super) fn decide_background_exit(
    policy: BackgroundExit,
    running: &[BackgroundTaskInfo],
) -> BackgroundExitDecision {
    if running.is_empty() {
        return BackgroundExitDecision::Clear;
    }
    match policy {
        BackgroundExit::Error => BackgroundExitDecision::Error(background_task_error(running)),
        BackgroundExit::Wait => BackgroundExitDecision::Wait,
        BackgroundExit::Stop => BackgroundExitDecision::Stop,
    }
}

/// Names every running task by ID and command so the caller can act on it
/// instead of guessing what `host.shutdown()` is about to kill.
pub(super) fn background_task_error(running: &[BackgroundTaskInfo]) -> String {
    let tasks = running
        .iter()
        .map(|task| format!("{} (`{}`)", task.task_id, task.command))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{} background shell task(s) still running at exit: {tasks}; rerun with \
         `--background-exit wait` to await them, `--background-exit stop` to end them, \
         or call `task_stop` before the turn ends",
        running.len()
    )
}

/// Applies the background-exit policy to whatever background shell tasks are
/// still running when the final answer is ready.
///
/// `host.shutdown()` kills every registered group regardless of policy — true
/// orphaning is not possible — so this only decides what gets reported and
/// how long the process waits before that happens.
pub(super) async fn apply_background_exit_policy(
    registry: &BackgroundTaskRegistry,
    session_id: &SessionId,
    policy: BackgroundExit,
) -> (Option<String>, Option<BackgroundExitOutput>) {
    let running = registry.running_tasks(session_id);
    match decide_background_exit(policy, &running) {
        BackgroundExitDecision::Clear => (None, None),
        BackgroundExitDecision::Error(message) => {
            let tasks = running
                .iter()
                .map(BackgroundTaskOutput::still_running)
                .collect();
            (
                Some(message),
                Some(BackgroundExitOutput {
                    policy: policy.as_str(),
                    tasks,
                }),
            )
        }
        BackgroundExitDecision::Wait => {
            let tasks = await_background_tasks(registry, session_id, &running, None).await;
            (
                None,
                Some(BackgroundExitOutput {
                    policy: policy.as_str(),
                    tasks,
                }),
            )
        }
        BackgroundExitDecision::Stop => {
            registry.stop_all_session_tasks(session_id, TaskStatus::Stopped);
            let tasks = await_background_tasks(
                registry,
                session_id,
                &running,
                Some(BACKGROUND_STOP_POLL_BOUND),
            )
            .await;
            (
                None,
                Some(BackgroundExitOutput {
                    policy: policy.as_str(),
                    tasks,
                }),
            )
        }
    }
}

/// Polls the registry until every named task leaves the running set — or,
/// under `bound`, until that ceiling passes — then reads back each task's
/// terminal state. `wait` passes no bound: a background task may legitimately
/// run as long as the model let it; only `stop` needs a ceiling, since its
/// signal should resolve within the worker's own kill grace period.
pub(super) async fn await_background_tasks(
    registry: &BackgroundTaskRegistry,
    session_id: &SessionId,
    running: &[BackgroundTaskInfo],
    bound: Option<Duration>,
) -> Vec<BackgroundTaskOutput> {
    let poll_until_terminal = async {
        loop {
            let running_ids: BTreeSet<String> = registry
                .running_tasks(session_id)
                .into_iter()
                .map(|task| task.task_id)
                .collect();
            if !running
                .iter()
                .any(|task| running_ids.contains(&task.task_id))
            {
                return;
            }
            tokio::time::sleep(BACKGROUND_TASK_POLL_INTERVAL).await;
        }
    };
    match bound {
        Some(bound) => {
            let _ = tokio::time::timeout(bound, poll_until_terminal).await;
        }
        None => poll_until_terminal.await,
    }

    let mut report = Vec::with_capacity(running.len());
    for task in running {
        let status = registry
            .get_task_output(session_id, &task.task_id, 0, 1)
            .await
            .map(|result| result.status)
            .unwrap_or_else(|_| task.status.clone());
        report.push(BackgroundTaskOutput::terminal(
            task.task_id.clone(),
            &status,
        ));
    }
    report
}
