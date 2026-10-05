use super::*;

#[test]
fn live_and_history_replay_render_the_same_changed_rows() {
    let cases = [
        (
            "shell",
            serde_json::json!({"command": "ls -la"}),
            (1..=20)
                .map(|line| format!("line {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        (
            "read",
            serde_json::json!({"path": "src/retry.rs"}),
            "1  first\n2  second\n3  third".to_owned(),
        ),
        (
            "edit",
            serde_json::json!({"path": "src/retry.rs", "old_string": "before", "new_string": "one\ntwo\nthree\nfour"}),
            "edited `src/retry.rs` (1 replacement(s))".to_owned(),
        ),
        (
            "search",
            serde_json::json!({"pattern": "needle", "path": "src"}),
            "src/retry.rs:42\nsrc/retry.rs:43".to_owned(),
        ),
        (
            "list",
            serde_json::json!({"path": "src"}),
            "retry.rs\nmain.rs".to_owned(),
        ),
        (
            "advisor",
            serde_json::json!({}),
            "Cover cancellation too.".to_owned(),
        ),
        (
            "third_party",
            serde_json::json!({"SECRET_ARGUMENT_NAME": "SECRET_VALUE"}),
            "reviewed result".to_owned(),
        ),
        (
            "write_todos",
            serde_json::json!({"items": []}),
            "recorded".to_owned(),
        ),
        (
            "agent",
            serde_json::json!({"action": "wait", "child_id": "child-1"}),
            "ready".to_owned(),
        ),
        (
            "agent",
            serde_json::json!({"action": "spawn", "task": "review the retry policy"}),
            "child started".to_owned(),
        ),
    ];
    for (name, arguments, output) in cases {
        for is_error in [false, true] {
            let result = if is_error {
                "permission denied"
            } else {
                &output
            };
            let user = "Inspect the retry path and explain how cancellation behaves after repeated provider failures.";
            let answer =
                "The retry policy keeps cancellation responsive while it waits for the provider.";
            let history = [
                Message::user(user),
                Message::assistant(vec![
                    ContentPart::Text {
                        text: answer.to_owned(),
                    },
                    ContentPart::ToolCall(ToolCall {
                        id: ToolCallId::new("call"),
                        name: name.to_owned(),
                        arguments: arguments.clone(),
                    }),
                ]),
                Message::tool_result(ToolResultBlock {
                    call_id: ToolCallId::new("call"),
                    name: name.to_owned(),
                    content: vec![ContentPart::text(result)],
                    is_error,
                }),
            ];
            let mut live = App::new("m", "p");
            live.transcript.push_user(user);
            live.transcript.push_text_delta(answer);
            let keys = arguments
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            live.apply(&event(RuntimeEvent::ToolCallRequested {
                call: ToolCallId::new("call"),
                name: name.to_owned(),
                argument_keys: keys,
                argument_fingerprint: agent_runtime_registry::Fingerprint::of("protected"),
                arguments: None,
            }));
            live.apply(&event(RuntimeEvent::ToolCallCompleted {
                call: ToolCallId::new("call"),
                name: name.to_owned(),
                is_error,
            }));
            let mut resumed = App::new("m", "p");
            resumed.transcript.replace_from_history(&history);
            for app in [&mut live, &mut resumed] {
                if let Some(display) = smith_tools::project_tool_call_display(name, &arguments) {
                    app.set_tool_display("call", display);
                }
                app.set_tool_result_preview("call", result);
            }
            for expanded in [false, true] {
                live.work_details = expanded;
                resumed.work_details = expanded;
                for width in [44, 80, 100] {
                    for theme in [Theme::new(), Theme::new().without_color()] {
                        let lines = transcript_lines(&live, theme, width);
                        assert_eq!(
                            lines,
                            transcript_lines(&resumed, theme, width),
                            "{name}, error={is_error}, expanded={expanded}, width={width}"
                        );
                        let text = lines
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("\n");
                        assert!(
                            !text.contains("SECRET_ARGUMENT_NAME")
                                && !text.contains("SECRET_VALUE"),
                            "{text}"
                        );
                        assert!(!text.contains("details unavailable"), "{text}");
                    }
                }
            }
        }
    }

    for (call, is_error, output) in [
        (
            Some("shortcut-call"),
            false,
            (1..=20)
                .map(|line| format!(" M crates/smith-cli/src/a_long_file_name_{line}.rs"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        (Some("failed-call"), true, "command failed".to_owned()),
        (
            None,
            true,
            "shell action failed: requires an idle session".to_owned(),
        ),
        (
            Some("denied-call"),
            true,
            "approval declined: the user declined".to_owned(),
        ),
        (Some("quiet-call"), false, String::new()),
    ] {
        let history = [
            Message::user("Inspect the worktree"),
            Message::assistant(vec![ContentPart::text("Later answer")]),
        ];
        let mut live = App::new("m", "p");
        live.transcript.push_user("Inspect the worktree");
        let echo = live.transcript.push_shell_shortcut("git status --short");
        let result = live
            .transcript
            .finish_shell_shortcut(echo, call, is_error, &output);
        live.transcript.push_text_delta("Later answer");
        live.transcript.close_open();
        let mut resumed = App::new("m", "p");
        resumed
            .transcript
            .replace_from_history_with_shell_shortcuts(
                &history,
                &[crate::transcript::RestoredShellShortcut {
                    anchor: 1,
                    call: call.map(str::to_owned),
                    command: "git status --short".to_owned(),
                    is_error,
                    result,
                }],
            );
        for expanded in [false, true] {
            assert_eq!(live.work_details, expanded);
            assert_eq!(resumed.work_details, expanded);
            for width in [44, 80, 100] {
                for theme in [Theme::new(), Theme::new().without_color()] {
                    let lines = transcript_lines(&live, theme, width);
                    assert_eq!(
                        lines,
                        transcript_lines(&resumed, theme, width),
                        "shortcut {call:?}, expanded={expanded}, width={width}"
                    );
                    let text = lines
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("\n");
                    assert_eq!(text.matches("! git status --short").count(), 1, "{text}");
                    if call == Some("shortcut-call") {
                        assert_eq!(text.contains("ctrl+o to expand"), !expanded, "{text}");
                    }
                }
            }
            for app in [&mut live, &mut resumed] {
                app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
            }
        }
    }
}

#[test]
fn journal_replay_keeps_external_tool_rows_and_results_identical() {
    for ok in [true, false] {
        let output = (1..=8)
            .map(|line| format!("agent output {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let events = [
            event(RuntimeEvent::ExternalToolInvoked {
                id: "external-bash".to_owned(),
                name: "Bash".to_owned(),
                detail: serde_json::json!({"command": "echo hello"}),
            }),
            event(RuntimeEvent::ExternalToolCompleted {
                id: "external-bash".to_owned(),
                ok,
                detail: serde_json::Value::String(output),
            }),
        ];
        let bytes = serde_json::to_vec(&events).expect("journal events");
        let replayed: Vec<EventEnvelope> =
            serde_json::from_slice(&bytes).expect("replayable events");
        let mut live = App::new("m", "p");
        let mut replay = App::new("m", "p");
        for event in &events {
            live.apply(event);
        }
        for event in &replayed {
            replay.apply(event);
        }
        assert_eq!(live.transcript.blocks(), replay.transcript.blocks());
        for expanded in [false, true] {
            live.work_details = expanded;
            replay.work_details = expanded;
            for width in [44, 80, 100] {
                for theme in [Theme::new(), Theme::new().without_color()] {
                    assert_eq!(
                        transcript_lines(&live, theme, width),
                        transcript_lines(&replay, theme, width),
                        "ok={ok}, expanded={expanded}, width={width}"
                    );
                }
            }
        }
    }
}

#[test]
fn shell_admission_errors_are_nested_under_the_exact_user_echo() {
    let mut app = App::new("m", "p");
    let echo = app.transcript.push_shell_shortcut("ls -la");
    let _ = app.transcript.finish_shell_shortcut(
        echo,
        None,
        true,
        "shell action failed: requires an idle session",
    );
    let text = transcript_lines(&app, Theme::new().without_color(), 100)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert_eq!(
        text,
        [
            "! ls -la failed",
            "  ⎿  shell action failed: requires an idle session"
        ]
    );
}
