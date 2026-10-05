use super::*;
use agent_runtime_core::content::{ToolCall, ToolResultBlock};
use agent_runtime_core::ids::ToolCallId;
use serde_json::json;

#[test]
fn streamed_deltas_accumulate_into_one_block() {
    let mut transcript = Transcript::new();
    for delta in ["The ", "retry ", "policy"] {
        transcript.push_text_delta(delta);
    }
    assert_eq!(transcript.len(), 1);
    assert_eq!(
        transcript.blocks()[0],
        Block::Assistant {
            text: "The retry policy".into(),
            open: true
        }
    );
}

#[test]
fn a_closed_block_does_not_absorb_the_next_reply() {
    let mut transcript = Transcript::new();
    transcript.push_text_delta("first");
    transcript.close_open();
    transcript.push_text_delta("second");
    assert_eq!(transcript.len(), 2);
}

#[test]
fn a_feedback_kind_cannot_record_a_block_or_split_a_streaming_reply() {
    let mut transcript = Transcript::new();
    transcript.push_text_delta("still");
    transcript.push_notice(NoticeKind::Clipboard, "nothing to attach");
    transcript.push_text_delta(" streaming");
    assert!(
        matches!(transcript.blocks(), [Block::Assistant { text, open: true }]
        if text == "still streaming")
    );
}

#[test]
fn a_notice_never_splices_into_a_streaming_reply() {
    let mut transcript = Transcript::new();
    transcript.push_text_delta("analyzing");
    transcript.push_notice(
        NoticeKind::NamedMonitor("build".to_owned()),
        "error[E0433]: failed to resolve",
    );
    transcript.push_text_delta(" the failure");

    // The notice stands alone, and the reply resumes in a fresh block
    // rather than having the monitor line spliced into its text.
    assert_eq!(transcript.len(), 3);
    assert!(matches!(
        transcript.blocks()[0],
        Block::Assistant { open: false, .. }
    ));
    assert!(matches!(transcript.blocks()[1], Block::Notice { .. }));
    match &transcript.blocks()[2] {
        Block::Assistant { text, .. } => assert_eq!(text, " the failure"),
        other => panic!("expected an assistant block, got {other:?}"),
    }
}

#[test]
fn typed_agent_resume_notices_keep_the_existing_stream_boundary() {
    for resume in [
        AgentResumeReport::RequiresIdle,
        AgentResumeReport::Started {
            child: "child-1".to_owned(),
        },
    ] {
        let mut transcript = Transcript::new();
        transcript.push_text_delta("analyzing");
        transcript.push_local(LocalResult::Agent(Box::new(AgentReport::Resume(resume))));
        assert!(matches!(
            transcript.blocks()[0],
            Block::Assistant { open: true, .. }
        ));
        transcript.push_text_delta(" the failure");
        assert_eq!(transcript.len(), 3);
        assert!(matches!(
            transcript.blocks()[0],
            Block::Assistant { open: false, .. }
        ));
        assert!(matches!(
            &transcript.blocks()[2],
            Block::Assistant { text, .. } if text == " the failure"
        ));
    }
}

#[test]
fn typed_recovery_notices_keep_the_existing_stream_boundary() {
    use smith_client::recovery_report::{RecoveryAction, RecoveryApplied};

    for report in [
        RecoveryReport::Applied(RecoveryApplied::Undo),
        RecoveryReport::Applied(RecoveryApplied::Redo),
        RecoveryReport::Applied(RecoveryApplied::Revert {
            scope: "path#1".to_owned(),
        }),
        RecoveryReport::Cancelled(RecoveryAction::Undo),
        RecoveryReport::Cancelled(RecoveryAction::Redo),
        RecoveryReport::Cancelled(RecoveryAction::Revert),
    ] {
        let mut transcript = Transcript::new();
        transcript.push_text_delta("analyzing");
        transcript.push_local(LocalResult::Recovery(Box::new(report)));
        assert!(matches!(
            transcript.blocks()[0],
            Block::Assistant { open: true, .. }
        ));
        transcript.push_text_delta(" the failure");
        assert_eq!(transcript.len(), 3);
        assert!(matches!(
            transcript.blocks()[0],
            Block::Assistant { open: false, .. }
        ));
        assert!(
            matches!(&transcript.blocks()[2], Block::Assistant { text, .. } if text == " the failure")
        );
    }
}

