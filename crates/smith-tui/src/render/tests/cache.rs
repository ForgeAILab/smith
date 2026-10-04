use super::*;
use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::content::{ContentPart, Message, ToolCall};
use agent_runtime_core::ids::{AttemptId, EventId, RequestId, SessionId, ToolCallId};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use smith_client::NoticeKind;
use smith_client::message_report::MessageReport;
use smith_runtime::client::{SmithEvent, SmithEventKind, TurnFinish};

const STATUSES: [ToolStatus; 6] = [
    ToolStatus::Running,
    ToolStatus::WaitingForApproval,
    ToolStatus::Ok,
    ToolStatus::Failed,
    ToolStatus::Denied,
    ToolStatus::Unreported,
];

#[test]
fn streaming_table_cache_replaces_the_placeholder_when_the_block_closes() {
    let table = "| Name | State |\n| --- | --- |\n| Retry | ready |\n| Cancel | waiting |";
    for width in [44, 80, 100] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            let mut app = App::new("model", "project");
            app.transcript.push_text_delta(table);
            let open_revision = app.transcript.block_revision(0);
            let receiving = transcript_rows(&app, theme, width).window(0, 100);
            assert_eq!(receiving.len(), 2);
            assert_eq!(receiving[1].to_string(), "  receiving table…");
            let renders = app.transcript_cache.borrow().renders;
            app.transcript.close_open();
            assert_ne!(app.transcript.block_revision(0), open_revision);
            let full = transcript_rows(&app, theme, width).window(0, 100);
            assert_eq!(app.transcript_cache.borrow().renders, renders + 1);
            assert_eq!(
                full,
                wrap_lines(&render_assistant_lines(table, theme, width, false), width)
            );
            assert!(full.iter().any(|row| row.to_string().contains("Retry")));
            assert!(full.iter().any(|row| row.to_string().contains("Cancel")));
            assert_matches_uncached(&app, theme, width);
        }
    }
}

#[test]
fn streaming_table_speculative_and_open_paths_render_in_full_after_a_paragraph() {
    let table = "| Name | State |\n| --- | --- |\n| Retry | ready |";
    for width in [44, 80, 100] {
        let theme = Theme::new().without_color();
        for open_prefix in [false, true] {
            let mut app = App::new("model", "project");
            if open_prefix {
                app.transcript.push_text_delta("Earlier paragraph.\n\n");
            }
            app.apply(&event(SmithEventKind::TextDelta {
                request: RequestId::new("r"),
                attempt: AttemptId::new("a"),
                text: table.to_owned(),
            }));
            assert_matches_uncached(&app, theme, width);
            let receiving = transcript_rows(&app, theme, width).window(0, 100);
            assert_eq!(receiving.last().unwrap().to_string(), "  receiving table…");
            app.apply(&event(SmithEventKind::ProviderAttemptOutputCommitted {
                request: RequestId::new("r"),
                attempt: AttemptId::new("a"),
            }));
            assert_eq!(
                transcript_rows(&app, theme, width).window(0, 100),
                receiving
            );
            app.transcript.push_text_delta("\n\nFollowing paragraph.");
            assert_matches_uncached(&app, theme, width);
            let full = transcript_rows(&app, theme, width).window(0, 100);
            assert!(full.iter().any(|row| row.to_string().contains("Retry")));
            assert!(
                !full
                    .iter()
                    .any(|row| row.to_string().contains("receiving table"))
            );
        }
    }
}

fn event(payload: SmithEventKind) -> SmithEvent {
    SmithEvent::new(
        0,
        EventId::new("e"),
        SessionId::new("s"),
        None,
        Timestamp::ZERO,
        payload,
    )
}

fn local_result(index: usize) -> LocalResult {
    LocalResult::Message(Box::new(MessageReport::Notice {
        title: "status".to_owned(),
        message: format!("Local result {index}\nNo provider request was needed."),
    }))
}

