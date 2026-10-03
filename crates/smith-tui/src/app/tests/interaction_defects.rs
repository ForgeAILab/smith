// Consequential prompt and command-picker regression tests.

fn prompt_clock(app: &mut App) -> std::sync::Arc<agent_runtime_testkit::clock::ManualClock> {
    let clock = agent_runtime_testkit::clock::ManualClock::shared(1_000);
    app.prompt_input_guard.clock = clock.clone();
    clock
}

fn elapse_prompt_guard(app: &mut App) {
    let ready_at = app
        .prompt_input_guard
        .quiet_until
        .expect("a consequential prompt has a quiet window");
    app.prompt_input_guard.clock =
        agent_runtime_testkit::clock::ManualClock::shared(ready_at.as_millis());
}

#[tokio::test]
async fn typing_through_an_approval_preserves_the_draft_and_requires_a_quiet_window() {
    let mut app = app();
    let clock = prompt_clock(&mut app);
    app.apply(&event(RuntimeEvent::TurnStarted));
    type_text(&mut app, "can you ");
    let (prompt, decision) =
        pending_prompt_with("shell", serde_json::json!({"command": "build"})).await;
    app.present_approval(prompt);

    for character in "also check again".chars() {
        clock.advance(100);
        assert_eq!(app.reduce_key(key(KeyCode::Char(character))), None);
        assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
        assert_eq!(app.composer.text(), "can you ");
        assert!(!decision.is_finished());
    }
    app.on_paste("a pasted decision is still text");
    assert_eq!(app.composer.text(), "can you ");

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30))
        .expect("a test terminal");
    terminal
        .draw(|frame| crate::render::draw(frame, &app, crate::theme::Theme::new().without_color()))
        .expect("an approval frame");
    let buffer = terminal.backend().buffer();
    let screen = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    for control in ["y allow once", "a allow", "n deny"] {
        assert!(screen.contains(control), "{screen}");
    }

    clock.advance(499);
    assert_eq!(app.on_key(key(KeyCode::Char('a'))), None);
    assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
    assert!(!decision.is_finished());

    clock.advance(500);
    assert_eq!(app.on_key(key(KeyCode::Char('y'))), None);
    assert!(app.overlay.is_none());
    assert_eq!(
        decision.await.expect("deliberate decision"),
        ApprovalDecision::Allow
    );
    assert_eq!(app.composer.text(), "can you ");
    assert!(!app.transcript.blocks().iter().any(|block| matches!(
        block,
        Block::Notice { source, text }
            if source == "approval" && text.contains("for the session")
    )));
}

#[tokio::test]
async fn every_approval_decision_targets_only_the_visible_prompt_after_its_quiet_window() {
    for code in [
        KeyCode::Char('y'),
        KeyCode::Char('a'),
        KeyCode::Char('n'),
        KeyCode::Esc,
    ] {
        let mut app = app();
        let clock = prompt_clock(&mut app);
        app.composer.replace("keep this draft");
        let (first, first_decision) =
            pending_prompt_with("shell", serde_json::json!({"command": "first"})).await;
        let (second, second_decision) =
            pending_prompt_with("patch", serde_json::json!({"path": "second"})).await;
        app.present_approval(first);
        clock.advance(400);
        app.present_approval(second);
        clock.advance(100);

        assert_eq!(app.on_key(key(code)), None);
        match &app.overlay {
            Some(Overlay::Approval { prompt, .. }) => assert_eq!(prompt.tool(), "patch"),
            other => panic!("expected the next prompt, got {other:?}"),
        }
        let resolved = first_decision.await.expect("first decision");
        if matches!(code, KeyCode::Char('y' | 'a')) {
            assert_eq!(resolved, ApprovalDecision::Allow);
        } else {
            assert!(matches!(resolved, ApprovalDecision::Deny { .. }));
        }
        let session_notices = app
            .transcript
            .blocks()
            .iter()
            .filter(|block| {
                matches!(
                    block,
                    Block::Notice { source, text }
                        if source == "approval" && text.contains("for the session")
                )
            })
            .count();
        assert_eq!(session_notices, usize::from(code == KeyCode::Char('a')));

        assert_eq!(app.on_key(key(code)), None);
        assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
        assert!(!second_decision.is_finished());
        assert_eq!(app.composer.text(), "keep this draft");

        clock.advance(500);
        app.on_key(key(KeyCode::Esc));
        assert!(matches!(
            second_decision.await.expect("second decision"),
            ApprovalDecision::Deny { .. }
        ));
        assert!(app.overlay.is_none());
        assert_eq!(app.pending_approval_count(), 0);
    }
}

