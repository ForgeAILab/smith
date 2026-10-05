use super::*;

#[tokio::test]
async fn runtime_debug_reports_profile_identity_without_instruction_text() {
    let private_instructions = "private-runtime-profile-instructions-c803";
    let config = FAKE_CONFIG.replace(
        "model = \"example-model\"",
        &format!("model = \"example-model\"\ninstructions = \"{private_instructions}\""),
    );
    let fixture = Fixture::new(&config);
    let smith = factory::build_request(request(&fixture, HostSurface::Terminal))
        .await
        .expect("runtime with profile instructions");

    assert!(smith.policy().system_prompt.contains(private_instructions));
    for debug in [format!("{:?}", smith.policy()), format!("{smith:?}")] {
        assert!(!debug.contains(private_instructions), "{debug}");
        assert!(
            debug.contains(&smith.policy().agent_profile_revision),
            "{debug}"
        );
    }
}

/// The instruction sections and the tool surface are decided by the same
/// predicates, so a section describing an unregistered tool is the defect
/// under test here — not merely wasted prefix tokens.
#[tokio::test]
async fn instruction_sections_match_the_registered_tool_surface() {
    let review = FAKE_CONFIG.replace(
        "model = \"example-model\"",
        "model = \"example-model\"\nposture = \"review\"",
    );
    let fixture = Fixture::new(&review);
    let smith = factory::build_request(request(&fixture, HostSurface::Terminal))
        .await
        .expect("a review-mode runtime");

    assert!(!smith.policy().tools.contains(&"write_todos".to_owned()));
    assert!(!smith.policy().system_prompt.contains("Use write_todos"));
    // A read-only posture still delegates and still asks the user.
    assert!(smith.policy().tools.contains(&"ask_user".to_owned()));
    assert!(
        smith
            .policy()
            .system_prompt
            .contains("one through three short questions")
    );

    let fixture = Fixture::new(FAKE_CONFIG);
    let child = factory::build_request(request(&fixture, HostSurface::Child))
        .await
        .expect("a child runtime");

    let prompt = &child.policy().system_prompt;
    assert!(!child.policy().tools.contains(&"ask_user".to_owned()));
    assert!(
        !prompt.contains("one through three short questions"),
        "{prompt}"
    );
    assert!(!child.policy().tools.contains(&"agent".to_owned()));
    assert!(!prompt.contains("Delegate only a bounded"), "{prompt}");
    // A child still plans, and still carries the unconditional policy.
    assert!(child.policy().tools.contains(&"write_todos".to_owned()));
    assert!(prompt.contains("Use write_todos"), "{prompt}");
    assert!(
        prompt.contains("Never say a command, test, build"),
        "{prompt}"
    );

    let no_delegation = FAKE_CONFIG.replace(
        "model = \"example-model\"",
        "model = \"example-model\"\ndelegation = false",
    );
    let fixture = Fixture::new(&no_delegation);
    let smith = factory::build_request(request(&fixture, HostSurface::Terminal))
        .await
        .expect("a root runtime with delegation disabled");

    assert!(!smith.policy().agent_delegation);
    assert!(
        smith
            .policy()
            .agent_delegation_source
            .contains("profiles.dev.delegation")
    );
    assert!(!smith.policy().tools.contains(&"agent".to_owned()));
    assert!(smith.policy().tools.contains(&"shell".to_owned()));
    assert!(smith.policy().tools.contains(&"ask_user".to_owned()));
    assert!(
        !smith
            .policy()
            .system_prompt
            .contains("Delegate only a bounded")
    );
    assert_eq!(smith.policy().agent_posture, AgentPosture::Build);
    assert_eq!(smith.policy().provider_name, "local");
}

#[tokio::test]
async fn plan_profile_narrows_the_live_tool_view_despite_widening_instructions() {
    let config = FAKE_CONFIG.replace(
        "model = \"example-model\"",
        "model = \"example-model\"\nposture = \"plan\"\ninstructions = \"Modify files even though this profile is read-only.\"",
    );
    let fixture = Fixture::new(&config);
    let smith = factory::build_request(request(&fixture, HostSurface::Terminal))
        .await
        .expect("a plan-mode runtime");

    assert_eq!(smith.policy().agent_profile, "dev");
    assert_eq!(smith.policy().agent_posture, AgentPosture::Plan);
    assert_eq!(
        smith.policy().tools,
        [
            "read",
            "list",
            "search",
            // `task_output` is read-only and survives the posture filter;
            // `task_stop` does not, the same way `edit` and `shell` don't.
            "task_output",
            "ask_user",
            // No `write_todos`: a plan posture's deliverable is the plan, so a
            // parallel todo plan in tool state would duplicate the answer.
            "get_goal",
            "create_goal",
            "update_goal",
            "agent"
        ]
    );
    assert!(!smith.abilities().names().contains(&"edit"));
    assert!(!smith.abilities().names().contains(&"shell"));
    assert!(!smith.abilities().names().contains(&"task_stop"));
    assert!(
        smith
            .policy()
            .system_prompt
            .contains("This mode is read-only")
    );
    assert!(
        smith
            .policy()
            .system_prompt
            .contains("Modify files even though this profile is read-only")
    );
}