fn mixed_app() -> App {
    let mut app = App::new("model", "project");
    let mut history = Vec::new();
    // One turn per status spans several screens while retaining Markdown,
    // wide characters, reasoning, tools, previews, local results, and notices.
    for index in 0..STATUSES.len() {
        history.push(Message::user(format!(
            "Question {index}: {}",
            "界 retry policy ".repeat(1 + index % 5)
        )));
        history.push(Message::assistant(vec![
            ContentPart::text(format!("## Answer {index}\n\n**Strong** text, *emphasis*, `code`, and [a link](https://example.com).\n\n- first item\n- second item\n\n```rust\nlet value = {index};\n```\n\n| Key | Value |\n| --- | --- |\n| retry | bounded |")),
            ContentPart::Reasoning {
                text: "hidden reasoning".to_owned(),
                redacted: false,
                signature: None,
                producer: None,
            },
            ContentPart::ToolCall(ToolCall {
                id: ToolCallId::new(format!("call-{index}")),
                name: "read".to_owned(),
                arguments: serde_json::json!({"path": format!("src/module_{index}.rs")}),
            }),
        ]));
    }
    // History leaves running calls without local clocks, making the generated
    // property independent of a second ticking between reference and cache.
    app.transcript.replace_from_history(&history);
    for (index, status) in STATUSES.into_iter().enumerate() {
        let call = format!("call-{index}");
        let display = smith_tools::project_tool_call_display(
            "read",
            &serde_json::json!({"path": format!("src/module_{index}.rs")}),
        )
        .expect("read display");
        app.transcript.set_tool_display(&call, display);
        app.transcript.complete_tool_call(&call, status);
        app.transcript.set_tool_result_preview(
            &call,
            format!(
                "line {index}\none\ntwo\nthree\nfour\nfive\nsix\n{}",
                "preview 界 ".repeat(12)
            ),
        );
        app.transcript.push_local(local_result(index));
        app.transcript
            .push_notice(NoticeKind::Monitor, format!("Notice {index}\ncontinued"));
    }
    for name in ["write_todos", "registry.search", "agent"] {
        let call = format!("suppressed-{name}");
        app.transcript.push_tool_call(
            &call,
            name,
            Some(&serde_json::json!({"action": "wait", "child_id": "child"})),
            &[],
        );
        app.transcript.complete_tool_call(&call, ToolStatus::Ok);
        app.transcript
            .set_tool_result_preview(&call, "This preview is suppressed too.");
    }
    let echo = app.transcript.push_shell_shortcut("printf 'shell echo'");
    app.transcript
        .finish_shell_shortcut(echo, None, false, "one\ntwo\nthree\nfour\nfive\nsix");
    app.transcript.push_user("");
    app.transcript.push_error("");
    app.transcript.push_notice(NoticeKind::Turn, "");
    app.transcript.push_text_delta("An open **streaming");
    app
}

fn assert_matches_uncached(app: &App, theme: Theme, width: u16) {
    // This reference renders every block afresh through the original
    // block_lines/conversation_lines path, without consulting the cache.
    let expected = wrap_lines(&transcript_lines(app, theme, width), width);
    let rows = transcript_rows(app, theme, width);
    let height = u16::try_from(expected.len()).expect("generated transcript fits a test window");
    assert_eq!(
        rows.window(0, height),
        expected,
        "width {width}, expanded {}",
        app.work_details
    );
    assert_eq!(
        rows.scroll_limit(Rect::new(0, 0, width, 17)),
        expected.len().saturating_sub(17)
    );
    for offset in [
        0,
        1,
        expected.len() / 2,
        expected.len().saturating_sub(17),
        expected.len(),
        expected.len() + 1,
    ] {
        assert_eq!(
            rows.window(offset, 17),
            expected
                .iter()
                .skip(offset)
                .take(17)
                .cloned()
                .collect::<Vec<_>>(),
            "window at {offset}, width {width}"
        );
    }
    if rows.cache.is_some() {
        for block in [
            0,
            1,
            app.transcript.len() / 2,
            app.transcript.len().saturating_sub(1),
        ]
        .into_iter()
        .filter(|block| *block < app.transcript.len())
        {
            let preceding = block_lines(
                &app.transcript.blocks()[..block],
                theme,
                width,
                app.work_details,
            );
            let offset = wrapped_row_count(&preceding, width) + usize::from(!preceding.is_empty());
            assert_eq!(rows.block_start_row(block), Some(offset));
            for start in [offset.saturating_sub(1), offset, offset + 1] {
                assert_eq!(
                    rows.window(start, 17),
                    expected
                        .iter()
                        .skip(start)
                        .take(17)
                        .cloned()
                        .collect::<Vec<_>>()
                );
            }
        }
        assert_eq!(rows.block_start_row(app.transcript.len()), None);
    }
}

#[test]
fn cached_transcript_matches_full_render_after_mutations() {
    for width in [44, 80, 100] {
        for expanded in [false, true] {
            for theme in [Theme::new(), Theme::new().without_color()] {
                check_mutations(width, expanded, theme);
            }
        }
    }
}

