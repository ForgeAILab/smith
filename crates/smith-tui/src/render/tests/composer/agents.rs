use super::*;

#[test]
fn live_findings_agents_panel_labels_counts_and_omits_session_identity() {
    use crate::app::ChildCounts;
    use agent_runtime_core::ids::ChildId;
    use smith_runtime::client::{ChildPhase, ChildRecoveryState};

    for (used, max, expected) in [
        (1, u32::MAX, "1 turn"),
        (2, u32::MAX, "2 turns"),
        (1, 1, "1/1 turn"),
        (1, 5, "1/5 turns"),
    ] {
        for phase in [
            ChildPhase::Recovered {
                child_session: SessionId::new("CHILD_SESSION_MUST_NOT_APPEAR"),
                state: ChildRecoveryState::Idle,
                resumable: false,
            },
            ChildPhase::ResumeStarted {
                child_session: SessionId::new("CHILD_SESSION_MUST_NOT_APPEAR"),
            },
            ChildPhase::Interrupted {
                child_session: SessionId::new("CHILD_SESSION_MUST_NOT_APPEAR"),
                resumable: true,
            },
        ] {
            let mut app = App::new("model", "project");
            app.apply(&event(RuntimeEvent::ChildProgress {
                child: ChildId::new("child-1"),
                phase,
            }));
            app.set_child_counts(std::collections::BTreeMap::from([(
                "child-1".to_owned(),
                ChildCounts {
                    turns_used: used,
                    max_turns: max,
                    tokens_used: 3_100,
                },
            )]));
            let screen = render(&app, 140, 24, Theme::new().without_color());
            let row = screen
                .lines()
                .find(|row| row.contains("○ child-1"))
                .expect("the child panel row");
            assert!(row.contains("durable"), "{row}");
            assert!(
                row.contains(app.children["child-1"].state.label().as_ref()),
                "{row}"
            );
            assert!(row.contains(&format!("{expected} · 3.1k tokens")), "{row}");
            assert!(
                !row.contains("CHILD_SESSION_MUST_NOT_APPEAR") && !row.contains("session"),
                "{row}"
            );
            assert_eq!(row.matches("tokens").count(), 1, "{row}");
            assert!(!row.contains("1 turns"), "{row}");
        }
    }
}

#[test]
fn delegated_agents_panel_lists_children_under_the_hint_with_clocks() {
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::ids::ChildId;

    let mut app = App::new("gpt-5.3", "~/work/api");
    for id in ["child-a", "child-b"] {
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: ChildId::new(id),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        }));
    }
    app.apply(&event(RuntimeEvent::ChildCompleted {
        child: ChildId::new("child-b"),
        result: "No findings.".to_owned(),
    }));

    let screen = render(&app, 80, 24, Theme::new().without_color());
    insta_like(
        &screen,
        &[
            "● main",
            // No spawn call preceded these events, so the panel falls
            // back to the root's own profile — the same "inherited"
            // resolution `ChildSpawned` applies when nothing was
            // selected.
            "○ child-a  build · read-only",
            "○ child-b  build · completed · No findings.",
        ],
    );
    let clocks = screen
        .lines()
        .filter(|line| line.contains("child-") && line.ends_with("0s"))
        .count();
    assert_eq!(clocks, 2, "both rows dock a right-aligned clock:\n{screen}");
    let main_row = screen
        .lines()
        .position(|line| line.contains("● main"))
        .expect("a main row");
    let composer_row = screen
        .lines()
        .position(|line| line.contains("Ask Smith to do anything"))
        .expect("the composer placeholder");
    assert!(
        main_row > composer_row,
        "the panel sits below the composer:\n{screen}"
    );
}

