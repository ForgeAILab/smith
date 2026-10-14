use super::*;

#[test]
fn mixed_harness_turn_preserves_text_order_and_one_row_per_owner() {
    let mut app = App::new("cli/claude-code/sonnet", "/repo");
    app.status.harness = Some("Claude Code".to_owned());
    let requested = RuntimeEvent::ToolCallRequested {
        call: ToolCallId::new("bridged-agent"),
        name: "agent".to_owned(),
        argument_keys: vec!["action".to_owned()],
        argument_fingerprint: fingerprint("list"),
        arguments: Some(serde_json::json!({"action":"list"})),
    };
    let completed = RuntimeEvent::ToolCallCompleted {
        call: ToolCallId::new("bridged-agent"),
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
        requested.clone(),
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
    let mut native = App::new("native-model", "/repo");
    native.apply(&event(requested));
    native.apply(&event(completed));
    let blocks = app.transcript.blocks();
    assert_eq!(blocks.len(), 5);
    assert!(matches!(&blocks[0], Block::Assistant { text, open: false } if text == "before"));
    assert!(
        matches!(&blocks[1], Block::Tool { name, status: ToolStatus::Ok, enrichment, result_preview: Some(result), .. } if name == "Read" && enrichment == &["agent"] && result == "fn main() {}")
    );
    assert!(matches!(&blocks[2], Block::Assistant { text, open: false } if text == "between"));
    assert_eq!(blocks[3], native.transcript.blocks()[0]);
    assert!(matches!(&blocks[4], Block::Assistant { text, open: false } if text == "after"));
    assert_eq!(app.status.harness.as_deref(), Some("Claude Code"));
    assert_eq!(app.status.activity, Activity::Idle);
    assert_eq!(app.turn_summary.as_deref(), Some("Worked"));
}

#[test]
fn bridge_diagnostic_is_visible_and_successful_terminal_stays_completed() {
    let mut app = app();
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(RuntimeEvent::ExternalText {
        text: "answer".to_owned(),
    }));
    app.apply(&event(RuntimeEvent::Error {
        error: RuntimeError::config("Claude Code did not load Smith's tools for this turn"),
    }));
    app.apply(&event(RuntimeEvent::TurnCompleted {
        finish: TurnFinish::Completed,
        visible_output: true,
    }));
    assert!(
        matches!(app.transcript.blocks().last(), Some(Block::Error { message }) if message.contains("Claude Code did not load Smith's tools"))
    );
    assert_eq!(app.status.activity, Activity::Idle);
    assert_eq!(app.turn_summary.as_deref(), Some("Worked"));
    assert!(
        !app.transcript
            .blocks()
            .iter()
            .any(|block| matches!(block, Block::Notice { text, .. } if text.starts_with("Failed")))
    );
}