#[test]
fn typed_review_notices_keep_the_existing_stream_boundary() {
    for report in [
        ReviewReport::Empty,
        ReviewReport::Start(ReviewStartReport::Started {
            child: "child-1".to_owned(),
        }),
        ReviewReport::Start(ReviewStartReport::Queued {
            child: "child-2".to_owned(),
        }),
    ] {
        let mut transcript = Transcript::new();
        transcript.push_text_delta("analyzing");
        transcript.push_local(LocalResult::Review(Box::new(report)));
        assert!(matches!(
            transcript.blocks()[0],
            Block::Assistant { open: true, .. }
        ));
        transcript.push_text_delta(" the failure");
        assert_eq!(transcript.len(), 3);
        assert!(matches!(
            transcript.blocks()[0],
            Block::Assistant { open: false, .. }
        ));
        assert!(matches!(
            &transcript.blocks()[2],
            Block::Assistant { text, .. } if text == " the failure"
        ));
    }
}

#[test]
fn local_results_are_bounded_and_never_merge_with_model_output() {
    let mut transcript = Transcript::new();
    transcript.push_text_delta("answer");
    transcript.push_local(LocalResult::Message(Box::new(MessageReport::Notice {
        title: "diff\ninjected".to_owned(),
        message: "x".repeat(MAX_LOCAL_RESULT_BYTES + 16),
    })));
    transcript.push_text_delta("next");

    assert_eq!(transcript.len(), 3);
    match &transcript.blocks()[1] {
        Block::Local(LocalResult::Message(report)) => {
            let MessageReport::Notice { title, message } = report.as_ref() else {
                panic!("expected an informational message");
            };
            assert_eq!(title, "diff injected");
            assert!(message.ends_with("[local result truncated at the display limit]"));
        }
        other => panic!("expected a local result, got {other:?}"),
    }
    assert!(matches!(
        transcript.blocks()[2],
        Block::Assistant { open: true, .. }
    ));
}

#[test]
fn shell_reports_are_bounded_without_merging_into_model_output() {
    use smith_client::shell_report::ShellReport;

    let mut transcript = Transcript::new();
    transcript.push_text_delta("answer");
    transcript.push_local(LocalResult::Shell(Box::new(ShellReport::new(
        "x".repeat(MAX_LOCAL_RESULT_BYTES + 16),
        false,
    ))));
    transcript.push_text_delta("next");

    assert_eq!(transcript.len(), 3);
    let Block::Local(LocalResult::Shell(report)) = &transcript.blocks()[1] else {
        panic!("expected a shell report");
    };
    let ShellOutput::Output(output) = &report.output else {
        panic!("expected command output");
    };
    assert!(!report.is_error);
    assert!(output.ends_with("[local result truncated at the display limit]"));
    assert!(matches!(
        transcript.blocks()[0],
        Block::Assistant { open: false, .. }
    ));
    assert!(matches!(
        transcript.blocks()[2],
        Block::Assistant { open: true, .. }
    ));
}

#[test]
fn typed_diff_limits_preserve_patch_kinds_and_model_stream_boundaries() {
    let cases = [
        vec![DiffLine {
            kind: DiffLineKind::Addition,
            text: format!("+{}終", "x".repeat(MAX_LOCAL_RESULT_BYTES - 2)),
        }],
        vec![
            DiffLine {
                kind: DiffLineKind::Removal,
                text: format!("-{}\n", "x".repeat(MAX_LOCAL_RESULT_BYTES - 2)),
            },
            DiffLine {
                kind: DiffLineKind::Addition,
                text: "+next\n".to_owned(),
            },
        ],
        (0..MAX_LOCAL_RESULT_LINES)
            .map(|_| DiffLine {
                kind: DiffLineKind::Metadata,
                text: "header\n".to_owned(),
            })
            .collect(),
    ];
    for patch in cases {
        let expected = bound_local_result(patch.iter().map(|line| line.text.as_str()).collect());
        let first_kind = patch[0].kind;
        let mut transcript = Transcript::new();
        transcript.push_text_delta("answer");
        transcript.push_local(LocalResult::Diff(Box::new(DiffReport {
            title: "diff\ninjected".to_owned(),
            outcome: DiffOutcome::Patch(patch),
        })));
        transcript.push_text_delta("next");

        let Block::Local(LocalResult::Diff(report)) = &transcript.blocks()[1] else {
            panic!("expected a typed diff report");
        };
        assert_eq!(report.title, "diff injected");
        assert_eq!(smith_client::diff_report::render_plain(report), expected);
        let DiffOutcome::Patch(patch) = &report.outcome else {
            panic!("expected classified patch lines");
        };
        assert_eq!(patch[0].kind, first_kind);
        assert_eq!(
            patch.last().expect("truncation note").kind,
            DiffLineKind::Context
        );
        assert!(matches!(
            transcript.blocks()[0],
            Block::Assistant { open: false, .. }
        ));
        assert!(matches!(
            transcript.blocks()[2],
            Block::Assistant { open: true, .. }
        ));
    }
}

