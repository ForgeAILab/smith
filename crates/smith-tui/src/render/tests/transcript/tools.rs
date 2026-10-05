use super::*;

#[test]
fn a_tool_row_states_non_success_in_words_not_only_color() {
    for status in [
        ToolStatus::Failed,
        ToolStatus::Denied,
        ToolStatus::Unreported,
    ] {
        let mut app = conversation();
        app.transcript.complete_tool_call("c1", status);
        let screen = render(&app, 74, 16, Theme::new().without_color());
        assert!(
            screen.contains(&format!("● Read(src/retry.rs) {}", status.label())),
            "{screen}"
        );
    }
}

#[test]
fn compact_tool_rows_show_redacted_details_without_results_or_unknown_values() {
    let call_id = ToolCallId::new("search-1");
    let history = vec![
        Message::assistant(vec![ContentPart::ToolCall(ToolCall {
            id: call_id.clone(),
            name: "search".to_owned(),
            arguments: serde_json::json!({
                "pattern": "TOP_SECRET_PATTERN",
                "path": "src/\n\u{1b}[31m\u{202e}tests",
                "unknown": "TOP_SECRET_UNKNOWN"
            }),
        })]),
        Message::tool_result(ToolResultBlock {
            call_id,
            name: "search".to_owned(),
            content: vec![ContentPart::text("TOP_SECRET_RESULT")],
            is_error: false,
        }),
    ];
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.transcript.replace_from_history(&history);
    app.set_tool_display(
        "search-1",
        smith_tools::project_tool_call_display(
            "search",
            &serde_json::json!({
                "pattern": "[redacted]",
                "path": "src/\n\u{1b}[31m\u{202e}tests"
            }),
        )
        .expect("reviewed search projection"),
    );
    app.transcript
        .push_tool_call("unknown-1", "third_party", None, &["path".to_owned()]);
    app.transcript
        .complete_tool_call("unknown-1", ToolStatus::Failed);

    let screen = render(&app, 74, 16, Theme::new().without_color());
    assert!(
        screen.contains("● Search(\"[redacted]\" · src/ [31m tests)"),
        "{screen}"
    );
    assert!(
        screen.contains("● third_party(arguments hidden) failed"),
        "{screen}"
    );
    assert!(!screen.contains("TOP_SECRET_PATTERN"), "{screen}");
    assert!(!screen.contains("TOP_SECRET_UNKNOWN"), "{screen}");
    assert!(!screen.contains("TOP_SECRET_RESULT"), "{screen}");
    assert!(!screen.contains('\u{1b}'), "{screen:?}");
    assert!(!screen.contains('\u{202e}'), "{screen:?}");
    assert!(screen.contains("⎿  Completed"), "{screen}");
}

#[test]
fn running_tool_call_displays_elapsed_time() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("c1"),
        name: "shell".to_owned(),
        argument_keys: vec!["command".to_owned(), "cwd".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: Some(serde_json::json!({
            "command": "cargo test",
            "cwd": "."
        })),
    }));

    let screen = render(&app, 74, 16, Theme::new().without_color());
    assert!(screen.contains("● Bash(cargo test) running 0s"), "{screen}");
}

#[tokio::test]
async fn approval_waiting_is_rendered_in_the_tool_and_working_rows() {
    let mut app = App::new("gpt-5.3", "/repo");
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("approval-evidence"),
        name: "shell".to_owned(),
        argument_keys: vec!["command".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: Some(serde_json::json!({"command": "git status --short"})),
    }));
    app.present_approval(approval_evidence_prompt("git status --short", false).await);
    app.work_details = true;
    let lines = transcript_lines(&app, Theme::new().without_color(), 100);
    let tool = lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        tool.contains("● Bash(git status --short) waiting for approval"),
        "{tool}"
    );
    assert!(!tool.contains("running"), "{tool}");
    let work = working_line(&app, Theme::new().without_color(), 140).to_string();
    assert!(work.contains("tool shell · waiting for approval"), "{work}");
    assert!(!work.contains("running"), "{work}");
}

