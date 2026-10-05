use super::*;

/// A parent renders its child's tool activity from the same two things it
/// renders its own from: an event that names the call, and canonical history
/// the call id resolves against. The event stays value-free; the arguments and
/// the result come from the child's history, redacted by the parent host.
#[tokio::test]
async fn a_child_tool_call_reaches_the_parent_with_the_identity_its_arguments_resolve_from() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.project.path().join("retry.rs"),
        "fn retry() { /* backoff */ }\n",
    )
    .expect("a readable child fixture");
    let mut read = tool_call_fragments(
        0,
        "child-read-1",
        "read",
        &serde_json::json!({ "path": "retry.rs" }).to_string(),
    );
    read.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(read),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "the retry helper backs off".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let mut runtime_request = request(&fixture, provider);
    runtime_request.workspace = Some(Arc::new(
        ProjectWorkspace::new(fixture.project.path()).expect("a project workspace"),
    ));
    let smith = factory::build_request(runtime_request)
        .await
        .expect("a runtime that can delegate");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a parent session");
    let delegation = smith.delegation().expect("a delegation surface");
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wires");
    let coordinator = delegation.coordinator().expect("a coordinator");
    let mut parent_events = session.subscribe();

    let spawned = coordinator
        .spawn(ChildSpec {
            task: UserInput::text("read the retry helper"),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(1),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the child spawns");
    let child = match spawned {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };
    // The host learns about a child from the parent stream and subscribes by
    // id — it never sees the handle the spawning tool call received.
    let mut child_events = coordinator
        .child_events(&child)
        .expect("a live child's own stream is reachable by id");
    coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the child completes");

    let mut parent_phases = Vec::new();
    while let Ok(Some(envelope)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), parent_events.next()).await
    {
        if let RuntimeEvent::ChildProgress { phase, .. } = envelope.payload {
            parent_phases.push(phase);
        }
    }
    assert!(
        parent_phases
            .iter()
            .all(|phase| matches!(phase, ChildPhase::TurnStarted | ChildPhase::TurnFinished)),
        "the parent stream carries delegation's boundaries, not a summary of the child's \
         work: {parent_phases:?}"
    );

    let mut requested = None;
    let mut completed = None;
    while let Ok(Some(envelope)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), child_events.next()).await
    {
        match envelope.payload {
            RuntimeEvent::ToolCallRequested {
                call,
                name,
                argument_keys,
                arguments,
                ..
            } => requested = Some((call, name, argument_keys, arguments)),
            RuntimeEvent::ToolCallCompleted {
                call,
                name,
                is_error,
            } => completed = Some((call, name, is_error)),
            _ => {}
        }
    }
    let (call, name, argument_keys, arguments) =
        requested.expect("the child's own stream reports its tool call");
    assert_eq!(name, "read");
    assert_eq!(argument_keys, vec!["path".to_owned()]);
    assert_eq!(
        arguments, None,
        "a child's events protect argument values exactly as the parent's own do"
    );
    assert_eq!(completed, Some((call.clone(), "read".to_owned(), false)));

    // The event carried no argument values and no result text. Both come from
    // the child's canonical history, addressed by the id the event did carry —
    // the lookup `HostSession::child_tool_call_display` performs.
    let found = coordinator
        .with_child_history(&child, |history| {
            let arguments = history.iter().rev().find_map(|message| {
                message.content.iter().rev().find_map(|part| match part {
                    agent_runtime_core::content::ContentPart::ToolCall(candidate)
                        if candidate.id == call =>
                    {
                        Some(candidate.arguments.clone())
                    }
                    _ => None,
                })
            });
            let result = history.iter().rev().find_map(|message| {
                message.content.iter().rev().find_map(|part| match part {
                    agent_runtime_core::content::ContentPart::ToolResult(candidate)
                        if candidate.call_id == call =>
                    {
                        Some(
                            candidate
                                .content
                                .iter()
                                .filter_map(|part| part.as_text())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        )
                    }
                    _ => None,
                })
            });
            (arguments, result)
        })
        .expect("a live child's history is reachable from its parent");
    assert_eq!(
        found.0,
        Some(serde_json::json!({ "path": "retry.rs" })),
        "the arguments the event withheld are resolvable from the child's history"
    );
    assert!(
        found.1.is_some_and(|text| text.contains("backoff")),
        "so is the result the child's tool returned"
    );

    session.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn concurrent_child_needs_input_is_lossless_ordered_and_never_opens_root_ui() {
    const PRIVATE_PROMPT: &str = "PRIVATE CHILD PROMPT MUST NOT ENTER PARENT";
    let fixture = Fixture::new();
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            questionnaire_script(
                "ask-public",
                "public-direction",
                "Choose the public direction",
                "public",
            ),
            questionnaire_script(
                "ask-sensitive",
                "private-direction",
                PRIVATE_PROMPT,
                "sensitive",
            ),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "parent handled both child requests".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let mut runtime_request = request(&fixture, provider.clone());
    runtime_request.event_buffer = 1;
    let smith = factory::build_request(runtime_request)
        .await
        .expect("a runtime with a one-event presentation buffer");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a delegation surface");
    let _lifecycle = wire_delegation(&session, delegation)
        .await
        .expect("delegation wires");
    let coordinator = delegation.coordinator().expect("a coordinator");

    let spawn = |task: &str| ChildSpec {
        task: UserInput::text(task),
        model: ChildModelSelection::Inherit,
        limits: ChildLimits::turns(1),
        tools: ToolViewScope::ReadOnly,
        workspace: WorkspacePolicy::ReadOnlyView,
    };
    let first = coordinator
        .spawn(spawn(
            "Ask the user to choose a public implementation direction",
        ))
        .await
        .expect("first child spawns");
    let second = coordinator
        .spawn(spawn(
            "Ask the user for the sensitive missing implementation detail",
        ))
        .await
        .expect("second child spawns");
    let child_ids = [first, second].map(|outcome| match outcome {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a running child, got {other:?}"),
    });

    let (first_outcome, second_outcome) =
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(
                coordinator.wait_task_outcome(&child_ids[0]),
                coordinator.wait_task_outcome(&child_ids[1])
            )
        })
        .await
        .expect("children do not wait on a root UI broker");
    for outcome in [first_outcome, second_outcome] {
        assert!(matches!(
            outcome.expect("a typed child outcome"),
            ChildTaskOutcome::NeedsInput { .. }
        ));
    }
    assert!(
        provider.requests().len() >= 2,
        "each questionnaire child must finish its own provider turn"
    );

    wait_for_provider_requests(&provider, 3).await;
    let parent_request = provider.requests().last().cloned().expect("parent request");
    let parent_wire =
        serde_json::to_string(&parent_request.messages).expect("serializable parent messages");
    assert!(parent_wire.contains("delegation.child-completion"));
    assert!(parent_wire.contains("public-direction"));
    assert!(parent_wire.contains("private-direction"));
    let child_1 = parent_wire.find("child-1").expect("first child identity");
    let child_2 = parent_wire.find("child-2").expect("second child identity");
    assert!(
        child_1 < child_2,
        "keyed peers are canonicalized independently of completion order: {parent_wire}"
    );
    assert!(
        !parent_wire.contains("Choose the public direction"),
        "automatic admission carries bounded identities, not questionnaire text"
    );
    assert!(
        !parent_wire.contains(PRIVATE_PROMPT),
        "a sensitive child prompt entered ordinary parent context"
    );

    for child in &child_ids {
        assert!(
            matches!(
                coordinator
                    .task_outcome(child)
                    .expect("known child")
                    .expect("retained protected outcome"),
                ChildTaskOutcome::NeedsInput { .. }
            ),
            "automatic delivery consumed the exact protected request"
        );
    }

    session.shutdown().await.expect("a clean shutdown");
}