fn check_mutations(width: u16, expanded: bool, theme: Theme) {
    let mut app = mixed_app();
    app.work_details = expanded;
    // Keep this configuration warm across mutations: changing the cache
    // key before checking would hide a missing block revision bump.
    let check = |app: &App| assert_matches_uncached(app, theme, width);
    check(&app);
    assert!(
        transcript_rows(&app, theme, width).scroll_limit(Rect::new(0, 0, width, 17)) >= 34,
        "mixed fixture must span at least three screens"
    );
    app.transcript.push_text_delta(" answer**\n\n");
    check(&app);
    app.transcript.push_text_delta("```rust\n");
    check(&app);
    app.transcript.push_text_delta("let retry = true;\n");
    app.transcript.push_text_delta("```\n\nFinished.");
    check(&app);
    app.transcript
        .push_notice(NoticeKind::Monitor, "Arrived during streaming");
    check(&app);
    app.transcript.push_text_delta("A new assistant boundary.");
    check(&app);
    app.transcript.push_reasoning_delta("hidden", false);
    check(&app);
    app.transcript.push_reasoning_delta(" continuation", false);
    check(&app);
    app.transcript
        .push_reasoning_delta("redacted boundary", true);
    check(&app);
    // Leave call-0 running so name-based completion and settling below
    // still mutate real blocks after all statuses have been exercised.
    let call = "call-2";
    for status in STATUSES {
        app.transcript.complete_tool_call(call, status);
        check(&app);
    }
    for preview in ["updated\npreview\nwith\nfive\nlines", "short preview"] {
        app.transcript.set_tool_result_preview(call, preview);
        check(&app);
    }
    app.transcript.set_tool_display(
        "call-1",
        smith_tools::project_tool_call_display("read", &serde_json::json!({"path": "changed.rs"}))
            .expect("read display"),
    );
    check(&app);
    app.transcript
        .enrich_tool_call("call-1", ["host-confirmed".to_owned()]);
    check(&app);
    app.transcript
        .complete_tool_call("suppressed-write_todos", ToolStatus::Failed);
    check(&app);
    app.transcript
        .complete_tool_call("suppressed-write_todos", ToolStatus::Ok);
    check(&app);
    app.transcript
        .complete_tool_call_by_name("read", ToolStatus::Failed);
    check(&app);
    app.transcript
        .settle_running_tool_calls(ToolStatus::Unreported);
    check(&app);
    app.transcript.push_external_tool_call(
        "external",
        "Read",
        &serde_json::json!({"path": "external.rs"}),
    );
    app.transcript
        .complete_tool_call("external", ToolStatus::Ok);
    check(&app);
    let echo = app.transcript.push_shell_shortcut("echo exact identity");
    app.transcript
        .finish_shell_shortcut(echo, None, false, "first result");
    check(&app);
    app.transcript.bind_shell_shortcut(echo, "new-shell");
    check(&app);
    app.transcript
        .finish_shell_shortcut(echo, Some("new-shell"), true, "updated result");
    check(&app);
    let echo = app.transcript.push_shell_shortcut("echo merged identity");
    app.transcript
        .finish_shell_shortcut(echo, None, false, "echo result");
    app.transcript
        .push_tool_call("merged-shell", "shell", None, &[]);
    app.transcript
        .complete_tool_call("merged-shell", ToolStatus::Ok);
    app.transcript
        .set_tool_result_preview("merged-shell", "runtime result");
    check(&app);
    app.transcript.bind_shell_shortcut(echo, "merged-shell");
    check(&app);
    app.transcript
        .finish_shell_shortcut(echo, Some("merged-shell"), true, "late result");
    check(&app);
    app.transcript.push_local(local_result(STATUSES.len()));
    check(&app);
    // Resize and toggle expansion on the same populated cache, then
    // return to the original key before the remaining mutations.
    for resized in [100, 44, 80, width] {
        assert_matches_uncached(&app, theme, resized);
    }
    app.work_details = !expanded;
    check(&app);
    app.work_details = expanded;
    check(&app);
    app.transcript.retain_newest(12);
    check(&app);
    app.apply(&event(SmithEventKind::TurnStarted));
    app.transcript.push_text_delta("Committed **body");
    app.apply(&event(SmithEventKind::TextDelta {
        request: RequestId::new("r"),
        attempt: AttemptId::new("a"),
        text: " plus speculative tail**".to_owned(),
    }));
    check(&app);
    app.apply(&event(SmithEventKind::ProviderAttemptOutputCommitted {
        request: RequestId::new("r"),
        attempt: AttemptId::new("a"),
    }));
    check(&app);
    app.apply(&event(SmithEventKind::TurnCompleted {
        finish: TurnFinish::Completed,
        visible_output: true,
    }));
    check(&app);
    for text in ["First replacement", "Same-size replacement", ""] {
        app.transcript.replace_from_history_with_shell_shortcuts(
            &[Message::user(text)],
            &[crate::transcript::RestoredShellShortcut {
                anchor: 1,
                call: None,
                command: "saved echo".to_owned(),
                is_error: false,
                result: Some("saved result".to_owned()),
            }],
        );
        check(&app);
    }
    app.transcript.replace_from_history(&[]);
    check(&app);
}

