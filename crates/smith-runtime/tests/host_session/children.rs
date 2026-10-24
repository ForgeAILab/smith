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
    let limit_stopped = ChildId::new("child-limit-stopped-before-crash");
    let needs_input = ChildId::new("child-needs-input-before-crash");
    let stopped = ChildId::new("child-stopped-before-crash");
    let failed = ChildId::new("child-failed-before-crash");
    {
        let mut append = |payload: RuntimeEvent| {
            let seq = next_seq;
            next_seq = next_seq.saturating_add(1);
            // Every event an older build could write keeps that build's
            // event schema (16), so the journal of a session started before
            // the upgrade must still resume. Only the limit event is new.
            let older_schema = !matches!(payload, RuntimeEvent::ChildLimitReached { .. });
            let mut event = EventEnvelope::new(
                seq,
                EventId::new(format!("evt-before-crash-{seq}")),
                session_id.clone(),
                None,
                Timestamp(50),
                payload,
            );
            if older_schema {
                event.schema_version = 16;
            }
            records.push(JournalLine::new(JournalRecord::Event { event }));
        };
        append(spawned(&running));
        append(spawned(&completed));
        append(RuntimeEvent::ChildCompleted {
            child: completed.clone(),
            result: "available for follow-up".to_owned(),
        });
        // A task that stopped at a limit leaves its child available for a
        // follow-up, exactly like a completed one.
        append(spawned(&limit_stopped));
        append(RuntimeEvent::ChildLimitReached {
            child: limit_stopped.clone(),
            limit: agent_runtime_core::event::LimitKind::Output,
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
        [
            completed.clone(),
            limit_stopped.clone(),
            needs_input.clone(),
            running.clone()
        ]
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
    assert_eq!(
        markers[0].children,
        [completed, limit_stopped, needs_input, running]
    );
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

const ROUTED_CHILD_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"
use = ["main"]

[profiles.audit]
provider = "local"
model = "example-model"
posture = "review"
use = ["child"]
instructions = "Audit the requested scope."

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;

fn audit_profile(fixture: &Fixture) -> ResolvedConfig {
    resolve(
        &ResolveRequest::new(fixture.project.path())
            .with_home_dir(fixture.home.path())
            .with_cli(Overrides {
                profile: Some("audit".to_owned()),
                ..Overrides::default()
            })
            .with_profile_use(smith_config::model::ProfileUse::Child),
    )
    .expect("the child profile")
    .config
}

fn routed_request(fixture: &Fixture, provider: &Arc<FakeProvider>) -> HostSessionRequest {
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone());
    request
        .runtime
        .child_profiles
        .push(smith_runtime::factory::ChildProfileRequest {
            vision_profile: None,
            config: audit_profile(fixture),
            catalog_sources: Vec::new(),
        });
    request
}

/// Runs one routed child to completion and returns the parent and child ids.
async fn routed_child_session(
    fixture: &Fixture,
    provider: &Arc<FakeProvider>,
) -> (SessionId, agent_runtime_core::ids::ChildId) {
    let first = start(routed_request(fixture, provider))
        .await
        .expect("the first Smith host");
    let parent = first.session().id().clone();
    let mut parent_events = first.session().subscribe();
    let coordinator = first
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .expect("the first coordinator");
    let audit = audit_profile(fixture);
    let route = smith_runtime::delegation::profile_route_key(
        &audit.agent.profile.name,
        &audit.agent.profile.revision,
    );
    let child = match coordinator
        .spawn(ChildSpec {
            task: UserInput::text("Audit the parser."),
            model: ChildModelSelection::Explicit {
                provider: Some(route),
                model: agent_runtime_core::provider::ModelId::new("example-model"),
            },
            limits: ChildLimits::turns(1),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the routed child starts")
    {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };
    coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the routed child completes");
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
    first.shutdown().await.expect("the first host shuts down");
    (parent, child)
}

fn delivery_provider() -> Arc<FakeProvider> {
    Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta {
                text: "parent received the audit".to_owned(),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])],
    ))
}

#[tokio::test]
async fn a_parent_resumes_after_its_child_profile_changed_or_was_removed() {
    for removed in [false, true] {
        let fixture = Fixture::with_config(ROUTED_CHILD_CONFIG);
        let provider = delivery_provider();
        let (parent, child) = routed_child_session(&fixture, &provider).await;

        let mut resume = if removed {
            let config = ROUTED_CHILD_CONFIG.replace("use = [\"child\"]", "use = [\"main\"]");
            std::fs::write(fixture.project.path().join(".smith/config.toml"), config)
                .expect("the profile is no longer a child profile");
            let mut request = fixture.request(HostSurface::Headless);
            request.runtime.provider = Some(provider.clone());
            request
        } else {
            let config = ROUTED_CHILD_CONFIG.replace(
                "Audit the requested scope.",
                "Audit the requested scope and cite evidence.",
            );
            std::fs::write(fixture.project.path().join(".smith/config.toml"), config)
                .expect("the child profile changes");
            routed_request(&fixture, &provider)
        };
        resume = resume.resume(parent.clone());
        let resumed = start(resume)
            .await
            .unwrap_or_else(|error| panic!("the parent resumes (removed={removed}): {error}"));
        let status = resumed
            .runtime()
            .delegation()
            .and_then(|delegation| delegation.coordinator())
            .expect("the resumed coordinator")
            .status(&child)
            .expect("the child is still listed");
        assert_eq!(status.state, ChildState::Idle);
        assert!(
            status.incompatibility.is_some(),
            "a child recorded under another policy is reported changed (removed={removed})"
        );
        resumed
            .shutdown()
            .await
            .expect("the resumed host shuts down");
    }
}
