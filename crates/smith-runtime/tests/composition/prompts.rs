use super::*;

#[tokio::test]
async fn provider_planning_records_every_versioned_smith_prompt_fragment() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let provider = Arc::new(FakeProvider::text_reply("done"));
    let project_instructions =
        ProjectInstructionsSnapshot::from_body("PROJECT_PROMPT_MARKER: run exact checks.")
            .expect("bounded project instructions");
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        project_instructions: Some(project_instructions.clone()),
        ..request(&fixture, HostSurface::Headless)
    };
    let smith = factory::build_request(request).await.expect("a runtime");
    assert_eq!(
        smith
            .policy()
            .project_instructions
            .as_ref()
            .expect("project composition evidence"),
        &project_instructions.identity()
    );
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("explain this project"))
        .await
        .expect("the turn runs");

    let expected = smith_runtime::prompt::fragments(&smith_runtime::prompt::DynamicPromptContext {
        project_instructions: Some(project_instructions),
        agent_profile: Some(smith_runtime::prompt::AgentProfilePrompt {
            name: "dev".to_owned(),
            posture: AgentPosture::Build,
            instructions: None,
            revision: smith.policy().agent_profile_revision.clone(),
        }),
        todo_planning: true,
        questionnaire: true,
        delegation: true,
        ..smith_runtime::prompt::DynamicPromptContext::default()
    });
    let snapshot = session.snapshot();
    let manifest = snapshot.manifests.last().expect("a run manifest");
    let smith_segments = manifest
        .manifest
        .segments
        .iter()
        .filter(|segment| segment.id.as_str().starts_with("smith.prompt."))
        .collect::<Vec<_>>();
    assert_eq!(smith_segments.len(), expected.len());
    for (record, fragment) in smith_segments.iter().zip(expected) {
        assert_eq!(record.id.as_str(), fragment.id.as_str());
        assert_eq!(record.content_hash, fragment.content_hash());
    }

    let wire_text = provider.requests()[0]
        .messages
        .iter()
        .map(|message| message.joined_text())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(wire_text.contains("understand the request"));
    assert!(wire_text.contains("committed successful tool result"));
    assert!(wire_text.contains("PROJECT_PROMPT_MARKER"));
    session.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn a_complete_prompt_override_ignores_project_instructions() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let provider = Arc::new(FakeProvider::text_reply("done"));
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        system_prompt: Some("COMPLETE_HOST_OVERRIDE_MARKER".to_owned()),
        project_instructions: Some(
            ProjectInstructionsSnapshot::from_body("PROJECT_INSTRUCTIONS_MUST_BE_ABSENT")
                .expect("bounded project instructions"),
        ),
        ..request(&fixture, HostSurface::Headless)
    };
    let smith = factory::build_request(request).await.expect("a runtime");
    assert_eq!(smith.policy().project_instructions, None);
    assert!(
        smith
            .policy()
            .system_prompt
            .contains("COMPLETE_HOST_OVERRIDE_MARKER")
    );
    assert!(
        !smith
            .policy()
            .system_prompt
            .contains("PROJECT_INSTRUCTIONS_MUST_BE_ABSENT")
    );

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("answer once"))
        .await
        .expect("a turn");
    let wire = serde_json::to_string(&provider.requests()[0].messages).expect("provider request");
    assert!(wire.contains("COMPLETE_HOST_OVERRIDE_MARKER"), "{wire}");
    assert!(
        !wire.contains("PROJECT_INSTRUCTIONS_MUST_BE_ABSENT"),
        "{wire}"
    );
    session.shutdown().await.expect("a clean shutdown");
}