#[test]
fn denied_tool_result_renders_the_same_live_from_history_and_from_events() {
    let arguments = serde_json::json!({"command": "git status --short"});
    let reason = "approval declined: the user declined";
    let requested = event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("c1"),
        name: "shell".to_owned(),
        argument_keys: vec!["command".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    });
    let completed = event(RuntimeEvent::ToolCallCompleted {
        call: ToolCallId::new("c1"),
        name: "shell".to_owned(),
        is_error: true,
    });
    let history = vec![
        Message::assistant(vec![ContentPart::ToolCall(ToolCall {
            id: ToolCallId::new("c1"),
            name: "shell".to_owned(),
            arguments: arguments.clone(),
        })]),
        Message::tool_result(ToolResultBlock {
            call_id: ToolCallId::new("c1"),
            name: "shell".to_owned(),
            content: vec![ContentPart::text(reason)],
            is_error: true,
        }),
    ];
    let mut live = App::new("m", "p");
    live.apply(&requested);
    // The user's decision reaches the row before the completion event.
    live.transcript.complete_tool_call("c1", ToolStatus::Denied);
    live.set_tool_result_preview("c1", reason);
    live.apply(&completed);
    let mut history_replay = App::new("m", "p");
    history_replay.transcript.replace_from_history(&history);
    assert_eq!(
        history_replay.transcript.tool_status("c1"),
        Some(ToolStatus::Denied)
    );
    let mut event_replay = App::new("m", "p");
    event_replay.apply_recovered(&requested);
    event_replay.apply_recovered(&completed);
    for app in [&mut live, &mut history_replay, &mut event_replay] {
        app.set_tool_display(
            "c1",
            smith_tools::project_tool_call_display("shell", &arguments).expect("shell display"),
        );
        app.set_tool_result_preview("c1", reason);
        assert_eq!(app.transcript.tool_status("c1"), Some(ToolStatus::Denied));
    }
    for expanded in [false, true] {
        live.work_details = expanded;
        history_replay.work_details = expanded;
        event_replay.work_details = expanded;
        for width in [100, 80, 44] {
            let screen = render(&live, width, 16, Theme::new().without_color());
            assert!(
                screen.contains("● Bash(git status --short) denied"),
                "{screen}"
            );
            assert!(
                screen.contains("⎿  approval declined: the user declined"),
                "{screen}"
            );
            assert!(
                !screen.contains("failed") && !screen.contains("● approval"),
                "{screen}"
            );
            assert_eq!(
                screen,
                render(&history_replay, width, 16, Theme::new().without_color())
            );
            assert_eq!(
                screen,
                render(&event_replay, width, 16, Theme::new().without_color())
            );
        }
    }
}

#[test]
fn generate_image_rows_show_progress_saved_path_and_provider_errors() {
    let theme = Theme::new().without_color().without_motion();
    let success_args = serde_json::json!({
        "prompt": "A quiet lake at sunrise",
        "reference_paths": ["assets/shore.png"]
    });
    let mut success = App::new("gpt-5.3", "~/work/api");
    success.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("image-success"),
        name: "generate_image".to_owned(),
        argument_keys: vec!["prompt".to_owned(), "reference_paths".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("image arguments"),
        arguments: Some(success_args.clone()),
    }));
    let display = smith_tools::project_tool_call_display("generate_image", &success_args)
        .expect("reviewed generate_image projection");
    success.set_tool_display("image-success", display.clone());

    let running = render(&success, 100, 20, theme);
    assert!(running.contains("Generate Image("), "{running}");
    assert!(running.contains("A quiet lake at sunrise"), "{running}");
    assert!(running.contains("1 reference"), "{running}");
    assert!(running.contains(" running 0s"), "{running}");

    success.apply(&event(RuntimeEvent::ToolCallCompleted {
        call: ToolCallId::new("image-success"),
        name: "generate_image".to_owned(),
        is_error: false,
    }));
    // Completion re-projects from canonical arguments, as the host does.
    success.set_tool_display("image-success", display);
    success.set_tool_result_preview(
        "image-success",
        "Saved ~/.smith/generated_images/session/image-success.png (1024x1024)",
    );

    let completed = render(&success, 100, 20, theme);
    assert!(completed.contains("Generate Image("), "{completed}");
    assert!(
        !completed.contains("running") && !completed.contains(" · ok"),
        "{completed}"
    );
    assert!(
        completed.contains("Saved ~/.smith/generated_images/session/image-success.png (1024x1024)"),
        "{completed}"
    );

    let failure_args = serde_json::json!({"prompt": "Watercolor meadow"});
    let mut failure = App::new("gpt-5.3", "~/work/api");
    failure.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("image-failure"),
        name: "generate_image".to_owned(),
        argument_keys: vec!["prompt".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("image arguments"),
        arguments: Some(failure_args.clone()),
    }));
    failure.set_tool_display(
        "image-failure",
        smith_tools::project_tool_call_display("generate_image", &failure_args)
            .expect("reviewed generate_image projection"),
    );
    failure.apply(&event(RuntimeEvent::ToolCallCompleted {
        call: ToolCallId::new("image-failure"),
        name: "generate_image".to_owned(),
        is_error: true,
    }));
    failure.set_tool_result_preview("image-failure", "Image provider error: request timed out");

    let failed = render(&failure, 100, 20, theme);
    assert!(failed.contains("Generate Image("), "{failed}");
    assert!(failed.contains(" failed"), "{failed}");
    assert!(
        failed.contains("Image provider error: request timed out"),
        "{failed}"
    );
}

