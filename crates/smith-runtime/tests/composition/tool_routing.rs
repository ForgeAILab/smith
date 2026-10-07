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

const CORE_TOOLS: [&str; 7] = [
    "edit",
    "list",
    "read",
    "search",
    "shell",
    "task_output",
    "task_stop",
];

#[tokio::test]
async fn the_core_tools_reach_the_first_request_whatever_the_prompt_says() {
    let fixture = Fixture::new(FAKE_CONFIG);
    // The second prompt is the one that, before the core set was pinned,
    // matched no keyword and left the model believing it had no terminal.
    for prompt in ["hi", "scan oc.example.com and tell me what is exposed"] {
        let names = provider_tool_names_for(&fixture, prompt).await;
        for tool in CORE_TOOLS {
            assert!(
                names.iter().any(|name| name == tool),
                "`{prompt}` lacks {tool}: {names:?}"
            );
        }
        assert!(
            names.iter().any(|name| name == "registry.search"),
            "{names:?}"
        );
        assert!(
            names.iter().any(|name| name == "registry.activate"),
            "{names:?}"
        );
        assert!(
            !names.iter().any(|name| name == "agent"),
            "capabilities outside the core still arrive by discovery: {names:?}"
        );
    }
}

#[tokio::test]
async fn a_read_only_posture_pins_only_the_read_subset() {
    let config = FAKE_CONFIG.replace(
        "model = \"example-model\"",
        "model = \"example-model\"\nposture = \"plan\"",
    );
    let fixture = Fixture::new(&config);
    let names = provider_tool_names_for(&fixture, "edit the Rust file and run the tests").await;

    for tool in ["read", "list", "search", "task_output"] {
        assert!(
            names.iter().any(|name| name == tool),
            "{tool} missing: {names:?}"
        );
    }
    for tool in ["edit", "shell", "task_stop"] {
        assert!(
            !names.iter().any(|name| name == tool),
            "{tool} leaked: {names:?}"
        );
    }
}

#[tokio::test]
async fn a_denied_capability_is_absent_for_the_profile_and_for_the_session() {
    let by_profile = Fixture::new(&format!(
        "{FAKE_CONFIG}\n[profiles.dev.capabilities]\ndeny = [\"tool:shell\"]\n"
    ));
    let names = provider_tool_names_for(&by_profile, "run the tests in the shell").await;
    assert!(!names.iter().any(|name| name == "shell"), "{names:?}");
    assert!(names.iter().any(|name| name == "edit"), "{names:?}");

    let fixture = Fixture::new(FAKE_CONFIG);
    let provider = Arc::new(FakeProvider::text_reply("done"));
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        capability_denials: vec!["tool:edit".to_owned()],
        ..request(&fixture, HostSurface::Headless)
    };
    let smith = factory::build_request(request).await.expect("a runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("edit the Rust file"))
        .await
        .expect("the turn runs");
    let rows = smith_runtime::capability_limits::catalog(&session, &["tool:edit".to_owned()]);
    session.shutdown().await.expect("a clean shutdown");

    let names = provider.requests()[0]
        .tools
        .iter()
        .map(|schema| schema.name.clone())
        .collect::<Vec<_>>();
    assert!(!names.iter().any(|name| name == "edit"), "{names:?}");
    assert!(names.iter().any(|name| name == "shell"), "{names:?}");
    let standing = |id: &str| rows.iter().find(|row| row.id == id).map(|row| row.standing);
    assert_eq!(
        standing("tool:edit"),
        Some(smith_runtime::capability_limits::CapabilityStanding::DeniedBySession)
    );
    assert_eq!(
        standing("tool:shell"),
        Some(smith_runtime::capability_limits::CapabilityStanding::Active)
    );
    assert!(
        rows.iter().all(|row| !row.id.starts_with("tool:registry.")),
        "the discovery bootstraps are not listed as capabilities"
    );
}

#[tokio::test]
async fn the_agent_browses_and_activates_a_capability_by_id() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let mut browse = tool_call_fragments(0, "browse-1", "registry.search", r#"{"domain":"tool"}"#);
    browse.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let mut activate = tool_call_fragments(
        0,
        "activate-1",
        "registry.activate",
        r#"{"ids":["tool:agent","tool:no-such-tool"]}"#,
    );
    activate.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(browse),
            ScriptedStream::new(activate),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "ready".to_owned(),
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
        .run(UserInput::text("hi"))
        .await
        .expect("the discovery loop completes");
    session.shutdown().await.expect("a clean shutdown");

    let requests = provider.requests();
    assert_eq!(requests.len(), 3, "{requests:?}");
    let tools = |index: usize| {
        requests[index]
            .tools
            .iter()
            .map(|schema| schema.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(tools(0), tools(1), "browsing activates nothing");
    assert!(
        !tools(1).iter().any(|name| name == "agent"),
        "{:?}",
        tools(1)
    );
    assert!(
        tools(2).iter().any(|name| name == "agent"),
        "the named capability missed the next request: {:?}",
        tools(2)
    );
    let transcript = format!("{:?}", requests[2].messages);
    assert!(transcript.contains("listing"), "{transcript}");
    assert!(transcript.contains("tool:agent"), "{transcript}");
    assert!(
        transcript.contains("unknown or not authorized"),
        "an unknown id is rejected without failing the call: {transcript}"
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
