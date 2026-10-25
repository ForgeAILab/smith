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
        tokio::spawn(async move { first_registry.stop_task(&first_session, &first_task).await });
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

#[cfg(unix)]
#[tokio::test]
async fn a_task_whose_shell_exits_first_stays_stoppable() {
    let registry = BackgroundTaskRegistry::new();
    let session = unique_session();
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("orphan.txt");

    // `sh` exits at once; the `&` subshell keeps the spool open until it
    // finishes, and `task_stop` must still reach it.
    let command = format!(
        "( sleep 2; echo orphaned > {} ) & echo started",
        marker.display()
    );
    let (task_id, _) = registry
        .spawn_background_task(&session, command, std::env::temp_dir(), None)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        registry
            .running_tasks(&session)
            .iter()
            .any(|task| task.task_id == task_id),
        "the task must stay running while its spool is open"
    );

    let status = registry.stop_task(&session, &task_id).await.unwrap();
    assert_eq!(status, TaskStatus::Stopped);

    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(!marker.exists(), "the stopped task's process survived");
}

/// Runs one `shell` call through `prepare` and `invoke`, as the runtime does.
async fn run_shell(
    tool: &smith_tools::ShellTool,
    session: &SessionId,
    arguments: serde_json::Value,
) -> agent_runtime_core::tool::ToolOutcome {
    use agent_runtime_core::cancel::Cancellation;
    use agent_runtime_core::clock::{Deadline, SystemClock};
    use agent_runtime_core::ids::{RequestId, ToolCallId};
    use agent_runtime_core::tool::{InvocationContext, PreparationContext, Tool};

    let dir = tempfile::tempdir().unwrap();
    let workspace = Arc::new(smith_host::workspace::ProjectWorkspace::new(dir.path()).unwrap());
    let invocation = InvocationContext {
        session: session.clone(),
        turn: None,
        call_id: ToolCallId::new("call-1"),
        request: RequestId::new("request-1"),
        workspace: workspace.clone(),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 32_000,
    };
    let preparation = PreparationContext {
        session: session.clone(),
        turn: None,
        call_id: invocation.call_id.clone(),
        request: invocation.request.clone(),
        workspace,
        clock: invocation.clock.clone(),
        cancel: invocation.cancel.clone(),
        deadline: invocation.deadline,
    };
    let prepared = tool.prepare(arguments, &preparation).await.unwrap();
    tool.invoke(prepared, &invocation).await.unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn a_foreground_call_hands_a_process_holding_its_output_to_the_registry() {
    let services = BackgroundServices::new();
    let session = unique_session();

    // The shell is reaped before the registry adopts what it left
    // running, so the worker starts from an exited child.
    let tool = smith_tools::ShellTool::new(services.host());
    let outcome = run_shell(
        &tool,
        &session,
        serde_json::json!({"command": "sleep 10 & echo started"}),
    )
    .await;
    assert!(!outcome.is_error);
    assert_eq!(outcome.value["exit_code"], 0);
    let task_id = outcome.value["task_id"]
        .as_str()
        .expect("the leftover process became a task")
        .to_owned();

    let status = services
        .registry()
        .stop_task(&session, &task_id)
        .await
        .unwrap();
    assert_eq!(status, TaskStatus::Stopped);
    let result = services
        .registry()
        .get_task_output(&session, &task_id, 0, 65536)
        .await
        .unwrap();
    assert!(result.output.contains("started"), "{}", result.output);
}

#[cfg(unix)]
#[tokio::test]
async fn a_call_moved_at_the_threshold_is_still_killed_at_its_timeout() {
    let services = BackgroundServices::new();
    let session = unique_session();
    let tool = smith_tools::ShellTool::new(services.host())
        .with_auto_background_after(Duration::from_millis(300));

    let started = std::time::Instant::now();
    let outcome = run_shell(
        &tool,
        &session,
        serde_json::json!({"command": "echo started; sleep 30", "timeout_ms": 1500}),
    )
    .await;
    assert!(!outcome.is_error);
    assert_eq!(outcome.value["backgrounded_automatically"], true);
    let task_id = outcome.value["task_id"]
        .as_str()
        .expect("the call moved to the background")
        .to_owned();
    let deadline = services
        .registry()
        .running_tasks(&session)
        .into_iter()
        .find(|task| task.task_id == task_id)
        .and_then(|task| task.timeout_ms)
        .expect("the task keeps a deadline");
    assert!((1..=1200).contains(&deadline), "{deadline}");

    let status = wait_for_terminal(services.registry(), &session, &task_id).await;
    assert_eq!(status, TaskStatus::DeadlineKill);
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    let result = services
        .registry()
        .get_task_output(&session, &task_id, 0, 65536)
        .await
        .unwrap();
    assert!(result.output.contains("started"), "{}", result.output);
}

#[cfg(unix)]
#[tokio::test]
async fn a_deadline_reaches_a_task_whose_shell_exited_first() {
    let registry = BackgroundTaskRegistry::new();
    let session = unique_session();
    let (task_id, _) = registry
        .spawn_background_task(
            &session,
            "sleep 30 & echo started".into(),
            std::env::temp_dir(),
            Some(300),
        )
        .await
        .unwrap();

    let status = wait_for_terminal(&registry, &session, &task_id).await;
    assert_eq!(status, TaskStatus::DeadlineKill);
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
