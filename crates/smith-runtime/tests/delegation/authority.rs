use super::*;

#[test]
fn the_agent_ability_advertises_its_host_defined_delegation_authority() {
    let tool = Arc::new(AgentTool::new(Arc::new(OnceLock::new())));
    let descriptor = ToolAbility::new(tool).descriptor();

    assert_eq!(descriptor.risk(), RiskLevel::High);
    assert!(
        descriptor
            .permissions()
            .contains(&Permission::other(DELEGATION_PERMISSION.to_owned()))
    );
    assert!(
        descriptor
            .affordances()
            .iter()
            .any(|affordance| affordance.as_str() == "host-defined-authority")
    );

    let description = AgentTool::new(Arc::new(OnceLock::new())).spec().description;
    assert!(description.contains("does not open user interface"));
    assert!(description.contains("root ask_user"));
    assert!(description.contains("explicit follow_up"));
}

#[tokio::test]
async fn an_unknown_agent_action_is_returned_to_the_model_as_a_tool_error() {
    let fixture = Fixture::new();
    let mut invalid_call = tool_call_fragments(
        0,
        "invalid-agent-action",
        AGENT_TOOL_NAME,
        r#"{"action":"gemini-review"}"#,
    );
    invalid_call.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(invalid_call),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "corrected after the tool error".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let smith = factory::build_request(request(&fixture, provider.clone()))
        .await
        .expect("a root runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a delegation surface");
    wire_delegation(&session, delegation)
        .await
        .expect("delegation wiring");

    session
        .run(UserInput::text("delegate a review to a sub-agent"))
        .await
        .expect("the model can recover from its invalid agent action");

    let requests = provider.requests();
    assert_eq!(requests.len(), 2, "the tool error must continue the loop");
    assert!(
        requests[0]
            .tools
            .iter()
            .any(|schema| schema.name == AGENT_TOOL_NAME),
        "the provider must have received the agent schema"
    );
    let error = requests[1]
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|part| match part {
            ContentPart::ToolResult(result)
                if result.call_id.as_str() == "invalid-agent-action" =>
            {
                Some(result)
            }
            _ => None,
        })
        .expect("the continuation request contains the invalid call's result");
    assert!(error.is_error);
    let error_text = error
        .content
        .iter()
        .filter_map(|part| part.as_text())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(error_text.contains("gemini-review"), "{error_text}");
    assert!(error_text.contains("spawn"), "{error_text}");

    session.shutdown().await.expect("a clean shutdown");
}

/// A child surface never composes the delegation tool.
#[tokio::test]
async fn a_child_surface_gets_no_delegation_tool() {
    let fixture = Fixture::new();
    let smith = factory::build_request(RuntimeRequest {
        workspace: Some(Arc::new(MemoryWorkspace::new("/repo"))),
        provider: Some(scripted(1, "child reply")),
        ..RuntimeRequest::new(fixture.config(), HostSurface::Child)
    })
    .await
    .expect("a child runtime");
    assert!(smith.delegation().is_none());
    assert!(
        !smith
            .policy()
            .tools
            .iter()
            .any(|name| name == AGENT_TOOL_NAME)
    );
}
