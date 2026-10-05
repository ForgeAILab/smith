use super::*;

#[tokio::test]
async fn a_durable_child_accepts_a_follow_up_after_parent_restart_with_prior_history() {
    let fixture = Fixture::new();
    let provider = scripted(2, "same specialist");
    let sessions = Arc::new(InMemorySessionStore::new());
    let checkpoints = Arc::new(InMemoryCheckpointStore::new());
    let parent_id = SessionId::new("durable-parent");

    let build = || {
        let mut request = request(&fixture, provider.clone());
        request.session_store = Some(sessions.clone());
        request.checkpoint_store = Some(checkpoints.clone());
        request
    };

    let first = factory::build_request(build())
        .await
        .expect("the first Smith runtime");
    let first_session = first
        .runtime()
        .start_session(StartSession::new().with_id(parent_id.clone()))
        .await
        .expect("the first parent session");
    let first_delegation = first.delegation().expect("a root delegation surface");
    let first_lifecycle = wire_delegation(&first_session, first_delegation)
        .await
        .expect("first delegation wiring");
    // Model a process boundary after wiring but before automatic admission;
    // the protected coordinator remains usable and persists the outcome.
    first_lifecycle.shutdown().await;
    let first_coordinator = first_delegation.coordinator().expect("first coordinator");
    let child = match first_coordinator
        .spawn(ChildSpec {
            task: UserInput::text("Inspect the parser and remember the important constraints."),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(3),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the durable child spawns")
    {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };
    first_coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the first child task completes");
    let before = first_coordinator
        .status(&child)
        .expect("first child status");
    assert_eq!(before.durability, ChildDurability::Durable);
    first_coordinator
        .flush()
        .await
        .expect("the child catalog flushes");
    first_session
        .shutdown()
        .await
        .expect("the first parent shuts down");
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    let second = factory::build_request(build())
        .await
        .expect("the resumed Smith runtime");
    let second_session = second
        .runtime()
        .start_session(StartSession::new().with_id(parent_id))
        .await
        .expect("the parent snapshot resumes");
    let second_delegation = second.delegation().expect("a resumed delegation surface");
    wire_delegation(&second_session, second_delegation)
        .await
        .expect("resumed delegation wiring");
    let second_coordinator = second_delegation
        .coordinator()
        .expect("resumed coordinator");
    let recovered = second_coordinator
        .status(&child)
        .expect("the same child is restored");
    assert_eq!(recovered.child, before.child);
    assert_eq!(recovered.session, before.session);
    assert_eq!(recovered.state, ChildState::Idle);
    assert_eq!(recovered.turns_used, 1);

    second_coordinator
        .follow_up(
            &child,
            UserInput::text("Now identify the highest-risk regression based on that review."),
        )
        .await
        .expect("the recovered child accepts a new turn");
    second_coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the follow-up completes");
    let after = second_coordinator.status(&child).expect("follow-up status");
    assert_eq!(after.session, before.session);
    assert_eq!(after.turns_used, 2);

    let follow_up_request = provider.requests()[1].clone();
    let wire = serde_json::to_string(&follow_up_request.messages).expect("provider messages");
    assert!(wire.contains("Inspect the parser"), "{wire}");
    assert!(wire.contains("same specialist"), "{wire}");
    assert!(wire.contains("highest-risk regression"), "{wire}");

    second_coordinator
        .flush()
        .await
        .expect("the resumed catalog flushes");
    second_session
        .shutdown()
        .await
        .expect("the resumed parent shuts down");
}

#[tokio::test]
async fn a_returned_child_question_survives_smith_restart_without_provider_work() {
    let fixture = Fixture::new();
    let text_script = |text: &str| {
        ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta { text: text.into() },
            usage_event(5, 2),
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])
    };
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            questionnaire_script(
                "ask-after-restart",
                "direction",
                "Choose the implementation direction",
                "public",
            ),
            text_script("parent surfaced the recovered question"),
            text_script("child continued with the answer"),
            text_script("parent received the continued child result"),
        ],
    ));
    let sessions = Arc::new(InMemorySessionStore::new());
    let checkpoints = Arc::new(InMemoryCheckpointStore::new());
    let parent_id = SessionId::new("question-parent");
    let build = || {
        let mut request = request(&fixture, provider.clone());
        request.session_store = Some(sessions.clone());
        request.checkpoint_store = Some(checkpoints.clone());
        request
    };

    let first = factory::build_request(build())
        .await
        .expect("the first Smith runtime");
    let first_session = first
        .runtime()
        .start_session(StartSession::new().with_id(parent_id.clone()))
        .await
        .expect("the first parent session");
    let first_delegation = first.delegation().expect("a root delegation surface");
    wire_delegation(&first_session, first_delegation)
        .await
        .expect("first delegation wiring");
    let first_coordinator = first_delegation.coordinator().expect("first coordinator");
    let (child, child_session) = match first_coordinator
        .spawn(ChildSpec {
            task: UserInput::text("Ask which implementation direction to use."),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(3),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the durable child spawns")
    {
        SpawnOutcome::Spawned { child, handle } => (child, handle.id().clone()),
        other => panic!("expected a spawned child, got {other:?}"),
    };
    assert!(matches!(
        first_coordinator
            .wait_task_outcome(&child)
            .await
            .expect("the child returns its question"),
        ChildTaskOutcome::NeedsInput { .. }
    ));
    first_coordinator.flush().await.expect("catalog flushes");
    first_session.shutdown().await.expect("first parent stops");
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    let second = factory::build_request(build())
        .await
        .expect("the resumed Smith runtime");
    let second_session = second
        .runtime()
        .start_session(StartSession::new().with_id(parent_id))
        .await
        .expect("the parent resumes");
    let second_delegation = second.delegation().expect("a resumed delegation surface");
    let _second_lifecycle = wire_delegation(&second_session, second_delegation)
        .await
        .expect("protected child question recovers");
    let second_coordinator = second_delegation
        .coordinator()
        .expect("resumed coordinator");
    assert_eq!(
        provider.requests().len(),
        1,
        "Smith startup recovered the question without a provider call"
    );
    assert!(matches!(
        second_coordinator
            .task_outcome(&child)
            .expect("known recovered child"),
        Some(ChildTaskOutcome::NeedsInput { .. })
    ));

    wait_for_provider_requests(&provider, 2).await;
    let root_wire = serde_json::to_string(&provider.requests()[1].messages).expect("messages");
    assert!(
        root_wire.contains("delegation.child-completion"),
        "{root_wire}"
    );
    assert!(
        root_wire.contains("needs protected input") && root_wire.contains("direction"),
        "{root_wire}"
    );
    assert!(
        !root_wire.contains("Choose the implementation direction"),
        "automatic admission carries bounded identities, not questionnaire text: {root_wire}"
    );

    second_coordinator
        .follow_up(&child, UserInput::text("Use direction one."))
        .await
        .expect("the answer starts a new turn on the same child");
    second_coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the child follow-up completes");
    let status = second_coordinator.status(&child).expect("child status");
    assert_eq!(status.session, child_session);
    assert_eq!(status.turns_used, 2);
    assert_eq!(
        status.last_result.as_deref(),
        Some("child continued with the answer")
    );
    wait_for_provider_requests(&provider, 4).await;
    let completion_wire = serde_json::to_string(&provider.requests()[3].messages)
        .expect("parent completion messages");
    assert!(
        completion_wire.contains("delegation.child-completion"),
        "{completion_wire}"
    );
    assert!(
        completion_wire.contains(child.as_str()),
        "{completion_wire}"
    );
    assert!(
        completion_wire.contains("child continued with the answer"),
        "{completion_wire}"
    );
    assert_eq!(provider.requests().len(), 4);
    second_session.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn an_interrupted_smith_child_resumes_exactly_once_and_never_on_startup() {
    let fixture = Fixture::new();
    let provider = Arc::new(CrashThenReplyProvider::new());
    let sessions = Arc::new(InMemorySessionStore::new());
    let crash_checkpoints = Arc::new(InMemoryCheckpointStore::new());
    let parent_id = SessionId::new("crashed-parent");
    let build = |checkpoint_store: Arc<InMemoryCheckpointStore>| {
        let mut request = request(&fixture, provider.clone());
        request.session_store = Some(sessions.clone());
        request.checkpoint_store = Some(checkpoint_store);
        request
    };

    let first = factory::build_request(build(crash_checkpoints.clone()))
        .await
        .expect("the first Smith runtime");
    let first_session = first
        .runtime()
        .start_session(StartSession::new().with_id(parent_id.clone()))
        .await
        .expect("the first parent");
    let first_delegation = first.delegation().expect("the first delegation surface");
    wire_delegation(&first_session, first_delegation)
        .await
        .expect("first delegation wiring");
    let first_coordinator = first_delegation.coordinator().expect("first coordinator");
    let (child, abandoned_handle) = match first_coordinator
        .spawn(ChildSpec {
            task: UserInput::text("Keep analyzing until the process interruption."),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(2),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the first child starts")
    {
        SpawnOutcome::Spawned { child, handle } => (child, handle),
        other => panic!("expected a spawned child, got {other:?}"),
    };
    provider.wait_for_calls(1).await;
    first_coordinator
        .flush()
        .await
        .expect("the running catalog and checkpoint flush");
    let before = first_coordinator
        .status(&child)
        .expect("running child status");
    assert_eq!(before.state, ChildState::Running);
    assert_eq!(before.turns_used, 1);

    // The first protected boundary is Accepted, before provider I/O. Model a
    // crash at exactly that safe point by retaining it in a fresh protected
    // store and aligning the parent-owned watermark. The later CallingModel
    // boundary is deliberately non-resumable because its provider outcome is
    // indeterminate and replay could double-spend.
    let accepted = crash_checkpoints
        .history(&before.session)
        .into_iter()
        .find(|checkpoint| {
            matches!(
                checkpoint.state,
                agent_runtime_core::checkpoint::TurnState::Accepted { .. }
            )
        })
        .expect("an accepted child checkpoint precedes provider I/O");
    let resume_checkpoints = Arc::new(InMemoryCheckpointStore::new());
    resume_checkpoints
        .seed(accepted.clone())
        .expect("the accepted crash boundary is retained");
    let mut parent_snapshot = sessions
        .load(&parent_id)
        .await
        .expect("the parent snapshot loads")
        .expect("the child catalog was persisted on the parent");
    let catalog_state = parent_snapshot
        .extension_state
        .get_mut(CHILD_CATALOG_NAMESPACE)
        .expect("the durable child catalog exists");
    let mut catalog: DurableChildCatalog =
        serde_json::from_value(catalog_state.value.clone()).expect("a valid child catalog");
    let record = catalog
        .children
        .first_mut()
        .expect("the child record exists");
    assert!(
        !record.checkpoint_resumable,
        "an indeterminate in-flight provider call was advertised as resumable"
    );
    record.checkpoint_watermark = Some(accepted.watermark);
    record.checkpoint_resumable = true;
    catalog_state.value = serde_json::to_value(catalog).expect("the fixture catalog serializes");
    sessions
        .save(&parent_snapshot)
        .await
        .expect("the accepted crash catalog is committed");

    // Starting a new owner over the persisted records models process loss:
    // no orderly child cancellation or terminal checkpoint occurred.
    let second = factory::build_request(build(resume_checkpoints))
        .await
        .expect("the replacement Smith runtime");
    let second_session = second
        .runtime()
        .start_session(StartSession::new().with_id(parent_id))
        .await
        .expect("the replacement parent");
    let second_delegation = second.delegation().expect("replacement delegation surface");
    wire_delegation(&second_session, second_delegation)
        .await
        .expect("replacement delegation wiring");
    let second_coordinator = second_delegation
        .coordinator()
        .expect("replacement coordinator");
    let recovered = second_coordinator
        .status(&child)
        .expect("interrupted child restored");
    assert!(matches!(
        recovered.state,
        ChildState::Interrupted { resumable: true }
    ));
    assert_eq!(recovered.session, before.session);
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "startup spent tokens"
    );

    let slot = Arc::new(OnceLock::new());
    slot.set(second_coordinator.clone())
        .expect("an empty agent slot");
    let tool = AgentTool::new(slot);
    let mut resumed_events = second_session.subscribe();
    let ctx = InvocationContext {
        session: second_session.id().clone(),
        turn: None,
        call_id: ToolCallId::new("resume-call"),
        request: RequestId::new("resume-request"),
        workspace: Arc::new(MemoryWorkspace::new("/repo")),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 100_000,
    };
    let resumed = invoke_agent(
        &tool,
        serde_json::json!({ "action": "resume", "child_id": child.as_str() }),
        &ctx,
    )
    .await
    .expect("the explicit resume operation returns");
    let resumed_wire = serde_json::to_string(&resumed.into_result_block(
        ToolCallId::new("resume-call"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("resume JSON");
    assert!(resumed_wire.contains("exact_checkpoint"), "{resumed_wire}");

    let outcome = match second_coordinator.wait_task_outcome(&child).await {
        Ok(outcome) => outcome,
        Err(error) => {
            let mut observed = Vec::new();
            while let Ok(Some(event)) =
                tokio::time::timeout(std::time::Duration::from_millis(10), resumed_events.next())
                    .await
            {
                observed.push(format!("{:?}", event.payload));
            }
            panic!(
                "the exact resumed turn completes: {error:?}; status={:?}; events={observed:?}",
                second_coordinator.status(&child)
            );
        }
    };
    assert!(matches!(
        outcome,
        ChildTaskOutcome::Completed { ref result, .. }
            if result.text == "resumed exact child"
    ));
    let after = second_coordinator.status(&child).expect("resumed status");
    assert_eq!(after.turns_used, 1, "resume consumed a new task slot");
    assert_eq!(after.session, before.session);
    assert_eq!(provider.requests().len(), 2);

    let duplicate = invoke_agent(
        &tool,
        serde_json::json!({ "action": "resume", "child_id": child.as_str() }),
        &ctx,
    )
    .await
    .expect("duplicate resume returns a structured tool error");
    let duplicate_wire = serde_json::to_string(&duplicate.into_result_block(
        ToolCallId::new("duplicate-resume"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("duplicate resume JSON");
    assert!(duplicate_wire.contains("no compatible interrupted checkpoint"));
    assert_eq!(
        provider.requests().len(),
        2,
        "duplicate resume called the provider"
    );

    second_coordinator
        .flush()
        .await
        .expect("replacement catalog flushes");
    second_session
        .shutdown()
        .await
        .expect("replacement parent shuts down");
    abandoned_handle.cancel(CancelReason::Shutdown);
    let _ = abandoned_handle.shutdown().await;
    let _ = first_session.shutdown().await;
}
