use super::*;

/// The model-facing `agent` tool drives the same coordinator: spawn, wait,
/// and list answer with structured JSON, and stop resolves a terminal state.
#[tokio::test]
async fn the_agent_tool_spawns_waits_and_lists() {
    let fixture = Fixture::new();
    let provider = scripted(1, "done");
    let smith = factory::build_request(request(&fixture, provider))
        .await
        .expect("a runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wires once");

    let slot = Arc::new(OnceLock::new());
    slot.set(delegation.coordinator().expect("a coordinator").clone())
        .expect("an empty slot");
    let tool = AgentTool::new(slot);
    let ctx = InvocationContext {
        session: session.id().clone(),
        turn: None,
        call_id: ToolCallId::new("call-1"),
        request: RequestId::new("req-1"),
        workspace: Arc::new(MemoryWorkspace::new("/repo")),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 100_000,
    };

    let spawned = invoke_agent(
        &tool,
        serde_json::json!({ "action": "spawn", "task": "do a thing" }),
        &ctx,
    )
    .await
    .expect("a spawn outcome");
    let spawned = serde_json::to_string(&spawned.into_result_block(
        ToolCallId::new("call-1"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("json");
    assert!(spawned.contains("child-1"), "{spawned}");

    let waited = invoke_agent(
        &tool,
        serde_json::json!({ "action": "wait", "child_id": "child-1" }),
        &ctx,
    )
    .await
    .expect("a wait outcome");
    let waited = serde_json::to_string(&waited.into_result_block(
        ToolCallId::new("call-2"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("json");
    assert!(waited.contains("idle"), "{waited}");
    assert!(waited.contains("done"), "{waited}");

    let listed = invoke_agent(&tool, serde_json::json!({ "action": "list" }), &ctx)
        .await
        .expect("a list outcome");
    let listed = serde_json::to_string(&listed.into_result_block(
        ToolCallId::new("call-3"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("json");
    assert!(listed.contains("child-1"), "{listed}");

    let stopped = invoke_agent(
        &tool,
        serde_json::json!({ "action": "stop", "child_id": "child-1" }),
        &ctx,
    )
    .await
    .expect("a stop outcome");
    let stopped = serde_json::to_string(&stopped.into_result_block(
        ToolCallId::new("call-4"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("json");
    assert!(stopped.contains("stopped"), "{stopped}");

    session.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn the_agent_tool_wait_is_bounded_without_stopping_the_child() {
    let fixture = Fixture::new();
    let provider = Arc::new(CrashThenReplyProvider::new());
    let smith = factory::build_request(request(&fixture, provider.clone()))
        .await
        .expect("a runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wires once");

    let slot = Arc::new(OnceLock::new());
    slot.set(delegation.coordinator().expect("a coordinator").clone())
        .expect("an empty slot");
    let tool = AgentTool::new(slot);
    let ctx = InvocationContext {
        session: session.id().clone(),
        turn: None,
        call_id: ToolCallId::new("call-1"),
        request: RequestId::new("req-1"),
        workspace: Arc::new(MemoryWorkspace::new("/repo")),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 100_000,
    };

    invoke_agent(
        &tool,
        serde_json::json!({ "action": "spawn", "task": "keep running" }),
        &ctx,
    )
    .await
    .expect("a spawn outcome");
    provider.wait_for_calls(1).await;

    let waited = invoke_agent(
        &tool,
        serde_json::json!({
            "action": "wait",
            "child_id": "child-1",
            "timeout_ms": 10
        }),
        &ctx,
    )
    .await
    .expect("a bounded wait outcome");
    let waited = serde_json::to_string(&waited.into_result_block(
        ToolCallId::new("call-2"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("json");
    assert!(waited.contains(r#"\"state\":\"running\""#), "{waited}");
    assert!(waited.contains(r#"\"timed_out\":true"#), "{waited}");

    let children = delegation.coordinator().expect("a coordinator").list();
    assert!(matches!(children[0].state, ChildState::Running));

    invoke_agent(
        &tool,
        serde_json::json!({ "action": "stop", "child_id": "child-1" }),
        &ctx,
    )
    .await
    .expect("a stop outcome");
    session.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn the_default_foreground_wait_releases_the_parent_without_stopping_the_child() {
    let fixture = Fixture::new();
    let provider = Arc::new(CrashThenReplyProvider::new());
    let smith = factory::build_request(request(&fixture, provider.clone()))
        .await
        .expect("a runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wires once");

    let slot = Arc::new(OnceLock::new());
    slot.set(delegation.coordinator().expect("a coordinator").clone())
        .expect("an empty slot");
    // Use a short policy in the test so the five-minute default behavior can be
    // exercised without making the test sleep for five minutes. The production
    // resolved default is five minutes; the child-lifetime assertion is the
    // same at either duration.
    let tool = AgentTool::new(slot).with_wait_policy(
        DelegationWaitPolicy::new(20, 30).expect("a short test foreground policy"),
    );
    let ctx = InvocationContext {
        session: session.id().clone(),
        turn: None,
        call_id: ToolCallId::new("call-1"),
        request: RequestId::new("req-1"),
        workspace: Arc::new(MemoryWorkspace::new("/repo")),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 100_000,
    };

    invoke_agent(
        &tool,
        serde_json::json!({ "action": "spawn", "task": "keep running" }),
        &ctx,
    )
    .await
    .expect("a spawn outcome");
    provider.wait_for_calls(1).await;

    let waited = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        invoke_agent(
            &tool,
            serde_json::json!({ "action": "wait", "child_id": "child-1" }),
            &ctx,
        ),
    )
    .await
    .expect("the foreground wait releases the parent")
    .expect("a wait outcome");
    let waited = serde_json::to_string(&waited.into_result_block(
        ToolCallId::new("call-2"),
        AGENT_TOOL_NAME.to_owned(),
        100_000,
    ))
    .expect("json");
    assert!(waited.contains(r#"\"state\":\"running\""#), "{waited}");
    assert!(waited.contains(r#"\"timed_out\":true"#), "{waited}");

    let children = delegation.coordinator().expect("a coordinator").list();
    assert!(matches!(children[0].state, ChildState::Running));

    invoke_agent(
        &tool,
        serde_json::json!({ "action": "stop", "child_id": "child-1" }),
        &ctx,
    )
    .await
    .expect("a stop outcome");
    session.shutdown().await.expect("a clean shutdown");
}