#[test]
fn reasoning_and_text_occupy_separate_blocks() {
    let mut transcript = Transcript::new();
    transcript.push_reasoning_delta("considering", false);
    transcript.push_text_delta("answer");
    assert_eq!(transcript.len(), 2);
    assert!(matches!(transcript.blocks()[0], Block::Reasoning { .. }));
}

#[test]
fn redacted_reasoning_starts_a_new_block() {
    let mut transcript = Transcript::new();
    transcript.push_reasoning_delta("visible", false);
    transcript.push_reasoning_delta("hidden", true);
    assert_eq!(transcript.len(), 2);
}

#[test]
fn a_completion_updates_the_matching_tool_row() {
    let mut transcript = Transcript::new();
    transcript.push_tool_call(
        "c1",
        "read",
        Some(&json!({"path": "src/retry.rs"})),
        &["path".into()],
    );
    transcript.push_tool_call(
        "c2",
        "shell",
        Some(&json!({"command": "cargo test"})),
        &["command".into()],
    );
    transcript.complete_tool_call("c1", ToolStatus::Ok);
    transcript.complete_tool_call("c2", ToolStatus::Failed);

    match &transcript.blocks()[0] {
        Block::Tool {
            display, status, ..
        } => {
            assert_eq!(
                display.as_deref().map(ToolCallDisplay::invocation),
                Some("Read(src/retry.rs)".to_owned())
            );
            assert_eq!(*status, ToolStatus::Ok);
        }
        other => panic!("expected a tool block, got {other:?}"),
    }
    match &transcript.blocks()[1] {
        Block::Tool { status, .. } => assert_eq!(*status, ToolStatus::Failed),
        other => panic!("expected a tool block, got {other:?}"),
    }
}

#[test]
fn result_previews_are_bounded_sanitized_and_matched_by_id() {
    let mut transcript = Transcript::new();
    transcript.push_tool_call(
        "c1",
        "registry.search",
        Some(&json!({"query": "browser", "max_results": 2})),
        &["max_results".into(), "query".into()],
    );
    transcript.complete_tool_call("c1", ToolStatus::Ok);
    transcript.set_tool_result_preview(
        "c1",
        "\nfirst card\u{202e}\nsecond card\n\nthird card\nfourth card\nfifth card\n",
    );
    transcript.set_tool_result_preview("unknown", "never lands");
    transcript.set_tool_result_preview("c1", "   \n \n");

    match &transcript.blocks()[0] {
        Block::Tool {
            result_preview: Some(preview),
            ..
        } => {
            assert_eq!(
                preview,
                "\nfirst card\nsecond card\n\nthird card\nfourth card\nfifth card"
            );
        }
        other => panic!("expected a tool block with a preview, got {other:?}"),
    }
}

#[test]
fn completing_an_unknown_call_changes_nothing() {
    let mut transcript = Transcript::new();
    transcript.push_tool_call(
        "c1",
        "read",
        Some(&json!({"path": "a.rs"})),
        &["path".into()],
    );
    let before = transcript.blocks().to_vec();
    transcript.complete_tool_call("nonexistent", ToolStatus::Ok);
    assert_eq!(transcript.blocks(), before.as_slice());
}