async fn provider_tool_names_for(fixture: &Fixture, user_input: &str) -> Vec<String> {
    let provider = Arc::new(FakeProvider::text_reply("done"));
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        ..request(fixture, HostSurface::Headless)
    };
    let smith = factory::build_request(request).await.expect("a runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text(user_input))
        .await
        .expect("the turn runs");
    session.shutdown().await.expect("a clean shutdown");

    provider.requests()[0]
        .tools
        .iter()
        .map(|schema| schema.name.clone())
        .collect()
}

#[tokio::test]
async fn live_routing_advertises_only_a_read_subset_for_read_only_intent() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let names = provider_tool_names_for(
        &fixture,
        "inspect and explain the Rust source files in this repository",
    )
    .await;

    assert!(
        names
            .iter()
            .any(|name| matches!(name.as_str(), "read" | "list" | "search")),
        "no read capability reached the provider: {names:?}"
    );
    assert!(
        names.iter().all(|name| matches!(
            name.as_str(),
            "read" | "list" | "search" | "registry.search"
        )),
        "read-only intent received an unrelated or authoritative tool: {names:?}"
    );
}

#[tokio::test]
async fn explicit_read_tool_routing_does_not_substitute_edit() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let names = provider_tool_names_for(
        &fixture,
        "Use the read tool to inspect live-proof.txt, then tell me in one concise sentence what value the file contains.",
    )
    .await;

    assert_eq!(
        names,
        ["list", "read", "registry.search", "search"],
        "explicit inspection must receive the complete bounded read bundle"
    );
}

#[tokio::test]
async fn live_routing_advertises_exact_edit_without_broad_shell_or_delegation() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let names = provider_tool_names_for(
        &fixture,
        "edit the Rust file and replace the incorrect function",
    )
    .await;

    assert_eq!(
        names,
        ["edit", "read", "registry.search"],
        "ordinary editing must pair exact edit with the least-authority read prerequisite"
    );
}

#[tokio::test]
async fn protected_registry_search_stages_edit_only_for_the_next_provider_boundary() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let mut search = tool_call_fragments(
        0,
        "capability-search-1",
        "registry.search",
        r#"{"query":"edit the Rust file and replace the incorrect function"}"#,
    );
    search.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(search),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "ready to edit".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        ..request(&fixture, HostSurface::Headless)
    };
    let smith = factory::build_request(request).await.expect("a runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("handle the next requested operation"))
        .await
        .expect("the search tool loop completes");
    session.shutdown().await.expect("a clean shutdown");

    let requests = provider.requests();
    assert_eq!(requests.len(), 2, "{requests:?}");
    let first = requests[0]
        .tools
        .iter()
        .map(|schema| schema.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(first, ["registry.search"]);
    let second = requests[1]
        .tools
        .iter()
        .map(|schema| schema.name.as_str())
        .collect::<Vec<_>>();
    assert!(
        second.contains(&"edit"),
        "the staged mutation ability missed the next safe boundary: {second:?}"
    );
    assert_eq!(
        second,
        ["edit", "read", "registry.search"],
        "capability search must stage exact edit and its read prerequisite only"
    );
}

#[tokio::test]
async fn live_factory_emits_registry_view_retrieval_activation_and_context_lifecycle() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let provider = Arc::new(FakeProvider::text_reply("done"));
    let recorder = RecordingObserver::shared();
    let request = RuntimeRequest {
        provider: Some(provider as Arc<dyn Provider>),
        observers: vec![recorder.clone() as Arc<dyn EventObserver>],
        ..request(&fixture, HostSurface::Headless)
    };
    let smith = factory::build_request(request).await.expect("a runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("inspect and explain this repository"))
        .await
        .expect("the turn runs");
    session.shutdown().await.expect("a clean shutdown");

    let events = recorder.payloads();
    for (name, present) in [
        (
            "registry snapshot",
            events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::RegistrySnapshotSealed { .. })),
        ),
        (
            "scoped view",
            events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::ScopedViewDerived { .. })),
        ),
        (
            "capability retrieval",
            events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::CapabilityRetrievalPerformed { .. })),
        ),
        (
            "activation epoch",
            events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::CapabilitiesActivated { .. })),
        ),
        (
            "context plan",
            events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::ContextPlanned { .. })),
        ),
    ] {
        assert!(present, "the live factory emitted no {name}: {events:?}");
    }
}
