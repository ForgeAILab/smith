//! Normalization against output recorded from the installed CLIs.

use super::{BridgeInvocation, CliAgentKind, ExternalAgentEvent, TurnState, normalize};

const CLAUDE_BESIDE: &str = include_str!(
    "../../tests/fixtures/cli_bridge/claude-code-2.1.296/stream/mcp-call-beside-own-tools.jsonl"
);
const CLAUDE_CALL: &str =
    include_str!("../../tests/fixtures/cli_bridge/claude-code-2.1.296/stream/mcp-call.jsonl");
const CLAUDE_TIMEOUT: &str = include_str!(
    "../../tests/fixtures/cli_bridge/claude-code-2.1.296/stream/mcp-call-timed-out.jsonl"
);
const CLAUDE_FAILED: &str =
    include_str!("../../tests/fixtures/cli_bridge/claude-code-2.1.296/stream/failed-turn.jsonl");
const CODEX_BESIDE: &str = include_str!(
    "../../tests/fixtures/cli_bridge/codex-0.162.1/stream/mcp-call-beside-own-tool.jsonl"
);
const CODEX_FAILED: &str =
    include_str!("../../tests/fixtures/cli_bridge/codex-0.162.1/stream/failed-turn.jsonl");

fn recorded_events(
    kind: CliAgentKind,
    fixture: &str,
    bridge_server: Option<&str>,
) -> Vec<ExternalAgentEvent> {
    let bridge = bridge_server.map(|server| BridgeInvocation {
        server: server.to_owned(),
        relay_executable: "/usr/local/bin/smith".into(),
        socket: "/tmp/smith.sock".into(),
    });
    let mut state = TurnState::for_bridge(bridge.as_ref());
    fixture
        .lines()
        .flat_map(|line| {
            let value = serde_json::from_str(line).expect("a recorded CLI JSON line");
            normalize(kind, &value, &mut state)
        })
        .collect()
}

fn tool_events(events: &[ExternalAgentEvent]) -> Vec<&ExternalAgentEvent> {
    events
        .iter()
        .filter(|event| {
            matches!(
                event,
                ExternalAgentEvent::ToolInvoked { .. } | ExternalAgentEvent::ToolCompleted { .. }
            )
        })
        .collect()
}

fn assert_tools(events: &[ExternalAgentEvent], expected: &[(&str, bool)]) {
    let tools = tool_events(events);
    assert_eq!(tools.len(), expected.len() * 2, "{tools:?}");
    for (pair, (expected_name, expected_ok)) in tools.chunks_exact(2).zip(expected) {
        match (pair[0], pair[1]) {
            (
                ExternalAgentEvent::ToolInvoked { id, name, .. },
                ExternalAgentEvent::ToolCompleted {
                    id: completed_id,
                    ok,
                    ..
                },
            ) => {
                assert_eq!(id, completed_id);
                assert_eq!(name.as_str(), *expected_name);
                assert_eq!(ok, expected_ok);
            }
            pair => panic!("each observed call has its own outcome: {pair:?}"),
        }
    }
    assert!(matches!(events.last(), Some(ExternalAgentEvent::Completed)));
}

#[test]
fn claude_bridge_echo_is_suppressed_beside_its_own_tools_and_outcomes() {
    let events = recorded_events(CliAgentKind::ClaudeCode, CLAUDE_BESIDE, Some("spike"));
    assert_tools(
        &events,
        &[
            ("ToolSearch", true),
            ("Bash", true),
            ("Write", false),
            ("Read", true),
        ],
    );
    assert!(events.iter().any(|event| matches!(
        event,
        ExternalAgentEvent::ToolCompleted { detail, .. } if detail == "hi-from-bash"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ExternalAgentEvent::ToolCompleted { detail, .. } if detail == "1\tseed-content-42\n2\t"
    )));
}

