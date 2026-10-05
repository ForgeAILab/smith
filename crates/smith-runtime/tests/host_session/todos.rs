use super::*;

#[tokio::test]
async fn todo_lifecycle_is_checkpointed_renderable_and_restored_as_context() {
    let fixture = Fixture::new();
    let write = |call: &str, items: serde_json::Value| {
        let mut events = tool_call_fragments(
            0,
            call,
            "write_todos",
            &serde_json::json!({ "items": items }).to_string(),
        );
        events.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        ScriptedStream::new(events)
    };
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            write(
                "plan-1",
                serde_json::json!([
                    {"id":"inspect","text":"Inspect the implementation","status":"in_progress"},
                    {"id":"change","text":"Implement the change","status":"pending"},
                    {"id":"verify","text":"Run focused tests","status":"pending"}
                ]),
            ),
            write(
                "plan-2",
                serde_json::json!([
                    {"id":"inspect","text":"Inspect the implementation","status":"completed"},
                    {"id":"change","text":"Implement the change","status":"completed"},
                    {"id":"verify","text":"Run focused tests","status":"in_progress"}
                ]),
            ),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "plan updated".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let observer = RecordingObserver::shared();
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone());
    request.runtime.observers.push(observer.clone());
    let host = start(request).await.expect("a hosted session");
    assert!(
        host.runtime()
            .policy()
            .tools
            .iter()
            .any(|tool| tool == "write_todos")
    );

    host.session()
        .run(UserInput::text(
            "Use write_todos to track this genuinely multi-step edit.",
        ))
        .await
        .expect("the todo lifecycle completes");
    let payloads = observer.payloads();
    let plan_events = payloads
        .iter()
        .cloned()
        .filter_map(|event| match event {
            RuntimeEvent::PlanUpdated {
                revision,
                sensitivity,
                counts,
                items,
            } => Some((revision, sensitivity, counts, items)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        plan_events.len(),
        3,
        "events: {payloads:?}; requests: {:?}",
        provider.requests()
    );
    assert_eq!(plan_events[0].0, 1);
    assert_eq!(plan_events[1].0, 2);
    assert_eq!(plan_events[2].0, 3);
    assert_eq!(
        plan_events[1].1,
        agent_runtime_core::event::PlanSensitivity::Public
    );
    assert_eq!(plan_events[1].2["in_progress"], 1);
    assert_eq!(
        plan_events[1].3.as_ref().expect("public item projection")[2].text,
        "Run focused tests"
    );
    assert_eq!(plan_events[2].2["in_progress"], 0);
    assert_eq!(plan_events[2].2["pending"], 0);
    assert_eq!(plan_events[2].2["cancelled"], 1);
    let terminal_verify = &plan_events[2].3.as_ref().expect("terminal public plan")[2];
    assert_eq!(
        terminal_verify.status,
        agent_runtime_core::event::PlanItemStatus::Cancelled
    );
    assert_eq!(
        terminal_verify.reason.as_deref(),
        Some("turn_ended_unfinished")
    );

    let snapshot = host.session().snapshot();
    let state = snapshot
        .extension_state
        .get("harness.todo.state")
        .expect("checkpointed todo state");
    assert_eq!(state.sensitivity, SessionStateSensitivity::RedactionSafe);
    assert_eq!(state.value["revision"], 3);
    let session = host.session().id().clone();
    host.shutdown().await.expect("clean shutdown");

    let resumed_provider = Arc::new(FakeProvider::text_reply("resumed with plan"));
    let mut resume = fixture
        .request(HostSurface::Headless)
        .resume(session.clone());
    resume.runtime.provider = Some(resumed_provider.clone());
    let resumed = start(resume).await.expect("the todo session resumes");
    resumed
        .session()
        .run(UserInput::text("Continue the multi-step work."))
        .await
        .expect("the restored plan contributes to the next request");
    let requests = resumed_provider.requests();
    let wire = serde_json::to_string(&requests[0].messages).expect("provider messages");
    assert!(wire.contains(r#"<todo_plan revision=\"3\">"#), "{wire}");
    assert!(wire.contains("[cancelled] verify"), "{wire}");
    assert!(wire.contains("Run focused tests"), "{wire}");
    resumed.shutdown().await.expect("clean resumed shutdown");
}
