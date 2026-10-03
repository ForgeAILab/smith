// Action-specific confirmation copy and prepared-action rendering.

#[test]
fn redo_confirmation_body_and_hint_offer_applying_the_redo() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.confirm_redo(recovery_preview("--- before\n+++ after\n-old\n+new"));
    let screen = render(&app, 120, 30, Theme::new().without_color());
    assert!(screen.contains("redo last exact Smith turn"), "{screen}");
    assert!(screen.contains("complete"), "{screen}");
    assert!(screen.contains("forward"), "{screen}");
    assert!(screen.contains("patch."), "{screen}");
    assert!(screen.contains("y apply redo"), "{screen}");
    assert_eq!(screen.matches("y apply redo").count(), 2, "{screen}");
    assert!(!screen.contains("apply undo"), "{screen}");
    assert!(!screen.contains("reverse patch"), "{screen}");
    assert_eq!(
        overlay_hint(&app).as_deref(),
        Some("y apply redo · n/esc cancel")
    );
}

#[test]
fn mcp_trust_confirmation_names_the_server_and_the_authority_it_permits() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.confirm_mcp_trust("docs", "command: docs-server\nidentity: reviewed-server");
    let screen = render(&app, 120, 30, Theme::new().without_color());
    assert!(screen.contains("Trust MCP server docs?"), "{screen}");
    assert!(screen.contains("launch this server"), "{screen}");
    assert!(screen.contains("declared"), "{screen}");
    assert!(screen.contains("tools."), "{screen}");
    assert!(screen.contains("command: docs-server"), "{screen}");
    assert!(screen.contains("y trust and connect"), "{screen}");
    assert_eq!(screen.matches("y trust and connect").count(), 2, "{screen}");
    for wrong_action in ["reverse patch", "undo", "revert"] {
        assert!(!screen.contains(wrong_action), "{screen}");
    }
    assert_eq!(
        overlay_hint(&app).as_deref(),
        Some("y trust and connect · n/esc leave untrusted")
    );
}

#[test]
fn skill_trust_confirmation_names_the_skill_and_the_instructions_it_activates() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.confirm_skill_trust("deploy", "path: skills/deploy\nidentity: reviewed-skill");
    let screen = render(&app, 120, 30, Theme::new().without_color());
    assert!(screen.contains("Trust project skill deploy?"), "{screen}");
    assert!(screen.contains("activate this skill's"), "{screen}");
    assert!(screen.contains("project"), "{screen}");
    assert!(screen.contains("instructions."), "{screen}");
    assert!(screen.contains("path: skills/deploy"), "{screen}");
    assert!(screen.contains("y trust and activate"), "{screen}");
    assert_eq!(
        screen.matches("y trust and activate").count(),
        2,
        "{screen}"
    );
    for wrong_action in ["reverse patch", "undo", "revert"] {
        assert!(!screen.contains(wrong_action), "{screen}");
    }
    assert_eq!(
        overlay_hint(&app).as_deref(),
        Some("y trust and activate · n/esc leave withheld")
    );
}

#[tokio::test]
async fn shell_approval_keeps_multiline_commands_and_one_warning() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.present_approval(approval_evidence_prompt("ls -la\npwd", false).await);
    let screen = render(&app, 100, 32, Theme::new().without_color());
    let rows = screen.lines().collect::<Vec<_>>();
    let command = rows
        .iter()
        .position(|row| row.contains("ls -la"))
        .expect("command row");
    assert!(rows[command + 1].contains("pwd"), "{screen}");
    assert_eq!(screen.matches("Warning:").count(), 1, "{screen}");
    assert!(!screen.contains("Host access:"), "{screen}");
    assert!(!screen.contains("ls -lapwd"), "{screen}");
}