#[test]
fn claude_suppresses_only_the_configured_bridge_server() {
    let suppressed = recorded_events(CliAgentKind::ClaudeCode, CLAUDE_CALL, Some("spike"));
    assert_tools(&suppressed, &[]);

    let kept = recorded_events(CliAgentKind::ClaudeCode, CLAUDE_CALL, Some("other"));
    assert_tools(&kept, &[("mcp__spike__fast_echo", true)]);
    assert!(kept.iter().any(|event| matches!(
        event,
        ExternalAgentEvent::ToolCompleted { detail, .. }
            if *detail == serde_json::json!([{"type": "text", "text": "ECHO:hello"}])
    )));
}

#[test]
fn claude_suppresses_the_failed_bridge_echo_too() {
    let suppressed = recorded_events(CliAgentKind::ClaudeCode, CLAUDE_TIMEOUT, Some("spike"));
    assert_tools(&suppressed, &[("ToolSearch", true)]);

    let kept = recorded_events(CliAgentKind::ClaudeCode, CLAUDE_TIMEOUT, Some("other"));
    assert_tools(
        &kept,
        &[("ToolSearch", true), ("mcp__spike__slow_wait", false)],
    );
    assert!(kept.iter().any(|event| matches!(
        event,
        ExternalAgentEvent::ToolCompleted { ok: false, detail, .. }
            if detail == "MCP server \"spike\" tool \"slow_wait\" timed out after 4s"
    )));
}

#[test]
fn codex_suppresses_the_bridge_echo_and_keeps_its_own_command() {
    let suppressed = recorded_events(CliAgentKind::Codex, CODEX_BESIDE, Some("spike"));
    assert_tools(&suppressed, &[("command_execution", true)]);
    assert!(suppressed.iter().any(|event| matches!(
        event,
        ExternalAgentEvent::ToolCompleted { detail, .. } if detail == "hi-from-shell\n"
    )));

    let kept = recorded_events(CliAgentKind::Codex, CODEX_BESIDE, Some("other"));
    assert_tools(
        &kept,
        &[("command_execution", true), ("mcp_tool_call", true)],
    );
    assert!(kept.iter().any(|event| matches!(
        event,
        ExternalAgentEvent::ToolInvoked { name, detail, .. }
            if name == "mcp_tool_call" && detail["server"] == "spike" && detail["tool"] == "fast_echo"
    )));
}

#[test]
fn turns_without_a_bridge_keep_both_clis_mcp_observations() {
    let claude = recorded_events(CliAgentKind::ClaudeCode, CLAUDE_BESIDE, None);
    assert_tools(
        &claude,
        &[
            ("ToolSearch", true),
            ("mcp__spike__fast_echo", true),
            ("Bash", true),
            ("Write", false),
            ("Read", true),
        ],
    );
    assert_eq!(
        claude,
        recorded_events(CliAgentKind::ClaudeCode, CLAUDE_BESIDE, Some("other"))
    );

    let codex = recorded_events(CliAgentKind::Codex, CODEX_BESIDE, None);
    assert_tools(
        &codex,
        &[("command_execution", true), ("mcp_tool_call", true)],
    );
    assert_eq!(
        codex,
        recorded_events(CliAgentKind::Codex, CODEX_BESIDE, Some("other"))
    );
}

#[test]
fn claude_recorded_failed_turn_preserves_the_reason() {
    let events = recorded_events(CliAgentKind::ClaudeCode, CLAUDE_FAILED, Some("spike"));
    assert!(matches!(
        events.last(),
        Some(ExternalAgentEvent::Failed { message })
            if message == "There's an issue with the selected model (no-such-model-xyz). It may not exist or you may not have access to it. Run --model to pick a different model."
    ));
    assert_eq!(events.iter().filter(|event| event.is_terminal()).count(), 1);
    assert!(tool_events(&events).is_empty());
}

#[test]
fn codex_recorded_failed_turn_preserves_the_reason() {
    let events = recorded_events(CliAgentKind::Codex, CODEX_FAILED, Some("spike"));
    assert!(matches!(
        events.last(),
        Some(ExternalAgentEvent::Failed { message })
            if message == r#"{"type":"error","status":400,"error":{"type":"invalid_request_error","message":"The 'no-such-model-xyz' model is not supported when using Codex with a ChatGPT account."}}"#
    ));
    assert_eq!(events.iter().filter(|event| event.is_terminal()).count(), 1);
    assert!(tool_events(&events).is_empty());
}
