// prompts behavior tests.

    #[test]
    fn a_double_slash_escapes_to_a_literal_prompt() {
        let mut app = app();
        type_text(&mut app, "//help me understand slashes");
        let submission = expect_whole_submission(app.on_key(key(KeyCode::Enter)));
        assert_eq!(submission.display_text(), "/help me understand slashes");
        app.whole_turn_dispatched(TurnId::new("turn-1"), &submission);
        assert_eq!(
            app.transcript.blocks()[0],
            Block::User {
                text: "/help me understand slashes".into()
            }
        );
    }

    #[test]
    fn slash_quit_follows_the_exit_policy() {
        let mut idle = app();
        type_text(&mut idle, "/quit");
        assert_eq!(idle.on_key(key(KeyCode::Enter)), Some(Action::Quit));

        let mut busy = app();
        busy.apply(&event(RuntimeEvent::TurnStarted));
        type_text(&mut busy, "/quit");
        assert_eq!(busy.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(busy.overlay, Some(Overlay::ExitConfirm { .. })));
    }

    #[test]
    fn first_ctrl_c_clears_and_stashes_the_draft_without_quitting() {
        let mut idle = app();
        type_text(&mut idle, "recover this draft");
        assert_eq!(idle.on_key(ctrl('c')), None);
        assert!(idle.composer.is_empty());
        assert!(!idle.should_quit);

        assert_eq!(idle.on_key(key(KeyCode::Up)), None);
        assert_eq!(idle.composer.text(), "recover this draft");
        assert!(!idle.should_quit);

        let mut busy = app();
        busy.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(busy.on_key(ctrl('c')), None);
        assert!(busy.overlay.is_none());
        assert!(!busy.should_quit);
        assert!(busy.is_busy());
    }

    #[test]
    fn a_non_ctrl_c_key_disarms_the_double_press_exit() {
        let mut app = app();
        assert_eq!(app.on_key(ctrl('c')), None);
        assert_eq!(app.on_key(key(KeyCode::Char('x'))), None);
        assert_eq!(app.on_key(ctrl('c')), None);
        assert!(!app.should_quit);
        assert!(app.composer.is_empty());
    }

    #[test]
    fn the_ctrl_c_exit_hint_expires_at_the_double_press_boundary() {
        let mut app = app();
        let pressed = Instant::now();
        app.last_ctrl_c = Some(pressed);

        assert!(app.ctrl_c_exit_hint_active());
        assert!(
            !app.expire_ctrl_c_exit_hint_at(pressed + FORCE_QUIT_WINDOW - Duration::from_millis(1))
        );
        assert!(app.expire_ctrl_c_exit_hint_at(pressed + FORCE_QUIT_WINDOW));
        assert!(!app.ctrl_c_exit_hint_active());
        assert!(app.last_ctrl_c.is_none());
    }

    #[test]
    fn a_second_ctrl_c_exits_when_idle_or_busy() {
        let mut idle = app();
        assert_eq!(idle.on_key(ctrl('c')), None);
        assert_eq!(idle.on_key(ctrl('c')), Some(Action::Quit));
        assert!(idle.should_quit);

        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(app.on_key(ctrl('c')), None);
        assert_eq!(app.on_key(ctrl('c')), Some(Action::Quit));
        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn cancelling_an_explicit_exit_restores_the_pending_approval() {
        let mut app = app();
        app.present_approval(prompt("shell").await);

        assert_eq!(app.request_exit(), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::ExitConfirm {
                approval: Some(_),
                ..
            })
        ));
        assert_eq!(app.on_key(key(KeyCode::Char('n'))), None);
        assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
    }

    #[tokio::test]
    async fn approval_is_never_granted_by_enter() {
        let mut app = app();
        app.present_approval(prompt("shell").await);

        // Enter, Tab, and an ordinary character must all leave the modal open.
        for code in [KeyCode::Enter, KeyCode::Tab, KeyCode::Char('x')] {
            assert_eq!(app.on_key(key(code)), None);
            assert!(
                matches!(app.overlay, Some(Overlay::Approval { .. })),
                "{code:?} must not answer an approval"
            );
        }

        app.on_key(key(KeyCode::Char('y')));
        assert!(app.overlay.is_none());
    }

    #[tokio::test]
    async fn parallel_approval_requests_are_presented_in_fifo_order() {
        let mut app = app();
        let (shell, shell_decision) =
            pending_prompt_with("shell", serde_json::json!({"command": "build"})).await;
        let (patch, patch_decision) =
            pending_prompt_with("patch", serde_json::json!({"path": "src/lib.rs"})).await;
        app.present_approval(shell);
        app.present_approval(patch);

        match &app.overlay {
            Some(Overlay::Approval { prompt, .. }) => assert_eq!(prompt.tool(), "shell"),
            other => panic!("expected the first approval, got {other:?}"),
        }
        assert_eq!(app.pending_approval_count(), 2);

        app.on_key(key(KeyCode::Char('n')));
        match &app.overlay {
            Some(Overlay::Approval { prompt, .. }) => assert_eq!(prompt.tool(), "patch"),
            other => panic!("expected the queued approval, got {other:?}"),
        }
        assert_eq!(app.pending_approval_count(), 1);

        app.on_key(key(KeyCode::Char('y')));
        assert!(app.overlay.is_none());
        assert_eq!(app.pending_approval_count(), 0);
        assert!(matches!(
            shell_decision.await.expect("shell decision"),
            ApprovalDecision::Deny { .. }
        ));
        assert_eq!(
            patch_decision.await.expect("patch decision"),
            ApprovalDecision::Allow
        );
    }

    #[test]
    fn questionnaire_requires_explicit_submit_and_resolves_once() {
        let mut app = app();
        app.present_questionnaire(questionnaire_form("question-1", Deadline::never()));

        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(
            app.take_questionnaire_resolution().is_none(),
            "Enter on an answer stages it; it must not submit the form"
        );
        assert!(matches!(app.overlay, Some(Overlay::Questionnaire { .. })));

        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        let (request_id, resolution) = app
            .take_questionnaire_resolution()
            .expect("explicit Submit resolves the request");
        assert_eq!(request_id, "question-1");
        assert!(matches!(resolution, QuestionnaireResolution::Submitted(_)));
        assert!(app.take_questionnaire_resolution().is_none());
        assert!(app.overlay.is_none());
    }

    #[tokio::test]
    async fn questionnaire_and_approval_prompts_share_one_fifo() {
        let mut approval_first = app();
        let (approval, decision) =
            pending_prompt_with("shell", serde_json::json!({"command": "build"})).await;
        approval_first.present_approval(approval);
        approval_first
            .present_questionnaire(questionnaire_form("queued-question", Deadline::never()));
        approval_first.on_key(key(KeyCode::Char('n')));
        assert!(matches!(
            approval_first.overlay,
            Some(Overlay::Questionnaire { .. })
        ));
        assert!(matches!(
            decision.await.expect("approval decision"),
            ApprovalDecision::Deny { .. }
        ));

        let mut question_first = app();
        let (approval, decision) =
            pending_prompt_with("edit", serde_json::json!({"path": "src/lib.rs"})).await;
        question_first
            .present_questionnaire(questionnaire_form("first-question", Deadline::never()));
        question_first.present_approval(approval);
        question_first.on_key(key(KeyCode::Tab));
        question_first.on_key(key(KeyCode::Tab));
        question_first.on_key(key(KeyCode::Enter));
        assert!(matches!(
            question_first.overlay,
            Some(Overlay::Approval { .. })
        ));
        assert!(matches!(
            question_first.take_questionnaire_resolution(),
            Some((
                request_id,
                QuestionnaireResolution::Declined
            )) if request_id == "first-question"
        ));
        question_first.on_key(key(KeyCode::Char('n')));
        assert!(matches!(
            decision.await.expect("approval decision"),
            ApprovalDecision::Deny { .. }
        ));
    }

    #[test]
    fn forced_exit_cancels_visible_and_queued_questionnaires_exactly_once() {
        let mut app = app();
        app.present_questionnaire(questionnaire_form("visible", Deadline::never()));
        app.present_questionnaire(questionnaire_form("queued", Deadline::never()).restored(true));

        assert_eq!(app.on_key(ctrl('c')), None);
        assert!(matches!(app.overlay, Some(Overlay::Questionnaire { .. })));
        assert_eq!(app.on_key(ctrl('c')), Some(Action::Quit));

        for expected in ["visible", "queued"] {
            assert!(matches!(
                app.take_questionnaire_resolution(),
                Some((request_id, QuestionnaireResolution::Cancelled))
                    if request_id == expected
            ));
        }
        assert!(app.take_questionnaire_resolution().is_none());
        assert!(app.overlay.is_none());
    }

    #[tokio::test]
    async fn questionnaire_deadlines_remove_queued_requests_without_answering_them() {
        let mut app = app();
        app.present_questionnaire(questionnaire_form("visible", Deadline::never()));
        app.present_questionnaire(questionnaire_form(
            "expired",
            Deadline::after(&SystemClock, 1),
        ));
        tokio::time::sleep(Duration::from_millis(5)).await;

        app.tick();

        assert!(matches!(
            app.take_questionnaire_resolution(),
            Some((request_id, QuestionnaireResolution::TimedOut))
                if request_id == "expired"
        ));
        assert!(matches!(app.overlay, Some(Overlay::Questionnaire { .. })));
        app.on_key(key(KeyCode::Esc));
        assert!(matches!(
            app.take_questionnaire_resolution(),
            Some((request_id, QuestionnaireResolution::Cancelled))
                if request_id == "visible"
        ));
        assert!(app.take_questionnaire_resolution().is_none());
    }

    #[test]
    fn session_shutdown_cancels_every_questionnaire_responder_once() {
        let mut app = app();
        app.present_questionnaire(questionnaire_form("visible", Deadline::never()));
        app.present_questionnaire(questionnaire_form("queued", Deadline::never()));

        app.apply(&event(RuntimeEvent::SessionShutdown));

        for expected in ["visible", "queued"] {
            assert!(matches!(
                app.take_questionnaire_resolution(),
                Some((request_id, QuestionnaireResolution::Cancelled))
                    if request_id == expected
            ));
        }
        assert!(app.take_questionnaire_resolution().is_none());
        assert!(app.overlay.is_none());
    }

    #[test]
    fn runtime_close_removes_visible_or_queued_questionnaires_idempotently() {
        let mut app = app();
        app.present_questionnaire(questionnaire_form("visible", Deadline::never()));
        app.present_questionnaire(questionnaire_form("queued", Deadline::never()));

        app.dismiss_questionnaire("visible");
        assert!(matches!(
            &app.overlay,
            Some(Overlay::Questionnaire { state })
                if state.form().request_id == "queued"
        ));
        app.dismiss_questionnaire("visible");
        assert!(app.take_questionnaire_resolution().is_none());

        app.dismiss_questionnaire("queued");
        app.dismiss_questionnaire("queued");
        assert!(app.overlay.is_none());
        assert_eq!(app.pending_questionnaire_count(), 0);
        assert!(app.take_questionnaire_resolution().is_none());
    }

    #[tokio::test]
    async fn terminal_exit_cancels_visible_and_queued_approval_responders() {
        let mut app = app();
        let (shell, shell_decision) =
            pending_prompt_with("shell", serde_json::json!({"command": "build"})).await;
        let (patch, patch_decision) =
            pending_prompt_with("patch", serde_json::json!({"path": "src/lib.rs"})).await;
        app.present_approval(shell);
        app.present_approval(patch);

        assert_eq!(app.on_key(ctrl('c')), None);
        assert_eq!(app.on_key(ctrl('c')), Some(Action::Quit));

        assert_eq!(
            shell_decision.await.expect("shell cancellation"),
            ApprovalDecision::Cancelled
        );
        assert_eq!(
            patch_decision.await.expect("patch cancellation"),
            ApprovalDecision::Cancelled
        );
    }

    #[tokio::test]
    async fn approval_deadlines_close_the_prompt_without_selecting_a_default() {
        let mut app = app();
        let (prompt, decision) = pending_prompt_with_deadline(
            "shell",
            serde_json::json!({"command": "build"}),
            Deadline::after(&SystemClock, 1),
        )
        .await;
        app.present_approval(prompt);
        tokio::time::sleep(Duration::from_millis(5)).await;

        app.tick();

        assert!(app.overlay.is_none());
        assert_eq!(
            decision.await.expect("timeout decision"),
            ApprovalDecision::TimedOut
        );
        assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Notice { source, text }
                if source == "approval" && text.contains("timed out")
        )));
    }

    #[tokio::test]
    async fn a_prompt_delivered_before_turn_started_is_not_mistaken_for_stale_work() {
        let mut app = app();
        let (prompt, decision) =
            pending_prompt_with("shell", serde_json::json!({"command": "build"})).await;
        app.present_approval(prompt);

        // Runtime events and approval prompts use independent channels. The
        // host loop may receive the prompt first even though TurnStarted was
        // emitted first on the event stream.
        app.apply(&event(RuntimeEvent::TurnStarted));

        assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
        app.on_key(key(KeyCode::Char('y')));
        assert_eq!(
            decision.await.expect("approval decision"),
            ApprovalDecision::Allow
        );
    }

    #[tokio::test]
    async fn an_edit_approval_carries_its_review_but_a_shell_one_does_not() {
        let mut edit = app();
        edit.present_approval(
            prompt_with(
                "edit",
                serde_json::json!({
                    "path": "src/retry.rs",
                    "old_string": "once();\n",
                    "new_string": "twice();\n",
                }),
            )
            .await,
        );
        match &edit.overlay {
            Some(Overlay::Approval { review, .. }) => {
                let review = review.as_ref().expect("an edit call is reviewable");
                assert_eq!(review.path, "src/retry.rs");
                assert_eq!(review.added, 1);
            }
            other => panic!("expected an approval, got {other:?}"),
        }

        let mut shell = app();
        shell.present_approval(prompt("shell").await);
        match &shell.overlay {
            Some(Overlay::Approval { review, .. }) => assert!(
                review.is_none(),
                "a shell call must fall back to its arguments"
            ),
            other => panic!("expected an approval, got {other:?}"),
        }
    }

    #[test]
    fn at_escape_and_shell_escape_are_provider_prompts() {
        let mut at = agent_first_app();
        at.on_key(key(KeyCode::Char('@')));
        assert!(matches!(at.overlay, Some(Overlay::ResourcePicker { .. })));
        at.on_key(key(KeyCode::Char('@')));
        type_text(&mut at, "owner please");
        assert_eq!(
            expect_whole_submission(at.on_key(key(KeyCode::Enter))).display_text(),
            "@owner please"
        );

        let mut shell = agent_first_app();
        shell.composer.replace("!cargo test --workspace");
        assert_eq!(
            shell.on_key(key(KeyCode::Enter)),
            Some(Action::RunShell {
                command: "cargo test --workspace".to_owned(),
            })
        );

        let mut literal_shell = agent_first_app();
        literal_shell.composer.replace("!!explain shell syntax");
        assert_eq!(
            expect_whole_submission(literal_shell.on_key(key(KeyCode::Enter))).display_text(),
            "!explain shell syntax"
        );
    }

    #[test]
    fn recovery_and_review_confirmations_have_no_enter_default() {
        let mut undo = app();
        undo.confirm_undo("--- current\n+++ restore\n-old\n+new\n");
        assert_eq!(undo.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(undo.overlay, Some(Overlay::UndoConfirm { .. })));
        assert_eq!(undo.on_key(key(KeyCode::Esc)), Some(Action::CancelUndo));

        let mut review = app();
        review.confirm_review("all", "provider-backed: yes");
        assert_eq!(review.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            review.overlay,
            Some(Overlay::ReviewConfirm { .. })
        ));

        let mut revert = app();
        revert.confirm_revert("file.txt", "fingerprint", "reverse patch");
        assert_eq!(
            revert.on_key(key(KeyCode::Char('n'))),
            Some(Action::CancelRevert {
                scope: "file.txt".to_owned(),
                fingerprint: "fingerprint".to_owned(),
            })
        );
    }