#[test]
fn a_working_childs_row_shows_the_reviewed_projection_profile_and_coordinator_counts() {
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::ids::ChildId;

    let mut app = App::new("gpt-5.3", "~/work/api");
    let spawn_args = serde_json::json!({
        "action": "spawn",
        "task": "review the diff",
        "tools": "read_only",
        "workspace": "shared",
        "profile": "review"
    });
    let display = smith_tools::project_tool_call_display("agent", &spawn_args)
        .expect("reviewed spawn projection");
    app.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("spawn-1"),
        name: "agent".to_owned(),
        argument_keys: vec![
            "action".into(),
            "task".into(),
            "tools".into(),
            "workspace".into(),
            "profile".into(),
        ],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
        arguments: None,
    }));
    app.note_pending_spawn("spawn-1", &display);
    app.set_tool_display("spawn-1", display);
    let child = ChildId::new("child-1");
    app.apply(&event(RuntimeEvent::ChildSpawned {
        child: child.clone(),
        workspace: WorkspacePolicy::SharedProject,
        max_turns: 5,
        max_tokens: None,
        deadline_ms: None,
    }));

    app.apply_child(
        child.as_str(),
        &event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("child-call-1"),
            name: "read".to_owned(),
            argument_keys: vec!["path".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }),
    );
    app.set_child_tool_display(
        child.as_str(),
        "child-call-1",
        smith_tools::project_tool_call_display(
            "read",
            &serde_json::json!({"path": "src/retry.rs"}),
        )
        .expect("reviewed read projection"),
    );
    app.set_child_counts(std::collections::BTreeMap::from([(
        child.to_string(),
        crate::app::ChildCounts {
            turns_used: 2,
            max_turns: 5,
            tokens_used: 12_400,
        },
    )]));

    let screen = render(&app, 100, 24, Theme::new().without_color());
    assert!(
        screen.contains("○ child-1  review · Read(src/retry.rs) · 2/5 turns · 12.4k tokens"),
        "the row shows the reviewed projection, not the bare tool name, beside the \
             child's profile and the coordinator's own counts:\n{screen}"
    );
}

#[test]
fn a_child_tool_with_no_reviewed_projection_names_the_tool_with_an_honest_label() {
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::ids::ChildId;

    const RAW_ARGUMENT: &str = "TOP_SECRET_QUERY";
    let mut app = App::new("gpt-5.3", "~/work/api");
    let child = ChildId::new("child-plain");
    app.apply(&event(RuntimeEvent::ChildSpawned {
        child: child.clone(),
        workspace: WorkspacePolicy::ReadOnlyView,
        max_turns: 1,
        max_tokens: None,
        deadline_ms: None,
    }));
    app.apply_child(
        child.as_str(),
        &event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("call-1"),
            name: "mcp__docs__some_third_party_tool".to_owned(),
            argument_keys: vec!["query".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: Some(serde_json::json!({"query": RAW_ARGUMENT})),
        }),
    );

    let screen = render(&app, 100, 24, Theme::new().without_color());
    let row = screen
        .lines()
        .find(|line| line.contains(child.as_str()))
        .expect("a panel row");
    assert!(
        row.contains("mcp__docs__some_third_party_tool(arguments hidden)"),
        "the tool is named with an honest unavailable label rather than a raw argument \
             value: {row}"
    );
    assert!(!row.contains("query"), "{row}");
    assert!(!row.contains(RAW_ARGUMENT), "{row}");
}

#[test]
fn a_long_child_activity_clips_before_the_docked_clock() {
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::ids::ChildId;

    let mut app = App::new("gpt-5.3", "~/work/api");
    let child = ChildId::new("child-verbose");
    app.apply(&event(RuntimeEvent::ChildSpawned {
        child: child.clone(),
        workspace: WorkspacePolicy::ReadOnlyView,
        max_turns: 1,
        max_tokens: None,
        deadline_ms: None,
    }));
    let many_keys = (0..12)
        .map(|index| format!("argument_key_number_{index}"))
        .collect::<Vec<_>>();
    app.apply_child(
        child.as_str(),
        &event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("call-1"),
            name: "mcp__docs__some_third_party_tool".to_owned(),
            argument_keys: many_keys,
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }),
    );

    let screen = render(&app, 80, 24, Theme::new().without_color());
    let row = screen
        .lines()
        .find(|line| line.contains(child.as_str()))
        .expect("a panel row");
    assert!(
        row.width() <= 80,
        "the row must not overflow the panel width: {row}"
    );
    assert!(
        row.trim_end().ends_with("0s"),
        "the elapsed clock stays docked at the right edge even though the activity is \
             long enough to clip: {row:?}"
    );
    assert!(
        !row.contains("argument_key_number_11"),
        "the key list is bounded, not an unbounded dump, and the whole thing still \
             clips well short of the clock: {row}"
    );
}

