use super::*;

use agent_runtime_core::ids::ChildId;
use smith_runtime::delegation::{ChildTranscriptEntry, project_child_transcript_history};
use smith_runtime::journal::DefaultRedactor;
use smith_runtime::tool_output::ToolOutputContextPolicy;

fn context(session: &SessionId) -> InvocationContext {
    InvocationContext {
        session: session.clone(),
        turn: None,
        call_id: ToolCallId::new("transcript-read"),
        request: RequestId::new("transcript-request"),
        workspace: Arc::new(LexicalWorkspace::new("/repo")),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 65536,
    }
}

fn tool(
    coordinator: &agent_runtime::delegation::DelegationCoordinator,
    store: Option<Arc<dyn ArtifactStore>>,
    redactor: DefaultRedactor,
) -> AgentTool {
    let slot = Arc::new(OnceLock::new());
    slot.set(coordinator.clone()).unwrap();
    AgentTool::new(slot).with_transcript_output(
        store,
        redactor,
        ToolOutputContextPolicy {
            inline_bytes: 8192,
            artifact_page_bytes: 2048,
            reply_bytes: 65536,
        },
    )
}

#[test]
fn local_projection_excludes_instructions_hidden_reasoning_and_credentials() {
    use agent_runtime_core::content::{Message, ToolCall, ToolResultBlock};

    let reasoning = |text: &str, redacted| ContentPart::Reasoning {
        text: text.into(),
        redacted,
        signature: Some("PROTECTED_SIGNATURE".into()),
        producer: None,
    };
    let canonical = vec![
        Message::system("INTERNAL_INSTRUCTIONS"),
        Message::user("task with TOP_SECRET"),
        Message::assistant(vec![
            reasoning("visible explanation TOP_SECRET", false),
            reasoning("HIDDEN_REASONING", true),
            ContentPart::ToolCall(ToolCall {
                id: ToolCallId::new("lookup"),
                name: "search".into(),
                arguments: serde_json::json!({"api_key": "STRUCTURAL_SECRET", "query": "TOP_SECRET"}),
            }),
        ]),
        Message::tool_result(ToolResultBlock {
            call_id: ToolCallId::new("lookup"),
            name: "search".into(),
            content: vec![
                ContentPart::text("result TOP_SECRET"),
                reasoning("NESTED_HIDDEN", true),
            ],
            is_error: false,
        }),
    ];
    let projected = project_child_transcript_history(
        &canonical,
        &DefaultRedactor::new().with_secret("TOP_SECRET"),
    );
    let wire = serde_json::to_string(&projected).unwrap();
    for excluded in [
        "INTERNAL_INSTRUCTIONS",
        "PROTECTED_SIGNATURE",
        "HIDDEN_REASONING",
        "NESTED_HIDDEN",
        "TOP_SECRET",
        "STRUCTURAL_SECRET",
    ] {
        assert!(!wire.contains(excluded), "{wire}");
    }
    assert!(wire.contains("visible explanation [redacted]"));
    assert!(wire.contains("result [redacted]"));
    assert_eq!(
        projected[1].tool_calls().next().unwrap().id.as_str(),
        "lookup"
    );
    assert!(
        serde_json::to_string(&canonical)
            .unwrap()
            .contains("TOP_SECRET")
    );
}

