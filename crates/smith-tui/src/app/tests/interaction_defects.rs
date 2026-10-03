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
    for control in ["y  Yes", "a  Yes", "n  No (esc)"] {
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

#[tokio::test]
async fn enter_never_answers_an_approval_before_or_after_expanding_and_settling() {
    let mut app = app();
    let clock = prompt_clock(&mut app);
    app.composer.replace("keep the draft");
    let (prompt, decision) =
        pending_prompt_with("shell", serde_json::json!({"command": "build"})).await;
    app.present_approval(prompt);
    for expanded in [false, true] {
        if expanded {
            app.on_key(ctrl('o'));
        }
        for elapsed in [0, 500] {
            clock.advance(elapsed);
            assert_eq!(app.on_key(key(KeyCode::Enter)), None);
            assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
            assert!(!decision.is_finished());
            assert_eq!(app.composer.text(), "keep the draft");
        }
    }
    clock.advance(500);
    app.on_key(key(KeyCode::Esc));
    assert!(matches!(
        decision.await.expect("explicit denial"),
        ApprovalDecision::Deny { .. }
    ));
}

#[tokio::test]
async fn approval_scroll_keys_keep_the_transcript_available_and_the_fifo_unanswered() {
    for settled in [false, true] {
        let mut app = app();
        let clock = prompt_clock(&mut app);
        app.composer.replace("keep the draft");
        app.sync_scroll_limit(40);
        let (first, first_decision) =
            pending_prompt_with("shell", serde_json::json!({"command": "first"})).await;
        let (second, second_decision) =
            pending_prompt_with("shell", serde_json::json!({"command": "second"})).await;
        let identity = first.prepared().fingerprint().clone();
        app.present_approval(first);
        app.present_approval(second);
        if settled {
            clock.advance(500);
        }
        for (event, offset) in [
            (key(KeyCode::PageUp), 10),
            (key(KeyCode::PageDown), 0),
            (key(KeyCode::Home), 40),
            (key(KeyCode::End), 0),
            (key(KeyCode::PageUp), 10),
            (ctrl('l'), 0),
        ] {
            assert_eq!(app.on_key(event), None);
            assert_eq!(app.scroll_back, offset, "{event:?}");
            assert_eq!(app.following, offset == 0);
            assert_eq!(app.composer.text(), "keep the draft");
            assert_eq!(app.pending_approval_count(), 2);
            match &app.overlay {
                Some(Overlay::Approval { prompt, .. }) => {
                    assert_eq!(prompt.prepared().fingerprint(), &identity)
                }
                other => panic!("navigation replaced the prompt: {other:?}"),
            }
            assert!(!first_decision.is_finished());
            assert!(!second_decision.is_finished());
        }
        clock.advance(500);
        app.on_key(key(KeyCode::Esc));
        assert!(matches!(
            first_decision.await.expect("first denied"),
            ApprovalDecision::Deny { .. }
        ));
        clock.advance(500);
        app.on_key(key(KeyCode::Esc));
        assert!(matches!(
            second_decision.await.expect("second denied"),
            ApprovalDecision::Deny { .. }
        ));
    }
}

#[tokio::test]
async fn mouse_wheel_scrolls_under_an_approval_without_answering_or_touching_the_draft() {
    let mut app = app();
    let clock = prompt_clock(&mut app);
    app.composer.replace("keep the draft");
    app.sync_scroll_limit(40);
    let (prompt, decision) =
        pending_prompt_with("shell", serde_json::json!({"command": "build"})).await;
    app.present_approval(prompt);
    for settled in [false, true] {
        if settled {
            clock.advance(500);
        }
        for (kind, offset) in [
            (MouseEventKind::ScrollUp, 3),
            (MouseEventKind::ScrollDown, 0),
        ] {
            assert_eq!(app.on_mouse(mouse(kind, 20, 4)), MouseOutcome::Redraw);
            assert_eq!(app.scroll_back, offset);
            assert_eq!(app.following, offset == 0);
            assert_eq!(app.composer.text(), "keep the draft");
            assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
            assert!(!decision.is_finished());
        }
    }
    app.on_key(key(KeyCode::Esc));
    assert!(matches!(
        decision.await.expect("explicit denial"),
        ApprovalDecision::Deny { .. }
    ));
}

#[tokio::test]
async fn scrolling_during_the_quiet_window_still_restarts_the_decision_guard() {
    let mut app = app();
    let clock = prompt_clock(&mut app);
    app.composer.replace("keep the draft");
    app.sync_scroll_limit(40);
    let (prompt, decision) =
        pending_prompt_with("shell", serde_json::json!({"command": "build"})).await;
    app.present_approval(prompt);
    clock.advance(499);
    app.on_key(key(KeyCode::PageUp));
    assert_eq!(app.scroll_back, 10);
    clock.advance(499);
    app.on_key(key(KeyCode::Char('y')));
    assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
    assert!(!decision.is_finished());
    assert_eq!(app.composer.text(), "keep the draft");
    clock.advance(500);
    app.on_key(key(KeyCode::Char('y')));
    assert_eq!(
        decision.await.expect("deliberate approval"),
        ApprovalDecision::Allow
    );
}