#[test]
fn recovery_and_trust_confirmations_ignore_typing_until_the_quiet_boundary() {
    for dialog in ["undo", "redo", "revert", "mcp", "skill"] {
        for code in [KeyCode::Char('y'), KeyCode::Char('n'), KeyCode::Esc] {
            let mut app = app();
            let clock = prompt_clock(&mut app);
            app.composer.replace("keep this draft");
            match dialog {
                "undo" => app.confirm_undo(recovery_preview("reverse patch")),
                "redo" => app.confirm_redo(recovery_preview("forward patch")),
                "revert" => app.confirm_revert(revert_preview("file.txt", "exact-preview", "reverse patch")),
                "mcp" => app.confirm_mcp_trust("docs", "command: docs-server"),
                "skill" => app.confirm_skill_trust("deploy", "path: skills/deploy"),
                _ => unreachable!(),
            }
            for early in [KeyCode::Char('a'), code] {
                clock.advance(499);
                assert_eq!(app.on_key(key(early)), None, "{dialog}: {early:?}");
                assert!(app.overlay.is_some(), "{dialog}: {early:?}");
                assert_eq!(app.composer.text(), "keep this draft");
            }
            clock.advance(500);
            let allowed = code == KeyCode::Char('y');
            let expected = match (dialog, allowed) {
                ("undo", true) => Some(Action::ApplyUndo),
                ("undo", false) => Some(Action::CancelUndo),
                ("redo", true) => Some(Action::ApplyRedo),
                ("redo", false) => Some(Action::CancelRedo),
                ("revert", true) => Some(Action::ApplyRevert {
                    scope: "file.txt".into(),
                    fingerprint: "exact-preview".into(),
                }),
                ("revert", false) => Some(Action::CancelRevert {
                    scope: "file.txt".into(),
                    fingerprint: "exact-preview".into(),
                }),
                ("mcp", true) => Some(Action::TrustMcpServer {
                    server: "docs".into(),
                }),
                ("skill", true) => Some(Action::TrustSkill {
                    skill: "deploy".into(),
                }),
                _ => None,
            };
            assert_eq!(app.on_key(key(code)), expected, "{dialog}: {code:?}");
            assert!(app.overlay.is_none());
            assert_eq!(app.composer.text(), "keep this draft");
        }
    }
}

#[tokio::test]
async fn restoring_an_approval_after_exit_requires_a_new_quiet_window() {
    let mut app = app();
    let clock = prompt_clock(&mut app);
    app.present_approval(prompt("shell").await);
    clock.advance(500);
    assert_eq!(app.request_exit(), None);
    app.on_key(key(KeyCode::Char('n')));
    assert_eq!(app.on_key(key(KeyCode::Char('a'))), None);
    assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
    clock.advance(500);
    app.on_key(key(KeyCode::Esc));
    assert!(app.overlay.is_none());
}

#[test]
fn cancelling_a_command_picker_clears_the_composer_before_the_next_command() {
    for command in [
        "/model",
        "/provider",
        "/profile",
        "/resume",
        "/connect",
        "/disconnect",
        "/think",
        "/effort",
        "/account",
        "/eff",
    ] {
        let mut app = app();
        type_text(&mut app, command);
        assert_eq!(app.on_key(key(KeyCode::Enter)), None, "{command}");
        assert!(
            matches!(app.overlay, Some(Overlay::ResourcePicker { .. })),
            "{command}"
        );
        assert_eq!(app.on_key(key(KeyCode::Esc)), None);
        assert!(app.overlay.is_none());
        assert!(app.composer.is_empty(), "{command}");
        type_text(&mut app, "/status");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Command(HostCommand::Status))
        );
        assert!(app.composer.is_empty());
    }
}

#[test]
fn cancelling_a_reference_picker_keeps_the_existing_draft() {
    let mut app = agent_first_app();
    type_text(&mut app, "review ");
    app.on_key(key(KeyCode::Char('@')));
    type_text(&mut app, "src");
    assert_eq!(app.on_key(key(KeyCode::Esc)), None);
    assert!(app.overlay.is_none());
    assert_eq!(app.composer.text(), "review ");
}

#[tokio::test]
async fn rotation_controls_ignore_typing_until_the_offer_has_a_quiet_window() {
    for code in [
        KeyCode::Char('y'),
        KeyCode::Char('n'),
        KeyCode::Esc,
        KeyCode::Char('3'),
    ] {
        let mut app = app();
        let clock = prompt_clock(&mut app);
        app.composer.replace("keep drafting");
        let (policy, mut requests) = InteractiveRotation::new(1);
        let request = offer(vec![
            member(1, "keychain:smith/work"),
            member(2, "keychain:smith/spare"),
        ]);
        let pending = tokio::spawn(async move { policy.decide(&request).await });
        app.present_rotation(requests.recv().await.expect("a rotation offer"));

        clock.advance(499);
        assert_eq!(app.on_key(key(code)), None);
        assert!(matches!(app.overlay, Some(Overlay::RotationConfirm { .. })));
        assert!(!pending.is_finished());
        clock.advance(499);
        assert_eq!(app.on_key(key(KeyCode::Char('a'))), None);
        assert!(matches!(app.overlay, Some(Overlay::RotationConfirm { .. })));

        clock.advance(500);
        app.on_key(key(code));
        let expected = match code {
            KeyCode::Char('y') => RotationDecision::Switch { position: 1 },
            KeyCode::Char('3') => RotationDecision::Switch { position: 2 },
            _ => RotationDecision::Decline,
        };
        assert_eq!(pending.await.expect("rotation decision"), expected);
        assert!(app.overlay.is_none());
        assert_eq!(app.composer.text(), "keep drafting");
    }
}