#[test]
fn two_servers_same_tool_name_render_distinguishably() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    for (call, name) in [
        ("docs-1", "mcp__docs__search"),
        ("wiki-1", "mcp__wiki__search"),
    ] {
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new(call),
            name: name.to_owned(),
            argument_keys: vec!["query".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        app.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new(call),
            name: name.to_owned(),
            is_error: false,
        }));
    }

    let screen = render(&app, 100, 20, Theme::new().without_color());
    assert!(
        screen.contains("mcp__docs__search") && screen.contains("mcp__wiki__search"),
        "each row names the server it belongs to, so the same tool name on two \
         servers cannot be confused: {screen}"
    );
}

#[test]
fn a_remote_tool_row_hides_its_arguments_and_says_so() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("remote-1"),
        name: "mcp__docs__search".to_owned(),
        argument_keys: vec!["query".to_owned(), "api_token".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));

    let screen = render(&app, 100, 20, Theme::new().without_color());
    assert!(screen.contains("mcp__docs__search"), "{screen}");
    assert!(
        screen.contains("arguments hidden"),
        "the withholding is stated rather than left to be inferred: {screen}"
    );
}

#[test]
fn resumed_remote_tool_row_matches_the_live_row() {
    // A server names its own argument fields, so Smith cannot tell which
    // of them carry secrets — and must not render one from persisted
    // history either.
    let history = vec![
        Message::assistant(vec![ContentPart::ToolCall(ToolCall {
            id: ToolCallId::new("remote-1"),
            name: "mcp__docs__search".to_owned(),
            arguments: serde_json::json!({
                "query": "boundaries",
                "api_token": "value-that-must-not-render"
            }),
        })]),
        Message::tool_result(ToolResultBlock {
            call_id: ToolCallId::new("remote-1"),
            name: "mcp__docs__search".to_owned(),
            content: vec![ContentPart::text("ok")],
            is_error: false,
        }),
    ];
    let mut resumed = App::new("gpt-5.3", "~/work/api");
    resumed.transcript.replace_from_history(&history);

    let mut live = App::new("gpt-5.3", "~/work/api");
    live.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("remote-1"),
        name: "mcp__docs__search".to_owned(),
        argument_keys: vec!["api_token".to_owned(), "query".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));
    live.apply(&event(RuntimeEvent::ToolCallCompleted {
        call: ToolCallId::new("remote-1"),
        name: "mcp__docs__search".to_owned(),
        is_error: false,
    }));

    let live_screen = render(&live, 100, 20, Theme::new().without_color());
    let resumed_screen = render(&resumed, 100, 20, Theme::new().without_color());
    for screen in [&live_screen, &resumed_screen] {
        assert!(screen.contains("mcp__docs__search"), "{screen}");
        assert!(screen.contains("arguments hidden"), "{screen}");
        assert!(!screen.contains("value-that-must-not-render"), "{screen}");
    }
}

