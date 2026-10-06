use super::*;

// -- Reviewed redundant-row suppression (tool-call-display group 2) ----

#[test]
fn successful_write_todos_and_registry_search_rows_are_suppressed_without_a_blank_line_artifact() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.transcript.push_user("plan the work");
    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("todos-1"),
        name: "write_todos".to_owned(),
        argument_keys: vec!["items".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));
    app.apply(&event(RuntimeEvent::ToolCallCompleted {
        call: ToolCallId::new("todos-1"),
        name: "write_todos".to_owned(),
        is_error: false,
    }));
    app.transcript
        .set_tool_result_preview("todos-1", "5 items recorded");
    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("search-1"),
        name: "registry.search".to_owned(),
        argument_keys: vec!["query".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));
    app.set_tool_display(
        "search-1",
        smith_tools::project_tool_call_display(
            "registry.search",
            &serde_json::json!({"query": "browser automation"}),
        )
        .expect("reviewed registry.search projection"),
    );
    app.apply(&event(RuntimeEvent::ToolCallCompleted {
        call: ToolCallId::new("search-1"),
        name: "registry.search".to_owned(),
        is_error: false,
    }));
    app.transcript
        .set_tool_result_preview("search-1", "browser-tool card");
    app.transcript.push_text_delta("Plan set.");
    app.transcript.close_open();

    let lines = transcript_lines(&app, Theme::new(), 74);
    let texts = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert!(
        !texts.iter().any(|text| text.contains("write_todos")
            || text.contains("Registry Search")
            || text.contains("5 items recorded")
            || text.contains("browser-tool card")),
        "both rows and their previews are suppressed: {texts:#?}"
    );
    let user_index = texts
        .iter()
        .position(|text| text.contains("plan the work"))
        .expect("the user row survives");
    let reply_index = texts
        .iter()
        .position(|text| text.contains("Plan set."))
        .expect("the assistant reply survives");
    // Exactly one blank separator between the two surviving blocks: no
    // doubled or leading blank line was left behind by suppressing the
    // two tool rows between them.
    assert_eq!(
        reply_index,
        user_index + 2,
        "a suppression must not leave a blank-line artifact: {texts:#?}"
    );
    assert_eq!(texts[user_index + 1], "", "{texts:#?}");
}

#[test]
fn a_running_registry_search_row_never_flashes_before_its_suppression() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.transcript.push_user("find a tool");
    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("search-1"),
        name: "registry.search".to_owned(),
        argument_keys: vec!["query".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));

    let texts = transcript_lines(&app, Theme::new(), 74)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert!(
        !texts.iter().any(|text| text.contains("Registry Search")),
        "the row would vanish on completion, so it is not drawn while running: {texts:#?}"
    );
}

#[test]
fn a_failed_denied_or_unreported_suppressed_call_still_renders() {
    let mut app = App::new("gpt-5.3", "~/work/api");

    // Failed: `write_todos` has no reviewed schema, so it renders on its
    // honest fallback, but it renders.
    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("todos-failed"),
        name: "write_todos".to_owned(),
        argument_keys: vec!["items".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));
    app.apply(&event(RuntimeEvent::ToolCallCompleted {
        call: ToolCallId::new("todos-failed"),
        name: "write_todos".to_owned(),
        is_error: true,
    }));

    // Denied: matched by tool name, exactly as a real approval denial
    // resolves it.
    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("search-denied"),
        name: "registry.search".to_owned(),
        argument_keys: vec!["query".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));
    app.set_tool_display(
        "search-denied",
        smith_tools::project_tool_call_display(
            "registry.search",
            &serde_json::json!({"query": "browser automation"}),
        )
        .expect("reviewed registry.search projection"),
    );
    app.transcript
        .complete_tool_call_by_name("registry.search", ToolStatus::Denied);

    // Unreported: the conversation ended mid-call.
    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("wait-unreported"),
        name: "agent".to_owned(),
        argument_keys: vec!["action".into(), "child_id".into()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));
    app.set_tool_display(
        "wait-unreported",
        smith_tools::project_tool_call_display(
            "agent",
            &serde_json::json!({"action": "wait", "child_id": "child-1"}),
        )
        .expect("reviewed wait projection"),
    );
    app.transcript
        .settle_running_tool_calls(ToolStatus::Unreported);

    let screen = render(&app, 100, 20, Theme::new().without_color());
    assert!(
        screen.contains("write_todos(arguments hidden)") && screen.contains("failed"),
        "a failed suppressed call still renders: {screen}"
    );
    assert!(
        screen.contains("Registry Search(\"browser automation\")") && screen.contains("denied"),
        "a denied suppressed call still renders: {screen}"
    );
    assert!(
        screen.contains("Agent(wait · child-1)") && screen.contains(ToolStatus::Unreported.label()),
        "an unreported suppressed call still renders: {screen}"
    );
}

#[test]
fn suppression_matches_between_the_live_and_resumed_paths() {
    let history = vec![
        Message::assistant(vec![ContentPart::ToolCall(ToolCall {
            id: ToolCallId::new("todos-1"),
            name: "write_todos".to_owned(),
            arguments: serde_json::json!({"items": []}),
        })]),
        Message::tool_result(ToolResultBlock {
            call_id: ToolCallId::new("todos-1"),
            name: "write_todos".to_owned(),
            content: vec![ContentPart::text("ok")],
            is_error: false,
        }),
    ];
    let mut resumed = App::new("gpt-5.3", "~/work/api");
    resumed.transcript.replace_from_history(&history);

    let mut live = App::new("gpt-5.3", "~/work/api");
    live.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("todos-1"),
        name: "write_todos".to_owned(),
        argument_keys: vec!["items".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));
    live.apply(&event(RuntimeEvent::ToolCallCompleted {
        call: ToolCallId::new("todos-1"),
        name: "write_todos".to_owned(),
        is_error: false,
    }));

    let live_screen = render(&live, 74, 16, Theme::new().without_color());
    let resumed_screen = render(&resumed, 74, 16, Theme::new().without_color());
    assert!(!live_screen.contains("write_todos"), "{live_screen}");
    assert!(!resumed_screen.contains("write_todos"), "{resumed_screen}");
}

#[test]
fn agent_follow_up_and_list_rows_are_never_suppressed() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("follow-1"),
        name: "agent".to_owned(),
        argument_keys: vec!["action".into(), "child_id".into(), "task".into()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));
    app.set_tool_display(
        "follow-1",
        smith_tools::project_tool_call_display(
            "agent",
            &serde_json::json!({
                "action": "follow_up",
                "child_id": "child-1",
                "task": "keep going"
            }),
        )
        .expect("reviewed follow_up projection"),
    );
    app.apply(&event(RuntimeEvent::ToolCallCompleted {
        call: ToolCallId::new("follow-1"),
        name: "agent".to_owned(),
        is_error: false,
    }));

    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("list-1"),
        name: "agent".to_owned(),
        argument_keys: vec!["action".into()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));
    app.set_tool_display(
        "list-1",
        smith_tools::project_tool_call_display("agent", &serde_json::json!({"action": "list"}))
            .expect("reviewed list projection"),
    );
    app.apply(&event(RuntimeEvent::ToolCallCompleted {
        call: ToolCallId::new("list-1"),
        name: "agent".to_owned(),
        is_error: false,
    }));

    let screen = render(&app, 74, 16, Theme::new().without_color());
    assert!(
        screen.contains("Agent(follow_up · child-1 · \"keep going\")"),
        "{screen}"
    );
    assert!(screen.contains("Agent(list)"), "{screen}");
}