#[tokio::test]
async fn transcript_pins_pages_without_consuming_completion_or_executing_work() {
    let fixture = Fixture::new();
    let provider = scripted(2, "a completed reply");
    let smith = factory::build_request(request(&fixture, provider.clone()))
        .await
        .unwrap();
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let delegation = smith.delegation().unwrap();
    let lifecycle = wire_delegation(&session, delegation).await.unwrap();
    lifecycle.shutdown().await;
    let coordinator = delegation.coordinator().unwrap();
    let tool = tool(coordinator, None, DefaultRedactor::new());
    let ctx = context(session.id());
    invoke_agent(
        &tool,
        serde_json::json!({"action": "spawn", "task": "initial task"}),
        &ctx,
    )
    .await
    .unwrap();
    let child = ChildId::new("child-1");
    coordinator.wait_task_outcome(&child).await.unwrap();
    let cursor_before = coordinator.child_outcome_cursor();
    let calls_before = provider.requests().len();
    let first = invoke_agent(
        &tool,
        serde_json::json!({
            "action": "transcript", "child_id": "child-1", "page_size": 1
        }),
        &ctx,
    )
    .await
    .unwrap();
    assert!(!first.is_error, "{:?}", first.value);
    assert_eq!(first.value["entries"][0]["role"], "user");
    assert_eq!(first.value["entries"][0]["text"], "initial task");
    assert_eq!(coordinator.child_outcome_cursor(), cursor_before);
    assert_eq!(provider.requests().len(), calls_before);

    coordinator
        .follow_up(&child, UserInput::text("later follow-up task"))
        .await
        .unwrap();
    coordinator.wait_task_outcome(&child).await.unwrap();
    let calls_after_followup = provider.requests().len();
    let ready_after_followup = coordinator.take_ready_task_outcomes();
    assert!(!ready_after_followup.is_empty());
    let second = invoke_agent(&tool, serde_json::json!({
        "action": "transcript", "child_id": "child-1", "cursor": first.value["next_cursor"], "page_size": 100
    }), &ctx).await.unwrap();
    assert!(!second.is_error, "{:?}", second.value);
    assert_eq!(second.value["entries"].as_array().unwrap().len(), 1);
    assert_eq!(second.value["entries"][0]["role"], "assistant");
    assert_eq!(second.value["next_cursor"], serde_json::Value::Null);
    assert_eq!(second.value["watermark"], first.value["watermark"]);
    assert!(!second.value.to_string().contains("later follow-up task"));

    let fresh = invoke_agent(
        &tool,
        serde_json::json!({"action": "transcript", "child_id": "child-1"}),
        &ctx,
    )
    .await
    .unwrap();
    assert!(fresh.value.to_string().contains("later follow-up task"));
    assert_eq!(provider.requests().len(), calls_after_followup);
    assert_eq!(
        coordinator.take_ready_task_outcomes(),
        ready_after_followup,
        "diagnostic reads must leave completion eligible for delivery"
    );

    for args in [
        serde_json::json!({"action":"transcript", "child_id":"missing"}),
        serde_json::json!({"action":"transcript", "child_id":"child-1", "cursor":"invalid"}),
        serde_json::json!({"action":"transcript", "child_id":"child-1", "page_size":0}),
        serde_json::json!({"action":"transcript", "child_id":"child-1", "page_size":101}),
    ] {
        assert!(invoke_agent(&tool, args, &ctx).await.unwrap().is_error);
    }
    let mut unrelated = context(&SessionId::new("unrelated-root"));
    unrelated.call_id = ToolCallId::new("unrelated-read");
    let denied = invoke_agent(
        &tool,
        serde_json::json!({"action":"transcript", "child_id":"child-1"}),
        &unrelated,
    )
    .await
    .unwrap();
    assert!(denied.is_error);
    assert!(denied.value.to_string().contains("owning root"));
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn running_child_history_is_read_without_new_provider_work() {
    let fixture = Fixture::new();
    let provider = Arc::new(CrashThenReplyProvider::new());
    let smith = factory::build_request(request(&fixture, provider.clone()))
        .await
        .unwrap();
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let delegation = smith.delegation().unwrap();
    let lifecycle = wire_delegation(&session, delegation).await.unwrap();
    lifecycle.shutdown().await;
    let coordinator = delegation.coordinator().unwrap();
    let tool = tool(coordinator, None, DefaultRedactor::new());
    let ctx = context(session.id());
    invoke_agent(
        &tool,
        serde_json::json!({"action":"spawn", "task":"inspect this running task"}),
        &ctx,
    )
    .await
    .unwrap();
    provider.wait_for_calls(1).await;
    let page = invoke_agent(
        &tool,
        serde_json::json!({"action":"transcript", "child_id":"child-1"}),
        &ctx,
    )
    .await
    .unwrap();
    assert!(!page.is_error, "{:?}", page.value);
    assert_eq!(page.value["state"], "running");
    assert_eq!(
        page.value["entries"][0]["text"],
        "inspect this running task"
    );
    assert_eq!(provider.requests().len(), 1);
    assert!(matches!(
        coordinator.status(&ChildId::new("child-1")).unwrap().state,
        ChildState::Running
    ));
    coordinator.stop(&ChildId::new("child-1")).await.unwrap();
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn oversized_replies_are_bounded_redacted_and_parent_readable() {
    let fixture = Fixture::new();
    let reply = format!(
        "BEGIN\n{}\nTOP_SECRET\nEND",
        "a complete report line\n".repeat(1500)
    );
    let provider = scripted(1, &reply);
    let store = Arc::new(SmithArtifactStore::new(
        smith_runtime::host::paths(&fixture.config(), fixture.project.path()).unwrap(),
    ));
    let mut req = request(&fixture, provider.clone());
    req.artifact_store = Some(store.clone());
    let smith = factory::build_request(req).await.unwrap();
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let delegation = smith.delegation().unwrap();
    let lifecycle = wire_delegation(&session, delegation).await.unwrap();
    lifecycle.shutdown().await;
    let coordinator = delegation.coordinator().unwrap();
    let tool = tool(
        coordinator,
        Some(store.clone()),
        DefaultRedactor::new().with_secret("TOP_SECRET"),
    );
    let ctx = context(session.id());
    invoke_agent(
        &tool,
        serde_json::json!({"action":"spawn", "task":"write the report"}),
        &ctx,
    )
    .await
    .unwrap();
    coordinator
        .wait_task_outcome(&ChildId::new("child-1"))
        .await
        .unwrap();
    let page = invoke_agent(
        &tool,
        serde_json::json!({"action":"transcript", "child_id":"child-1"}),
        &ctx,
    )
    .await
    .unwrap();
    assert!(!page.is_error, "{:?}", page.value);
    assert!(serde_json::to_vec(&page.value).unwrap().len() <= 8192);
    let archived: ChildTranscriptEntry =
        serde_json::from_value(page.value["entries"][1].clone()).unwrap();
    let reference = archived.artifact.unwrap();
    assert_eq!(reference.provenance.session, *session.id());
    let bytes = store
        .read(ArtifactRead {
            session: session.id().clone(),
            id: reference.id.clone(),
            offset: 0,
            limit: MAX_ARTIFACT_READ_BYTES,
        })
        .await
        .unwrap()
        .bytes;
    let complete: ChildTranscriptEntry = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        complete.text.as_deref(),
        Some(reply.replace("TOP_SECRET", "[redacted]").as_str())
    );
    assert!(
        store
            .read(ArtifactRead {
                session: SessionId::new("another-parent"),
                id: reference.id,
                offset: 0,
                limit: MAX_ARTIFACT_READ_BYTES,
            })
            .await
            .is_err()
    );
    // The same entry remains idempotently readable from another tool call.
    let again = invoke_agent(
        &tool,
        serde_json::json!({"action":"transcript", "child_id":"child-1"}),
        &context(session.id()),
    )
    .await
    .unwrap();
    assert!(!again.is_error, "{:?}", again.value);
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn archived_tool_results_keep_call_pairing_and_parent_readable_complete_content() {
    let fixture = Fixture::new();
    let complete = format!(
        "BEGIN\n{}\nTOP_SECRET\nEND",
        "retained source evidence\n".repeat(700)
    );
    std::fs::write(fixture.project.path().join("large.txt"), &complete).unwrap();
    let mut read = tool_call_fragments(
        0,
        "read-evidence",
        "read",
        r#"{"path":"large.txt","limit":10000}"#,
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
                    text: "evidence reviewed".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let store = Arc::new(SmithArtifactStore::new(
        smith_runtime::host::paths(&fixture.config(), fixture.project.path()).unwrap(),
    ));
    let mut req = request(&fixture, provider.clone());
    req.workspace = Some(Arc::new(
        ProjectWorkspace::new(fixture.project.path()).unwrap(),
    ));
    req.artifact_store = Some(store.clone());
    let smith = factory::build_request(req).await.unwrap();
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    let delegation = smith.delegation().unwrap();
    let lifecycle = wire_delegation(&session, delegation).await.unwrap();
    lifecycle.shutdown().await;
    let coordinator = delegation.coordinator().unwrap();
    let tool = tool(
        coordinator,
        Some(store.clone()),
        DefaultRedactor::new().with_secret("TOP_SECRET"),
    );
    let ctx = context(session.id());
    invoke_agent(
        &tool,
        serde_json::json!({"action":"spawn", "task":"read large.txt"}),
        &ctx,
    )
    .await
    .unwrap();
    coordinator
        .wait_task_outcome(&ChildId::new("child-1"))
        .await
        .unwrap();
    let page = invoke_agent(
        &tool,
        serde_json::json!({"action":"transcript", "child_id":"child-1"}),
        &ctx,
    )
    .await
    .unwrap();
    assert!(!page.is_error, "{:?}", page.value);
    assert_eq!(page.value["unavailable"], serde_json::json!([]));
    let entries: Vec<ChildTranscriptEntry> =
        serde_json::from_value(page.value["entries"].clone()).unwrap();
    let call = entries
        .iter()
        .find(|entry| entry.kind == "tool_call")
        .unwrap();
    let result = entries
        .iter()
        .find(|entry| entry.kind == "tool_result")
        .unwrap();
    assert_eq!(call.call_id, result.call_id);
    assert_eq!(call.tool_name.as_deref(), Some("read"));
    let reference = result
        .artifact
        .as_ref()
        .expect("large result gets an accessible parent copy");
    assert_eq!(reference.provenance.session, *session.id());
    let mut bytes = Vec::new();
    let mut offset = 0;
    loop {
        let chunk = store
            .read(ArtifactRead {
                session: session.id().clone(),
                id: reference.id.clone(),
                offset,
                limit: MAX_ARTIFACT_READ_BYTES,
            })
            .await
            .unwrap();
        bytes.extend(chunk.bytes);
        match chunk.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }
    let restored: ChildTranscriptEntry = serde_json::from_slice(&bytes).unwrap();
    let text = restored.text.unwrap();
    assert!(text.contains("BEGIN") && text.contains("END"));
    assert!(text.contains("retained source evidence"));
    assert!(!text.contains("TOP_SECRET"));
    assert!(text.contains("[redacted]"));
    assert_eq!(provider.requests().len(), 2);
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_dormant_child_transcript_is_readable_after_parent_restart() {
    let fixture = Fixture::new();
    let provider = scripted(1, "retained child answer");
    let sessions = Arc::new(InMemorySessionStore::new());
    let checkpoints = Arc::new(InMemoryCheckpointStore::new());
    let parent_id = SessionId::new("transcript-durable-parent");
    let build = || {
        let mut req = request(&fixture, provider.clone());
        req.session_store = Some(sessions.clone());
        req.checkpoint_store = Some(checkpoints.clone());
        req
    };
    let first = factory::build_request(build()).await.unwrap();
    let first_session = first
        .runtime()
        .start_session(StartSession::create(parent_id.clone(), Vec::new()))
        .await
        .unwrap();
    let first_delegation = first.delegation().unwrap();
    let lifecycle = wire_delegation(&first_session, first_delegation)
        .await
        .unwrap();
    lifecycle.shutdown().await;
    let coordinator = first_delegation.coordinator().unwrap();
    let first_tool = tool(coordinator, None, DefaultRedactor::new());
    invoke_agent(
        &first_tool,
        serde_json::json!({"action":"spawn", "task":"a retained delegated task"}),
        &context(first_session.id()),
    )
    .await
    .unwrap();
    let child = ChildId::new("child-1");
    coordinator.wait_task_outcome(&child).await.unwrap();
    coordinator.flush().await.unwrap();
    first_session.shutdown().await.unwrap();

    let second = factory::build_request(build()).await.unwrap();
    let second_session = second
        .runtime()
        .start_session(StartSession::resume(parent_id))
        .await
        .unwrap();
    let second_delegation = second.delegation().unwrap();
    let lifecycle = wire_delegation(&second_session, second_delegation)
        .await
        .unwrap();
    lifecycle.shutdown().await;
    let coordinator = second_delegation.coordinator().unwrap();
    assert!(
        coordinator.child_events(&child).is_none(),
        "history inspection must not create a live handle"
    );
    let second_tool = tool(coordinator, None, DefaultRedactor::new());
    let page = invoke_agent(
        &second_tool,
        serde_json::json!({"action":"transcript", "child_id":"child-1"}),
        &context(second_session.id()),
    )
    .await
    .unwrap();
    assert!(!page.is_error, "{:?}", page.value);
    assert!(page.value.to_string().contains("a retained delegated task"));
    assert!(page.value.to_string().contains("retained child answer"));
    assert_eq!(provider.requests().len(), 1);
    assert!(coordinator.child_events(&child).is_none());
    second_session.shutdown().await.unwrap();
}