#[test]
fn one_tool_row_nests_four_lines_and_expands_all_detail() {
    for width in [44, 80, 100] {
        let mut app = App::new("m", "p");
        app.transcript.push_tool_call(
            "bash-1",
            "shell",
            Some(&serde_json::json!({"command": "ls -la"})),
            &[],
        );
        app.transcript.complete_tool_call("bash-1", ToolStatus::Ok);
        app.set_tool_result_preview(
            "bash-1",
            (1..=20)
                .map(|line| format!("line {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let folded = transcript_lines(&app, Theme::new(), width);
        let text = folded.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(
            text,
            [
                "● Bash(ls -la)",
                "  ⎿  line 1",
                "     line 2",
                "     line 3",
                "     line 4",
                "     … +16 lines (ctrl+o to expand)"
            ]
        );
        assert_eq!(folded[0].spans[0].style.fg, Some(Color::Green));
        app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        let expanded = transcript_lines(&app, Theme::new(), width);
        assert_eq!(expanded.len(), 21);
        assert_eq!(expanded.last().unwrap().to_string(), "     line 20");
        assert_eq!(
            expanded
                .iter()
                .filter(|line| line.to_string().contains("Bash("))
                .count(),
            1
        );
        app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(transcript_lines(&app, Theme::new(), width), folded);
    }
}

#[test]
fn running_failed_and_denied_tool_markers_keep_their_status() {
    for (status, color, word) in [
        (ToolStatus::WaitingForApproval, None, "waiting for approval"),
        (ToolStatus::Running, None, "running"),
        (ToolStatus::Failed, Some(Color::Red), "failed"),
        (ToolStatus::Denied, Some(Color::Red), "denied"),
    ] {
        let mut app = App::new("m", "p");
        app.transcript.push_tool_call(
            "c",
            "shell",
            None,
            &["command".into(), "cwd".into(), "timeout_ms".into()],
        );
        if status != ToolStatus::Running {
            app.transcript.complete_tool_call("c", status);
        }
        let lines = transcript_lines(&app, Theme::new(), 100);
        assert_eq!(lines[0].spans[0].style.fg, color);
        if matches!(status, ToolStatus::Running | ToolStatus::WaitingForApproval) {
            assert!(lines[0].spans[0].style.add_modifier.contains(Modifier::DIM));
        }
        let text = lines[0].to_string();
        assert!(text.starts_with("● Bash(arguments hidden)"), "{text}");
        assert!(text.contains(word), "{text}");
        for protected in ["command", "cwd", "timeout_ms", "details unavailable"] {
            assert!(!text.contains(protected), "{text}");
        }
        assert!(
            transcript_lines(&app, Theme::new().without_color(), 100)[0]
                .to_string()
                .contains(word)
        );
    }
}

#[test]
fn reads_and_updates_have_one_line_summaries_with_expandable_detail() {
    let mut app = App::new("m", "p");
    app.transcript.push_tool_call(
        "read",
        "read",
        Some(&serde_json::json!({"path": "src/retry.rs"})),
        &[],
    );
    app.transcript.complete_tool_call("read", ToolStatus::Ok);
    app.set_tool_result_preview("read", "1  first\n2  second\n3  third");
    app.transcript.push_tool_call("edit", "edit", Some(&serde_json::json!({"path": "src/retry.rs", "old_string": "before", "new_string": "one\ntwo\nthree\nfour"})), &[]);
    app.transcript.complete_tool_call("edit", ToolStatus::Ok);
    app.set_tool_result_preview("edit", "edited `src/retry.rs` (1 replacement(s))");
    let folded = transcript_lines(&app, Theme::new(), 100)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert_eq!(
        folded,
        [
            "● Read(src/retry.rs)",
            "  ⎿  Read 3 lines",
            "",
            "● Update(src/retry.rs)",
            "  ⎿  Updated src/retry.rs with 4 additions and 1 removal"
        ]
    );
    app.toggle_work_details();
    let expanded = transcript_lines(&app, Theme::new(), 100)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert!(expanded.iter().any(|line| line == "  ⎿  1  first"));
    assert!(expanded.iter().any(|line| line == "     3  third"));
}

#[test]
fn user_shell_echo_and_runtime_call_are_one_row_with_a_nested_result() {
    let mut app = App::new("m", "p");
    app.composer.replace("!ls -la");
    assert!(matches!(
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Some(crate::app::Action::RunShell { .. })
    ));
    let echo = app.transcript.latest_shell_echo().unwrap();
    app.track_shell_shortcut(TurnId::new("shell-turn"), echo);
    let mut requested = event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("shell-call"),
        name: "shell".into(),
        argument_keys: vec!["command".into(), "cwd".into()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("protected"),
        arguments: None,
    });
    requested.turn = Some(TurnId::new("shell-turn"));
    app.apply(&requested);
    assert_eq!(app.transcript.blocks().len(), 1);
    app.transcript.bind_shell_shortcut(echo, "shell-call");
    app.transcript
        .complete_tool_call("shell-call", ToolStatus::Ok);
    app.set_tool_result_preview("shell-call", "total 8\nfile one\nfile two");
    for width in [44, 80, 100] {
        let text = transcript_lines(&app, Theme::new().without_color(), width)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(
            text,
            ["! ls -la", "  ⎿  total 8", "     file one", "     file two"]
        );
        assert!(!text.concat().contains("Bash("));
        assert!(!text.concat().contains("/shell"));
        assert!(!text.concat().contains("changes"));
    }
}
