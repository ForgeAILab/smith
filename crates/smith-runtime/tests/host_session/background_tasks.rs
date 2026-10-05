use super::*;

/// A background task start marker with no terminal marker is the same
/// crash shape as an unresolved monitor or child: resume must report it
/// through `recovered_ephemeral_work` and must never spawn a process for it,
/// since the in-memory registry that could own such a process starts empty
/// in every new Smith process.
#[tokio::test]
async fn resume_marks_an_unresolved_background_task_interrupted_without_spawning_it() {
    let fixture = Fixture::new();
    let first = start(fixture.request(HostSurface::Headless))
        .await
        .expect("a first host");
    let session_id = first.session().id().clone();
    let paths = first.paths().expect("persistent paths").clone();
    first.shutdown().await.expect("persist the base session");

    let journal_path = paths.journal(&session_id).expect("journal path");
    let recovery = read_journal(&journal_path)
        .await
        .expect("the first journal reads");
    let mut records = recovery
        .records
        .into_iter()
        .filter(|line| {
            !matches!(
                line.record,
                JournalRecord::Event {
                    event: EventEnvelope {
                        payload: RuntimeEvent::SessionShutdown,
                        ..
                    }
                }
            )
        })
        .collect::<Vec<_>>();

    let task = "task:before-crash".to_owned();
    records.push(JournalLine::new(JournalRecord::TaskStarted {
        task: task.clone(),
    }));

    let mut bytes = Vec::new();
    for line in &records {
        serde_json::to_writer(&mut bytes, line).expect("a serializable journal line");
        bytes.push(b'\n');
    }
    tokio::fs::write(&journal_path, bytes)
        .await
        .expect("the crash fixture is installed");

    let resumed = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session_id.clone()),
    )
    .await
    .expect("the interrupted session resumes");
    let interruption = resumed
        .recovered_ephemeral_work()
        .expect("an explicit interruption marker");
    assert_eq!(interruption.tasks.as_slice(), std::slice::from_ref(&task));
    assert!(
        resumed
            .background_tasks()
            .running_tasks(&session_id)
            .is_empty(),
        "resume must never spawn a process for a recovered background task"
    );

    resumed
        .shutdown()
        .await
        .expect("the resumed host shuts down");

    let recovery = read_journal(&journal_path)
        .await
        .expect("the reconciled journal reads");
    let markers = recovery
        .records
        .iter()
        .filter_map(|line| match &line.record {
            JournalRecord::EphemeralWorkInterrupted { interruption } => Some(interruption),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].tasks, [task]);
}

/// The host must never let a background task outlive its process: shutdown
/// signals every running task and waits, bounded, for its worker to kill the
/// owned process group and record the terminal journal marker — it must not
/// wait out the task's own duration.
#[tokio::test]
async fn shutdown_kills_a_running_background_task_within_the_grace_period() {
    let fixture = Fixture::new();
    let host = start(fixture.request(HostSurface::Headless))
        .await
        .expect("a hosted session");
    let session_id = host.session().id().clone();

    host.background_tasks()
        .spawn_background_task(
            &session_id,
            "sleep 30".to_owned(),
            fixture.project.path().to_path_buf(),
            None,
        )
        .await
        .expect("a background task spawns");
    assert_eq!(
        host.background_tasks().running_tasks(&session_id).len(),
        1,
        "the task is registered as running before shutdown"
    );

    let started = std::time::Instant::now();
    host.shutdown().await.expect("shutdown completes");
    let elapsed = started.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "shutdown must not wait out the task's own 30s sleep: {elapsed:?}"
    );
    assert!(
        host.background_tasks()
            .running_tasks(&session_id)
            .is_empty(),
        "shutdown must kill the task's owned process group before returning"
    );

    let paths = host.paths().expect("persistent paths");
    let journal_path = paths.journal(&session_id).expect("journal path");
    let recovery = read_journal(&journal_path)
        .await
        .expect("the journal reads");
    let started_markers = recovery
        .records
        .iter()
        .filter(|line| matches!(&line.record, JournalRecord::TaskStarted { .. }))
        .count();
    let exited_markers = recovery
        .records
        .iter()
        .filter(|line| matches!(&line.record, JournalRecord::TaskExited { .. }))
        .count();
    assert_eq!(started_markers, 1);
    assert_eq!(
        exited_markers, 1,
        "the worker's terminal marker must land before shutdown closes the journal"
    );
}

/// A background task's terminal notification is model-facing content, so it
/// must reach the parent the same way a completed child's result does: as
/// must-deliver injected content picked up at the next safe boundary, never
/// mutating a response already in flight.
#[tokio::test]
async fn a_background_task_terminal_notification_reaches_the_parent_model_at_a_safe_boundary() {
    let fixture = Fixture::new();
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta {
                text: "done".to_owned(),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])],
    ));
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone());
    let host = start(request).await.expect("a hosted session");
    let session_id = host.session().id().clone();

    host.background_tasks()
        .spawn_background_task(
            &session_id,
            "true".to_owned(),
            fixture.project.path().to_path_buf(),
            None,
        )
        .await
        .expect("a background task spawns");

    // The worker task runs concurrently; wait for it to leave the running
    // set, then give it one more beat to finish injecting the notification,
    // the same margin `HostSession::shutdown` gives it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !host
        .background_tasks()
        .running_tasks(&session_id)
        .is_empty()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the trivial background task did not reach a terminal state in time"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    host.session()
        .run(UserInput::text("anything"))
        .await
        .expect("the parent turn runs");

    let deliveries = provider.requests()[0]
        .messages
        .iter()
        .filter_map(|message| {
            let text = message.joined_text();
            text.contains(r#""type":"background_task_terminal""#)
                .then(|| serde_json::from_str::<serde_json::Value>(&text).expect("typed delivery"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        deliveries.len(),
        1,
        "the terminal notification is injected exactly once"
    );
    assert_eq!(deliveries[0]["status"], "exited");
    assert_eq!(deliveries[0]["exit_code"], 0);

    host.shutdown().await.expect("clean shutdown");
}
