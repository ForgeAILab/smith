use super::*;

/// Finds process-owned child and monitor work whose latest journal lifecycle
/// has no terminal resolution. Recovery markers participate so a later resume
/// never reports the same interrupted work twice.
///
/// Monitor start/stop records are metadata-only lifecycle seams. This scanner
/// never creates or restarts monitor execution.
pub(super) fn unresolved_ephemeral_work(
    recovery: &JournalRecovery,
) -> Option<EphemeralWorkInterruption> {
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

pub(super) async fn wait_for_background_tasks_to_stop(
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
