use super::*;

fn bridged_agent_request() -> RuntimeEvent {
    RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("bridge-agent"),
        name: "agent".to_owned(),
        argument_keys: vec!["action".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("list"),
        arguments: Some(serde_json::json!({"action":"list"})),
    }
}

#[test]
fn mixed_harness_rows_render_once_with_native_smith_detail_and_cli_origin() {
    let mut app = App::new("cli/claude-code/sonnet", "/repo");
    app.status.harness = Some("Claude Code".to_owned());
    let completed = RuntimeEvent::ToolCallCompleted {
        call: ToolCallId::new("bridge-agent"),
        name: "agent".to_owned(),
        is_error: false,
    };
    for payload in [
        RuntimeEvent::TurnStarted,
        RuntimeEvent::ExternalSessionStarted {
            session: "cli-session".to_owned(),
        },
        RuntimeEvent::ExternalText {
            text: "before".to_owned(),
        },
        RuntimeEvent::ExternalToolInvoked {
            id: "cli-read".to_owned(),
            name: "Read".to_owned(),
            detail: serde_json::json!({"file_path":"src/main.rs"}),
        },
        RuntimeEvent::ExternalToolCompleted {
            id: "cli-read".to_owned(),
            ok: true,
            detail: serde_json::json!("fn main() {}"),
        },
        RuntimeEvent::ExternalText {
            text: "between".to_owned(),
        },
        bridged_agent_request(),
        completed.clone(),
        RuntimeEvent::ExternalText {
            text: "after".to_owned(),
        },
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ] {
        app.apply(&event(payload));
    }
    app.set_tool_result_preview("bridge-agent", "children: []");
    let mut native = App::new("native-model", "/repo");
    native.apply(&event(bridged_agent_request()));
    native.apply(&event(completed));
    native.set_tool_result_preview("bridge-agent", "children: []");
    for width in [44, 100] {
        let lines = transcript_lines(&app, Theme::new().without_color(), width);
        let native_lines = transcript_lines(&native, Theme::new().without_color(), width);
        let lines = lines.iter().map(ToString::to_string).collect::<Vec<_>>();
        let native_lines = native_lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let text = lines.join("\n");
        assert_eq!(
            text.matches("Read(src/main.rs · agent)").count(),
            1,
            "{text}"
        );
        assert_eq!(text.matches("children: []").count(), 1, "{text}");
        assert!(text.contains("agent"), "{text}");
        assert!(
            lines
                .windows(native_lines.len())
                .any(|rows| rows == native_lines),
            "{text}"
        );
        let positions = [
            "before",
            "Read(src/main.rs · agent)",
            "between",
            "children: []",
            "after",
        ]
        .map(|fragment| text.find(fragment).expect("ordered transcript fragment"));
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
        let screen = render(&app, width, 32, Theme::new().without_color());
        assert!(screen.contains("cli/claude-code/sonnet"), "{screen}");
        assert!(!screen.contains("mcp__smith__agent"), "{screen}");
    }
}

#[tokio::test]
async fn bridged_spawn_approval_matches_native_action_prompt() {
    let mut harness = App::new("cli/claude-code/sonnet", "/repo");
    harness.status.harness = Some("Claude Code".to_owned());
    harness.apply(&event(RuntimeEvent::TurnStarted));
    harness.apply(&event(RuntimeEvent::ExternalSessionStarted {
        session: "cli-session".to_owned(),
    }));
    harness.apply(&event(RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("bridge-agent"),
        name: "agent".to_owned(),
        argument_keys: vec!["action".to_owned(), "task".to_owned()],
        argument_fingerprint: agent_runtime_registry::Fingerprint::of("spawn"),
        arguments: Some(
            serde_json::json!({"action":"spawn","task":"Make exactly one edit in src/main.rs"}),
        ),
    }));
    let mut native = App::new("native-model", "/repo");
    for app in [&mut harness, &mut native] {
        app.present_approval(
            child_agent_approval_prompt("delegation.spawn", child_agent_spawn_material()).await,
        );
    }
    for width in [44, 100] {
        let bridged_screen = render(&harness, width, 32, Theme::new().without_color());
        let native_screen = render(&native, width, 32, Theme::new().without_color());
        let bridged = pane_rows(&bridged_screen, "Start a child agent");
        assert_eq!(
            bridged,
            pane_rows(&native_screen, "Start a child agent")
        );
        assert_eq!(bridged[0], "Make exactly one edit in src/main.rs");
        insta_like(
            &approval_screen_words(&bridged_screen),
            &["Warning: the child acts on its own", "1. Yes", "3. No (esc)"],
        );
    }
}

#[test]
fn bridge_diagnostic_renders_without_a_failed_turn() {
    let mut app = App::new("cli/claude-code/sonnet", "/repo");
    app.status.harness = Some("Claude Code".to_owned());
    for payload in [
        RuntimeEvent::TurnStarted,
        RuntimeEvent::ExternalText {
            text: "answer".to_owned(),
        },
        RuntimeEvent::Error {
            error: agent_runtime_core::error::RuntimeError::config(
                "Claude Code did not load Smith's tools",
            ),
        },
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ] {
        app.apply(&event(payload));
    }
    let screen = render(&app, 100, 24, Theme::new().without_color());
    insta_like(
        &screen,
        &["answer", "Claude Code did not load Smith's tools"],
    );
    assert_eq!(app.turn_summary.as_deref(), Some("Worked"));
    assert!(!screen.contains("Failed"), "{screen}");
}
