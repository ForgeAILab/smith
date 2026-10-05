use super::*;

#[tokio::test]
async fn durable_child_follow_up_survives_a_full_smith_host_restart() {
    let fixture = Fixture::new();
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "remembered parser constraints".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "parent received the completed review".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "follow-up regression risk".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let mut first_request = fixture.request(HostSurface::Headless);
    first_request.runtime.provider = Some(provider.clone());
    let first = start(first_request).await.expect("the first Smith host");
    let parent = first.session().id().clone();
    let mut parent_events = first.session().subscribe();
    let first_coordinator = first
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .expect("the first coordinator");
    let child = match first_coordinator
        .spawn(ChildSpec {
            task: UserInput::text("Inspect the parser and retain its important constraints."),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(3),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the durable child starts")
    {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };
    first_coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the first task completes");
    // The admission worker delivers the result in a parent turn of its own.
    // Shutting down after that turn is admitted but before its provider call
    // leaves the result consumed and unanswered, so wait for the turn.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(event) = parent_events.next().await {
            if matches!(event.payload, RuntimeEvent::TurnCompleted { .. }) {
                return;
            }
        }
        panic!("the parent event stream ended before the delivery turn completed");
    })
    .await
    .expect("the automatic delivery turn completes");
    let before = first_coordinator
        .status(&child)
        .expect("first child status");
    assert_eq!(before.durability, ChildDurability::Durable);
    assert_eq!(before.state, ChildState::Idle);
    first.shutdown().await.expect("the first host shuts down");

    let mut resume_request = fixture
        .request(HostSurface::Headless)
        .resume(parent.clone());
    resume_request.runtime.provider = Some(provider.clone());
    let resumed = start(resume_request)
        .await
        .expect("the same Smith host session resumes");
    assert!(
        resumed.recovered_ephemeral_work().is_none(),
        "a durable catalog child was mislabeled legacy ephemeral"
    );
    let resumed_coordinator = resumed
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .expect("the resumed coordinator");
    let recovered = resumed_coordinator
        .status(&child)
        .expect("the same child identity is retained");
    assert_eq!(recovered.parent, parent);
    assert_eq!(recovered.session, before.session);
    assert_eq!(recovered.state, ChildState::Idle);

    resumed_coordinator
        .follow_up(
            &child,
            UserInput::text("Identify the highest-risk regression using that retained review."),
        )
        .await
        .expect("the recovered child accepts a follow-up");
    resumed_coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the follow-up completes");
    let after = resumed_coordinator
        .status(&child)
        .expect("follow-up status");
    assert_eq!(after.session, before.session);
    assert_eq!(after.turns_used, 2);

    let requests = provider.requests();
    assert_eq!(
        requests.len(),
        3,
        "the completed first child task is delivered through one automatic parent continuation before the child follow-up"
    );
    let follow_up_wire =
        serde_json::to_string(&requests[2].messages).expect("follow-up provider request");
    assert!(
        follow_up_wire.contains("Inspect the parser"),
        "{follow_up_wire}"
    );
    assert!(
        follow_up_wire.contains("remembered parser constraints"),
        "{follow_up_wire}"
    );
    assert!(
        follow_up_wire.contains("highest-risk regression"),
        "{follow_up_wire}"
    );
    resumed
        .shutdown()
        .await
        .expect("the resumed host shuts down");
}

#[tokio::test]
async fn resume_marks_unresolved_ephemeral_work_interrupted_without_restarting_it() {
    let fixture = Fixture::new();
    let first = start(fixture.request(HostSurface::Headless))
        .await
        .expect("a first host");
    let session_id = first.session().id().clone();
    let paths = first.paths().expect("persistent paths").clone();
    first.shutdown().await.expect("persist the base session");

    // Replace the orderly terminal tail with the state a crashed process
    // would have left: a child was spawned and no task-resolution event was
    // committed. The saved session/checkpoint remains the authoritative
    // resumable root state.
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
    let mut next_seq = records
        .iter()
        .filter_map(|line| match &line.record {
            JournalRecord::Event { event } => Some(event.seq),
            _ => None,
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let spawned = |child: &ChildId| RuntimeEvent::ChildSpawned {
        child: child.clone(),
        workspace: WorkspacePolicy::ReadOnlyView,
        max_turns: 2,
        max_tokens: None,
        deadline_ms: None,
    };
    let running = ChildId::new("child-running-before-crash");
    let completed = ChildId::new("child-completed-before-crash");
    let needs_input = ChildId::new("child-needs-input-before-crash");
    let stopped = ChildId::new("child-stopped-before-crash");
    let failed = ChildId::new("child-failed-before-crash");
    {
        let mut append = |payload| {
            let seq = next_seq;
            next_seq = next_seq.saturating_add(1);
            records.push(JournalLine::new(JournalRecord::Event {
                event: EventEnvelope::new(
                    seq,
                    EventId::new(format!("evt-before-crash-{seq}")),
                    session_id.clone(),
                    None,
                    Timestamp(50),
                    payload,
                ),
            }));
        };
        append(spawned(&running));
        append(spawned(&completed));
        append(RuntimeEvent::ChildCompleted {
            child: completed.clone(),
            result: "available for follow-up".to_owned(),
        });
        append(spawned(&needs_input));
        append(RuntimeEvent::ChildNeedsInput {
            child: needs_input.clone(),
            child_session: SessionId::new("child-session-before-crash"),
            turn: TurnId::new("child-turn-before-crash"),
            call: ToolCallId::new("child-call-before-crash"),
            request: InteractionRequestId::new("child-request-before-crash"),
            question_ids: vec![QuestionId::new("child-question-before-crash")],
            sensitivity: InteractionSensitivity::Sensitive,
        });
        append(spawned(&stopped));
        append(RuntimeEvent::ChildStopped {
            child: stopped,
            reason: CancelReason::Shutdown,
        });
        append(spawned(&failed));
        append(RuntimeEvent::ChildFailed {
            child: failed,
            error: agent_runtime_core::error::RuntimeError::internal("child failed"),
        });
    }
    let running_monitor = "monitor:build-before-crash".to_owned();
    let stopped_monitor = "monitor:lint-before-crash".to_owned();
    records.push(JournalLine::new(JournalRecord::MonitorStarted {
        monitor: running_monitor.clone(),
    }));
    records.push(JournalLine::new(JournalRecord::MonitorStarted {
        monitor: stopped_monitor.clone(),
    }));
    records.push(JournalLine::new(JournalRecord::MonitorStopped {
        monitor: stopped_monitor,
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
    assert_eq!(
        interruption.children,
        [completed.clone(), needs_input.clone(), running.clone()]
    );
    assert_eq!(
        interruption.monitors.as_slice(),
        std::slice::from_ref(&running_monitor)
    );
    assert!(
        resumed
            .runtime()
            .delegation()
            .and_then(|delegation| delegation.coordinator())
            .expect("a fresh coordinator")
            .list()
            .is_empty(),
        "resume recreated an ephemeral child"
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
    assert_eq!(markers[0].children, [completed, needs_input, running]);
    assert_eq!(markers[0].monitors, [running_monitor]);

    // The marker participates in the next scan, so the same prior work is not
    // reported or appended twice.
    let resumed_again = start(fixture.request(HostSurface::Headless).resume(session_id))
        .await
        .expect("a second resume");
    assert!(resumed_again.recovered_ephemeral_work().is_none());
    resumed_again
        .shutdown()
        .await
        .expect("the second resume shuts down");
}