#[test]
fn inspecting_a_child_swaps_the_transcript_for_its_log_and_returns_it_on_escape() {
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::ids::ChildId;

    let mut app = App::new("gpt-5.3", "~/work/api");
    app.transcript.push_user("explain the retry policy");
    let child = ChildId::new("child-a");
    app.apply(&event(RuntimeEvent::ChildSpawned {
        child: child.clone(),
        workspace: WorkspacePolicy::ReadOnlyView,
        max_turns: 3,
        max_tokens: None,
        deadline_ms: None,
    }));
    // The child's own stream, folded by the same code as the root's.
    app.apply_child(
        child.as_str(),
        &event(RuntimeEvent::ToolCallRequested {
            call: agent_runtime_core::ids::ToolCallId::new("child-call-1"),
            name: "search".to_owned(),
            argument_keys: vec!["path".to_owned(), "pattern".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }),
    );
    // The host resolves the call against the child's canonical history
    // and hands back the same redacted projection the root timeline gets.
    app.set_child_tool_display(
        child.as_str(),
        "child-call-1",
        smith_tools::project_tool_call_display(
            "search",
            &serde_json::json!({"path": "src/retry.rs", "pattern": "backoff"}),
        )
        .expect("reviewed search projection"),
    );
    app.apply_child(
        child.as_str(),
        &event(RuntimeEvent::ToolCallCompleted {
            call: agent_runtime_core::ids::ToolCallId::new("child-call-1"),
            name: "search".to_owned(),
            is_error: false,
        }),
    );
    app.set_child_tool_result_preview(child.as_str(), "child-call-1", "src/retry.rs:42");
    // Mid-sentence: uncommitted, and drawn like the root's own streaming
    // answer rather than held back until the child finishes.
    app.apply_child(
        child.as_str(),
        &event(RuntimeEvent::TextDelta {
            request: RequestId::new("child-request-1"),
            attempt: AttemptId::new("child-attempt-1"),
            text: "The retry policy backs off".to_owned(),
        }),
    );

    let root = render(&app, 80, 24, Theme::new().without_color());
    assert!(
        root.contains("explain the retry policy"),
        "the root timeline is what the transcript shows:\n{root}"
    );
    assert!(
        !root.contains("sub-agent · child-a ran"),
        "a child's tool call is panel activity, not a transcript notice:\n{root}"
    );

    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let inspected = render(&app, 80, 24, Theme::new().without_color());
    insta_like(
        &inspected,
        &[
            "child-a · running",
            "started · read-only · up to 3 turns",
            // The child's tool call draws as the root timeline's do:
            // same row, same shape, the same reviewed arguments and
            // result lines beneath it.
            "Search(\"backoff\" · src/retry.rs)",
            "src/retry.rs:42",
            "The retry policy backs off",
            "esc back to main",
        ],
    );
    assert!(
        !root.contains("The retry policy backs off"),
        "a child streaming is not the root session streaming:\n{root}"
    );
    assert!(
        !inspected.contains("explain the retry policy"),
        "the inspector borrows the whole transcript region:\n{inspected}"
    );
    // The panel marks which row the region belongs to.
    insta_like(&inspected, &["○ main", "● child-a"]);

    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let restored = render(&app, 80, 24, Theme::new().without_color());
    assert!(
        restored.contains("explain the retry policy") && !restored.contains("child-a · running"),
        "esc gives the region back unchanged:\n{restored}"
    );
}

#[test]
fn the_working_row_counts_live_delegated_agents() {
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::ids::ChildId;

    let mut app = App::new("gpt-5.3", "~/work/api");
    app.apply(&event(RuntimeEvent::TurnStarted));
    for id in ["child-a", "child-b"] {
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: ChildId::new(id),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        }));
    }
    let screen = render(&app, 80, 24, Theme::new().without_color());
    insta_like(&screen, &["· 2 agents"]);

    app.apply(&event(RuntimeEvent::ChildCompleted {
        child: ChildId::new("child-b"),
        result: "done".to_owned(),
    }));
    let screen = render(&app, 80, 24, Theme::new().without_color());
    insta_like(&screen, &["· 1 agent"]);
}

#[test]
fn background_tasks_join_the_delegated_panel_with_clocks_and_leave_on_poll() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.set_running_tasks(vec![crate::app::RunningTaskSummary {
        task_id: "task:3".to_owned(),
        command_hint: "npm test".to_owned(),
    }]);

    let screen = render(&app, 80, 24, Theme::new().without_color());
    insta_like(&screen, &["● main", "○ task:3  npm test"]);
    assert!(
        screen
            .lines()
            .any(|line| line.contains("task:3") && line.ends_with("0s")),
        "the task row docks a right-aligned clock:\n{screen}"
    );

    app.set_running_tasks(Vec::new());
    let cleared = render(&app, 80, 24, Theme::new().without_color());
    assert!(!cleared.contains("○ task:3"), "{cleared}");
    assert!(
        !cleared.contains("● main"),
        "an empty panel vanishes:\n{cleared}"
    );
}