#[test]
fn unchanged_blocks_are_reused_while_the_open_block_streams() {
    let mut app = App::new("model", "project");
    for index in 0..1_000 {
        app.transcript
            .push_notice(NoticeKind::Monitor, format!("Block {index}"));
    }
    app.transcript.push_text_delta("Streaming answer");
    let theme = Theme::new().without_motion();
    let area = Rect::new(0, 0, 100, 32);
    let mut terminal = Terminal::new(TestBackend::new(100, 32)).expect("terminal");
    terminal
        .draw(|frame| super::super::draw(frame, &app, theme))
        .expect("frame");
    let initial = app.transcript_cache.borrow().renders;
    assert_eq!(initial, app.transcript.len());
    for frame_index in 1..=8 {
        app.transcript.push_text_delta(" more text");
        super::super::layout(area, &app, theme).apply(&mut app);
        terminal
            .draw(|frame| super::super::draw(frame, &app, theme))
            .expect("frame");
        assert_eq!(app.transcript_cache.borrow().renders, initial + frame_index);
    }
    for (width, expanded, next_theme) in [
        (80, false, theme),
        (80, true, theme),
        (80, true, theme.without_color()),
    ] {
        let before = app.transcript_cache.borrow().renders;
        app.work_details = expanded;
        drop(transcript_rows(&app, next_theme, width));
        assert_eq!(
            app.transcript_cache.borrow().renders - before,
            app.transcript.len()
        );
    }
}

#[test]
fn replacement_transcripts_and_diverging_clones_do_not_reuse_old_rows() {
    let mut app = App::new("model", "project");
    app.transcript.push_text_delta("Original body");
    let mut other = app.transcript.clone();
    let theme = Theme::new();
    assert_matches_uncached(&app, theme, 80);
    app.transcript.push_text_delta(" first branch");
    assert_matches_uncached(&app, theme, 80);
    other.push_text_delta(" second branch");
    app.transcript = other;
    assert_matches_uncached(&app, theme, 80);
    app.transcript = crate::transcript::Transcript::new();
    app.transcript.push_text_delta("Independent replacement");
    assert_matches_uncached(&app, theme, 80);
}

#[test]
fn a_transcript_over_65535_rows_scrolls_to_its_first_row() {
    let mut app = App::new("model", "project");
    app.transcript.push_user(
        (0..70_000)
            .map(|row| format!("row {row}\n"))
            .collect::<String>(),
    );
    let theme = Theme::new().without_motion();
    let area = Rect::new(0, 0, 100, 32);
    super::super::layout(area, &app, theme).apply(&mut app);
    assert!(app.scroll_limit > usize::from(u16::MAX));
    app.on_key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL));
    super::super::layout(area, &app, theme).apply(&mut app);
    assert_eq!(app.scroll_back, app.scroll_limit);
    let mut terminal = Terminal::new(TestBackend::new(100, 32)).expect("terminal");
    terminal
        .draw(|frame| super::super::draw(frame, &app, theme))
        .expect("frame");
    assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), ">");
    assert_eq!(terminal.backend().buffer()[(2, 0)].symbol(), "r");
    let first = (0..7)
        .map(|column| terminal.backend().buffer()[(column, 0)].symbol())
        .collect::<String>();
    assert_eq!(first, "> row 0");
}

#[test]
fn repeated_draws_leave_application_state_unchanged() {
    let mut app = App::new("model", "project");
    for index in 0..20 {
        app.transcript
            .push_notice(NoticeKind::Monitor, format!("Block {index}"));
    }
    app.show_local_report(LocalResult::Help(Box::new(smith_client::commands::help())));
    let theme = Theme::new().without_motion();
    let area = Rect::new(0, 0, 80, 24);
    let before_layout = (
        app.following,
        app.scroll_back,
        app.scroll_limit,
        app.scroll_to_block,
        app.result_scroll_revision,
        app.approval_scroll,
        app.approval_scroll_limit,
    );
    let computed = super::super::layout(area, &app, theme);
    // Render-only cache changes are permitted; layout must leave all scroll
    // and result state alone until its explicit apply step.
    assert_eq!(
        before_layout,
        (
            app.following,
            app.scroll_back,
            app.scroll_limit,
            app.scroll_to_block,
            app.result_scroll_revision,
            app.approval_scroll,
            app.approval_scroll_limit,
        )
    );
    assert!(app.scroll_to_block.is_some());
    assert_eq!(app.scroll_limit, 0);
    computed.apply(&mut app);
    assert!(app.scroll_to_block.is_none());
    let before = format!("{app:?}");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
    terminal
        .draw(|frame| super::super::draw(frame, &app, theme))
        .expect("frame");
    let first = terminal.backend().buffer().clone();
    terminal
        .draw(|frame| super::super::draw(frame, &app, theme))
        .expect("frame");
    assert_eq!(terminal.backend().buffer(), &first);
    assert_eq!(format!("{app:?}"), before);
}