#[test]
fn unavailable_details_show_keys_and_an_honest_reason() {
    let mut transcript = Transcript::new();
    transcript.push_tool_call("c1", "shell", None, &["command".into(), "cwd".into()]);

    match &transcript.blocks()[0] {
        Block::Tool {
            display,
            protected_summary,
            ..
        } => {
            assert!(display.is_none());
            assert_eq!(protected_summary, "arguments hidden");
            assert!(!protected_summary.contains("cargo test"));
        }
        other => panic!("expected a tool block, got {other:?}"),
    }
}

#[test]
fn credential_redacted_arguments_use_only_the_reviewed_projector() {
    let mut transcript = Transcript::new();
    transcript.push_tool_call(
        "c1",
        "shell",
        Some(&json!({
            "command": "printf [redacted]",
            "cwd": "crates/smith-cli",
            "unknown": "omitted"
        })),
        &["command".into(), "cwd".into(), "unknown".into()],
    );

    let Block::Tool { display, .. } = &transcript.blocks()[0] else {
        panic!("expected a tool block");
    };
    let invocation = display.as_ref().expect("safe projection").invocation();
    assert_eq!(invocation, "Bash(printf [redacted] · cwd crates/smith-cli)");
    assert!(!invocation.contains("omitted"));
}

#[test]
fn history_replay_reconstructs_calls_with_their_outcomes() {
    let history = vec![
        Message::user("read the retry policy"),
        Message::assistant(vec![
            ContentPart::text("Looking."),
            ContentPart::ToolCall(ToolCall {
                id: ToolCallId::new("c1"),
                name: "read".into(),
                arguments: json!({"path": "src/retry.rs"}),
            }),
        ]),
        Message::tool_result(ToolResultBlock {
            call_id: ToolCallId::new("c1"),
            name: "read".into(),
            content: vec![ContentPart::text("fn retry() {}")],
            is_error: false,
        }),
    ];

    let mut transcript = Transcript::new();
    transcript.push_notice(NoticeKind::Stale, "dropped on replay");
    transcript.push_local(LocalResult::Message(Box::new(MessageReport::Notice {
        title: "status".to_owned(),
        message: "model: old".to_owned(),
    })));
    transcript.replace_from_history(&history);

    assert_eq!(transcript.len(), 3);
    assert!(matches!(transcript.blocks()[0], Block::User { .. }));
    match &transcript.blocks()[2] {
        Block::Tool {
            status,
            name,
            display,
            protected_summary,
            ..
        } => {
            assert_eq!(name, "read");
            assert_eq!(*status, ToolStatus::Ok);
            assert!(display.is_none());
            assert_eq!(protected_summary, "arguments hidden");
        }
        other => panic!("expected a tool block, got {other:?}"),
    }
}