#[test]
fn a_clean_finish_reads_as_success_wherever_its_state_is_named() {
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::error::RuntimeError;
    use agent_runtime_core::ids::ChildId;

    let mut app = App::new("gpt-5.3", "~/work/api");
    for id in ["child-done", "child-bad"] {
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: ChildId::new(id),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        }));
    }
    app.apply(&event(RuntimeEvent::ChildCompleted {
        child: ChildId::new("child-done"),
        result: "No findings.".to_owned(),
    }));
    app.apply(&event(RuntimeEvent::ChildFailed {
        child: ChildId::new("child-bad"),
        error: RuntimeError::internal("provider refused"),
    }));

    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("a test terminal");
    terminal
        .draw(|frame| draw(frame, &app, Theme::new()))
        .expect("a frame");
    let buffer = terminal.backend().buffer().clone();
    // The panel row, not the transcript notice that names the same child.
    let panel_colour_of = |child: &str| {
        let needle = format!("○ {child}");
        for y in 0..buffer.area.height {
            let row: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect();
            if let Some(x) = row.find(&needle) {
                let column = u16::try_from(x + 2).expect("an on-screen column");
                return buffer[(column, y)].fg;
            }
        }
        panic!("missing the `{child}` panel row on screen");
    };
    assert_eq!(panel_colour_of("child-done"), Color::Green);
    assert_eq!(
        panel_colour_of("child-bad"),
        Color::Red,
        "a failure must not be recoloured by the success rule"
    );

    // The inspector heading names the same state and must agree with it.
    app.inspect_child("child-done");
    let lines = transcript_lines(&app, Theme::new(), 80);
    let heading = lines
        .iter()
        .find(|line| line.spans.iter().any(|span| span.content == "child-done"))
        .expect("an inspector heading");
    assert_eq!(
        heading.spans.last().expect("the state segment").style.fg,
        Some(Color::Green)
    );
}

#[test]
fn the_inspector_renders_a_child_answer_as_prose_not_one_clipped_line() {
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::ids::ChildId;

    let mut app = App::new("gpt-5.3", "~/work/api");
    let child = ChildId::new("child-a");
    app.apply(&event(RuntimeEvent::ChildSpawned {
        child: child.clone(),
        workspace: WorkspacePolicy::ReadOnlyView,
        max_turns: 1,
        max_tokens: None,
        deadline_ms: None,
    }));
    app.apply(&event(RuntimeEvent::ChildCompleted {
        child: child.clone(),
        result: "## report\n\nOne finding in `resolve`.\nA second line.".to_owned(),
    }));
    app.inspect_child("child-a");

    let lines = transcript_lines(&app, Theme::new(), 80);
    let text = |line: &Line<'_>| {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    };
    let rendered = lines.iter().map(text).collect::<Vec<_>>();
    assert!(
        rendered.iter().all(|line| !line.contains('\n')),
        "an embedded newline draws as a glyph, not a line break: {rendered:#?}"
    );
    assert!(
        rendered.iter().any(|line| line.contains("A second line.")),
        "every line of the answer is kept: {rendered:#?}"
    );

    let heading = lines
        .iter()
        .find(|line| text(line).contains("report"))
        .expect("the answer's own heading");
    assert!(
        !text(heading).contains('#'),
        "the answer renders as Markdown, like the root transcript's prose"
    );
    let code = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .find(|span| span.content == "resolve")
        .expect("inline code in the answer");
    assert_eq!(code.style.fg, Some(Color::Cyan));
}

#[test]
fn a_child_answer_draws_exactly_as_the_root_timeline_draws_one() {
    use agent_runtime_core::delegation::WorkspacePolicy;
    use agent_runtime_core::ids::ChildId;

    const ANSWER: &str = "## report\n\nOne finding in `resolve`.\nA second line.";
    let styled = |lines: &[Line<'static>], needle: &str| {
        lines
            .iter()
            .filter(|line| line.spans.iter().any(|span| span.content.contains(needle)))
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| (span.content.to_string(), span.style))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };

    let mut root = App::new("gpt-5.3", "~/work/api");
    root.transcript.push_text_delta(ANSWER);
    root.transcript.close_open();
    let root_lines = transcript_lines(&root, Theme::new(), 80);

    let mut app = App::new("gpt-5.3", "~/work/api");
    let child = ChildId::new("child-a");
    app.apply(&event(RuntimeEvent::ChildSpawned {
        child: child.clone(),
        workspace: WorkspacePolicy::ReadOnlyView,
        max_turns: 1,
        max_tokens: None,
        deadline_ms: None,
    }));
    app.apply(&event(RuntimeEvent::ChildCompleted {
        child: child.clone(),
        result: ANSWER.to_owned(),
    }));
    app.inspect_child("child-a");
    let child_lines = transcript_lines(&app, Theme::new(), 80);

    for needle in ["report", "One finding", "A second line."] {
        let from_root = styled(&root_lines, needle);
        assert!(!from_root.is_empty(), "`{needle}` is missing from the root");
        assert_eq!(
            styled(&child_lines, needle),
            from_root,
            "a delegated child is an agent that reports back, so its answer \
                 must draw through the same renderer, down to the styles"
        );
    }
}
