use super::*;

#[test]
fn expanded_result_detail_is_bounded_without_discarding_the_folded_tail() {
    let mut transcript = Transcript::new();
    transcript.push_tool_call("c", "shell", None, &[]);
    let output = "safe output\n".repeat(MAX_LOCAL_RESULT_LINES + 10);
    transcript.set_tool_result_preview("c", output);
    let Block::Tool {
        result_preview: Some(detail),
        ..
    } = &transcript.blocks()[0]
    else {
        panic!("tool detail");
    };
    assert!(detail.lines().count() <= MAX_LOCAL_RESULT_LINES + 1);
    assert!(detail.ends_with("[local result truncated at the display limit]"));
    transcript.set_tool_result_preview("c", "x".repeat(MAX_LOCAL_RESULT_BYTES + 10));
    let Block::Tool {
        result_preview: Some(detail),
        ..
    } = &transcript.blocks()[0]
    else {
        panic!("tool detail");
    };
    assert!(detail.len() <= MAX_LOCAL_RESULT_BYTES + 64);
    assert!(detail.ends_with("[local result truncated at the display limit]"));
}

#[test]
fn a_rejected_shortcut_does_not_take_the_next_admitted_calls_identity() {
    let mut transcript = Transcript::new();
    let rejected = transcript.push_shell_shortcut("rejected command");
    let _ = transcript.finish_shell_shortcut(rejected, None, true, "admission rejected");
    let accepted = transcript.push_shell_shortcut("accepted command");
    transcript.bind_shell_shortcut(accepted, "accepted-call");
    transcript.push_tool_call("accepted-call", "shell", None, &[]);
    assert_eq!(transcript.blocks().len(), 2);
    assert!(
        matches!(&transcript.blocks()[0], Block::Tool { status: ToolStatus::Failed, call_id, .. } if call_id.is_empty())
    );
    assert!(
        matches!(&transcript.blocks()[1], Block::Tool { user_command: Some(command), call_id, .. } if command == "accepted command" && call_id == "accepted-call")
    );
}

#[test]
fn late_admission_failures_settle_only_their_own_echo() {
    let mut transcript = Transcript::new();
    let rejected = transcript.push_shell_shortcut("rejected");
    let accepted = transcript.push_shell_shortcut("accepted");
    transcript.bind_shell_shortcut(accepted, "accepted-call");
    let _ = transcript.finish_shell_shortcut(rejected, None, true, "requires an idle session");
    assert!(
        matches!(&transcript.blocks()[0], Block::Tool { status: ToolStatus::Failed, result_preview: Some(output), .. } if output == "requires an idle session")
    );
    assert!(
        matches!(&transcript.blocks()[1], Block::Tool { status: ToolStatus::Running, call_id, .. } if call_id == "accepted-call")
    );
    transcript.replace_from_history(&[]);
    let next = transcript.push_shell_shortcut("next");
    assert_ne!(next, accepted);
    let _ = transcript.finish_shell_shortcut(accepted, Some("accepted-call"), false, "late output");
    assert!(matches!(
        &transcript.blocks()[0],
        Block::Tool {
            status: ToolStatus::Running,
            result_preview: None,
            ..
        }
    ));
}

#[test]
fn a_result_arriving_before_the_runtime_event_keeps_the_single_user_echo() {
    let mut transcript = Transcript::new();
    let echo = transcript.push_shell_shortcut("ls -la");
    transcript.bind_shell_shortcut(echo, "local");
    transcript.complete_tool_call("local", ToolStatus::Ok);
    transcript.set_tool_result_preview("local", "files");
    transcript.push_tool_call("local", "shell", None, &["command".into()]);
    transcript.bind_shell_shortcut(echo, "local");
    assert_eq!(transcript.blocks().len(), 1);
    assert!(
        matches!(&transcript.blocks()[0], Block::Tool { user_command: Some(command), status: ToolStatus::Ok, result_preview: Some(output), .. } if command == "ls -la" && output == "files")
    );
}