#[test]
fn saved_shell_shortcuts_interleave_at_history_anchors() {
    let history = [
        Message::system("hidden system message"),
        Message::user("question"),
        Message::assistant(vec![ContentPart::text("answer")]),
    ];
    let saved = [
        (3, "at end"),
        (2, "in middle"),
        (0, "before everything"),
        (2, "second in middle"),
        (4, "beyond history"),
    ]
    .map(|(anchor, command)| RestoredShellShortcut {
        anchor,
        call: Some(command.to_owned()),
        command: command.to_owned(),
        is_error: false,
        result: Some(format!("output for {command}")),
    });
    let mut transcript = Transcript::new();
    let stale = transcript.push_shell_shortcut("unfinished");
    transcript.replace_from_history_with_shell_shortcuts(&history, &saved);
    let order = transcript
        .blocks()
        .iter()
        .map(|block| match block {
            Block::Tool {
                user_command: Some(command),
                shell_echo: Some(echo),
                started_at,
                status,
                ..
            } => {
                assert_ne!(*echo, stale);
                assert!(started_at.is_none());
                assert_eq!(*status, ToolStatus::Ok);
                command.as_str()
            }
            Block::User { text } | Block::Assistant { text, .. } => text.as_str(),
            other => panic!("unexpected restored block: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        [
            "before everything",
            "question",
            "in middle",
            "second in middle",
            "answer",
            "at end",
        ]
    );
    let echoes = transcript
        .blocks()
        .iter()
        .filter_map(|block| match block {
            Block::Tool { shell_echo, .. } => *shell_echo,
            _ => None,
        })
        .collect::<Vec<_>>();
    let next = transcript.push_shell_shortcut("next");
    assert!(!echoes.contains(&next));
}

#[test]
fn saved_shell_shortcuts_beyond_empty_history_are_dropped() {
    let saved = [0, 1].map(|anchor| RestoredShellShortcut {
        anchor,
        call: None,
        command: format!("anchor {anchor}"),
        is_error: true,
        result: Some("admission rejected".to_owned()),
    });
    let mut transcript = Transcript::new();
    transcript.replace_from_history_with_shell_shortcuts(&[], &saved);
    assert_eq!(transcript.len(), 1);
    assert!(matches!(
        &transcript.blocks()[0],
        Block::Tool {
            user_command: Some(command),
            status: ToolStatus::Failed,
            call_id,
            started_at: None,
            ..
        } if command == "anchor 0" && call_id.is_empty()
    ));
}

#[test]
fn saved_shell_shortcuts_keep_the_exact_live_result_bound() {
    for output in [
        "".to_owned(),
        "one\ttwo\nunsafe\u{202e}control".to_owned(),
        "line\n".repeat(MAX_LOCAL_RESULT_LINES + 10),
        "x".repeat(MAX_LOCAL_RESULT_BYTES + 10),
    ] {
        let mut live = Transcript::new();
        let echo = live.push_shell_shortcut("git status --short");
        let result = live.finish_shell_shortcut(echo, Some("shell-call"), false, &output);
        let saved = RestoredShellShortcut {
            anchor: 0,
            call: Some("shell-call".to_owned()),
            command: "git status --short".to_owned(),
            is_error: false,
            result,
        };
        let mut replay = Transcript::new();
        replay.replace_from_history_with_shell_shortcuts(&[], &[saved]);
        assert_eq!(live.blocks(), replay.blocks());
    }
}

#[test]
fn approval_denial_is_classified_only_from_a_canonical_error_prefix() {
    for (is_error, preview, expected) in [
        (
            true,
            "approval declined: the user declined",
            ToolStatus::Denied,
        ),
        (
            true,
            "approval denied: too many edited action proposals",
            ToolStatus::Denied,
        ),
        (
            false,
            "approval declined: printed by the command",
            ToolStatus::Ok,
        ),
        (
            true,
            "command failed\napproval declined: printed by the command",
            ToolStatus::Failed,
        ),
        (true, "approval timed out", ToolStatus::Failed),
    ] {
        let history = [
            Message::assistant(vec![ContentPart::ToolCall(ToolCall {
                id: ToolCallId::new("c1"),
                name: "shell".to_owned(),
                arguments: json!({}),
            })]),
            Message::tool_result(ToolResultBlock {
                call_id: ToolCallId::new("c1"),
                name: "shell".to_owned(),
                content: vec![ContentPart::text(preview)],
                is_error,
            }),
        ];
        let mut replay = Transcript::new();
        replay.replace_from_history(&history);
        assert_eq!(replay.tool_status("c1"), Some(expected));
        for preview_first in [false, true] {
            let mut live = Transcript::new();
            live.push_tool_call("c1", "shell", None, &[]);
            if preview_first {
                live.set_tool_result_preview("c1", preview);
            }
            live.complete_tool_call(
                "c1",
                if is_error {
                    ToolStatus::Failed
                } else {
                    ToolStatus::Ok
                },
            );
            if !preview_first {
                live.set_tool_result_preview("c1", preview);
            }
            assert_eq!(live.tool_status("c1"), Some(expected));
        }
        let mut shortcut = Transcript::new();
        let echo = shortcut.push_shell_shortcut("git status --short");
        let result = shortcut.finish_shell_shortcut(echo, Some("c1"), is_error, preview);
        assert_eq!(shortcut.tool_status("c1"), Some(expected));
        let mut replayed_shortcut = Transcript::new();
        replayed_shortcut.replace_from_history_with_shell_shortcuts(
            &[],
            &[RestoredShellShortcut {
                anchor: 0,
                call: Some("c1".to_owned()),
                command: "git status --short".to_owned(),
                is_error,
                result,
            }],
        );
        assert_eq!(shortcut.blocks(), replayed_shortcut.blocks());
    }
}

#[test]
fn live_enrichment_and_history_replay_have_built_in_and_unknown_parity() {
    let cases = [
        (
            "read",
            json!({"path": "src/lib.rs", "offset": 4, "limit": 2}),
            Some("Read(src/lib.rs · offset 4 · limit 2)"),
        ),
        (
            "list",
            json!({"recursive": true}),
            Some("List(. · recursive)"),
        ),
        (
            "search",
            json!({"pattern": "needle", "path": "crates"}),
            Some("Search(\"needle\" · crates)"),
        ),
        (
            "edit",
            json!({
                "path": "src/lib.rs",
                "old_string": "before",
                "new_string": "after"
            }),
            Some("Update(src/lib.rs)"),
        ),
        (
            "shell",
            json!({"command": "cargo test", "cwd": "crates"}),
            Some("Bash(cargo test · cwd crates)"),
        ),
        ("third_party", json!({"path": "TOP_SECRET_UNKNOWN"}), None),
    ];

    for (index, (name, arguments, expected)) in cases.into_iter().enumerate() {
        let call_id = format!("call-{index}");
        let keys = argument_keys(&arguments);
        let mut live = Transcript::new();
        live.push_tool_call(&call_id, name, None, &keys);
        if let Some(display) = project_tool_call_display(name, &arguments) {
            live.set_tool_display(&call_id, display);
        }
        live.complete_tool_call(&call_id, ToolStatus::Ok);
        live.set_tool_result_preview(&call_id, "safe first line\nsecond\nthird\nfourth\nfifth");

        let history = vec![
            Message::assistant(vec![ContentPart::ToolCall(ToolCall {
                id: ToolCallId::new(&call_id),
                name: name.to_owned(),
                arguments: arguments.clone(),
            })]),
            Message::tool_result(ToolResultBlock {
                call_id: ToolCallId::new(&call_id),
                name: name.to_owned(),
                content: vec![ContentPart::text("TOP_SECRET_RESULT")],
                is_error: false,
            }),
        ];
        let mut replay = Transcript::new();
        replay.replace_from_history(&history);
        if let Some(display) = project_tool_call_display(name, &arguments) {
            replay.set_tool_display(&call_id, display);
        }

        replay.set_tool_result_preview(&call_id, "safe first line\nsecond\nthird\nfourth\nfifth");
        assert_eq!(
            live.blocks(),
            replay.blocks(),
            "{name}: full result detail must survive replay enrichment"
        );

        let (
            Block::Tool {
                display: live_display,
                protected_summary: live_fallback,
                status: live_status,
                ..
            },
            Block::Tool {
                display: replay_display,
                protected_summary: replay_fallback,
                status: replay_status,
                ..
            },
        ) = (&live.blocks()[0], &replay.blocks()[0])
        else {
            panic!("expected matching tool blocks for {name}");
        };
        assert_eq!(live_display, replay_display, "{name}");
        assert_eq!(live_fallback, replay_fallback, "{name}");
        assert_eq!(live_status, replay_status, "{name}");
        assert_eq!(
            live_display.as_deref().map(ToolCallDisplay::invocation),
            expected.map(str::to_owned),
            "{name}"
        );
    }
}

#[test]
fn replayed_user_images_keep_a_visible_marker() {
    let history = vec![Message {
        role: Role::User,
        content: vec![
            ContentPart::text("what is this?"),
            ContentPart::Image {
                url: "data:image/png;base64,SECRETPIXELS".into(),
                detail: None,
            },
        ],
    }];
    let mut transcript = Transcript::new();
    transcript.replace_from_history(&history);

    match &transcript.blocks()[0] {
        Block::User { text } => {
            assert_eq!(text, "what is this?\n[image]");
            assert!(!text.contains("SECRETPIXELS"));
        }
        other => panic!("expected a user block, got {other:?}"),
    }
}

#[test]
fn system_messages_stay_out_of_the_transcript() {
    let mut transcript = Transcript::new();
    transcript.replace_from_history(&[Message::system("be concise"), Message::user("hi")]);
    assert_eq!(transcript.len(), 1);
    assert!(matches!(transcript.blocks()[0], Block::User { .. }));
}
