// Characterize reduce_key's ordering before introducing the descriptive table.

mod key_ordering {
    use super::*;

    #[tokio::test]
    async fn ctrl_c_twice_quits_through_an_approval_before_and_after_the_quiet_window() {
        for quiet in [true, false] {
            let mut app = app();
            prompt_clock(&mut app);
            let (prompt, decision) =
                pending_prompt_with("shell", serde_json::json!({"command": "git status"})).await;
            app.present_approval(prompt);
            if !quiet {
                elapse_prompt_guard(&mut app);
            }
            app.composer.replace("recover this draft");

            assert_eq!(app.on_key(ctrl('c')), None);
            assert!(app.composer.is_empty());
            assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
            assert!(!decision.is_finished());
            assert_eq!(app.on_key(ctrl('c')), Some(Action::Quit), "quiet={quiet}");
            assert!(app.should_quit);
            assert!(app.overlay.is_none());
            assert_eq!(app.pending_approval_count(), 0);
            assert_eq!(
                decision.await.expect("cancelled approval"),
                ApprovalDecision::Cancelled
            );
        }
    }

    #[tokio::test]
    async fn ctrl_c_twice_quits_through_confirmations_before_and_after_the_quiet_window() {
        for (index, case) in confirmation_cases().into_iter().enumerate() {
            for quiet in [true, false] {
                let mut app = agent_first_app();
                prompt_clock(&mut app);
                let rotation = open_confirmation_case(&mut app, index, "preview").await;
                assert!(matches!(app.overlay, Some(Overlay::Confirm(_))));
                if !quiet {
                    elapse_prompt_guard(&mut app);
                }
                assert_eq!(app.on_key(ctrl('c')), None);
                assert!(matches!(app.overlay, Some(Overlay::Confirm(_))));
                assert_eq!(
                    app.on_key(ctrl('c')),
                    Some(Action::Quit),
                    "{}: quiet={quiet}",
                    case.title
                );
                assert!(app.should_quit);
                assert!(app.overlay.is_none());
                if let Some(rotation) = rotation {
                    assert_eq!(
                        rotation.await.expect("cancelled rotation"),
                        RotationDecision::Decline
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn transcript_navigation_works_inside_an_approvals_quiet_window_without_answering() {
        let mut app = app();
        prompt_clock(&mut app);
        app.sync_scroll_limit(40);
        let (prompt, decision) =
            pending_prompt_with("shell", serde_json::json!({"command": "git status"})).await;
        app.present_approval(prompt);

        for (event, offset, following) in [
            (key(KeyCode::PageUp), 10, false),
            (key(KeyCode::Home), 40, false),
            (ctrl('l'), 0, true),
        ] {
            assert_eq!(app.on_key(event), None);
            assert_eq!(app.scroll_back, offset);
            assert_eq!(app.following, following);
            assert!(app.prompt_input_guard.quiet_until.is_some());
            assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
            assert_eq!(app.pending_approval_count(), 1);
            assert!(!decision.is_finished());
        }
        app.on_key(ctrl('c'));
        app.on_key(ctrl('c'));
        assert_eq!(
            decision.await.expect("cancelled approval"),
            ApprovalDecision::Cancelled
        );
    }

    #[test]
    fn shortcuts_escape_only_closes_even_with_work_and_a_draft() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.composer.replace("keep drafting");
        assert!(app.open_overlay(Overlay::Shortcuts));

        assert_eq!(app.on_key(key(KeyCode::Esc)), None);
        assert!(app.overlay.is_none());
        assert_eq!(app.composer.text(), "keep drafting");
        assert_eq!(app.status.activity, crate::status::Activity::Working);
    }

    #[test]
    fn other_shortcuts_closing_keys_keep_their_normal_meaning() {
        let mut app = agent_first_app();
        app.on_key(key(KeyCode::Char('?')));
        assert!(matches!(app.overlay, Some(Overlay::Shortcuts)));
        assert_eq!(
            app.on_key(key(KeyCode::Tab)),
            Some(Action::Reconfigure(SessionControl::Reconfigure(
                SelectionCommand::Profile("plan".to_owned())
            )))
        );
        assert!(app.overlay.is_none());

        for character in ['x', '?'] {
            app.composer.clear();
            app.on_key(key(KeyCode::Char('?')));
            assert!(matches!(app.overlay, Some(Overlay::Shortcuts)));
            assert_eq!(app.on_key(key(KeyCode::Char(character))), None);
            assert!(app.overlay.is_none());
            assert_eq!(app.composer.text(), character.to_string());
        }
    }

    #[tokio::test]
    async fn ctrl_o_toggles_detail_through_confirmations_before_and_after_the_quiet_window() {
        for (index, case) in confirmation_cases().into_iter().enumerate() {
            for quiet in [true, false] {
                let mut app = agent_first_app();
                prompt_clock(&mut app);
                let rotation = open_confirmation_case(&mut app, index, "preview").await;
                if !quiet {
                    elapse_prompt_guard(&mut app);
                }
                let draft = app.composer.text().to_owned();
                for expanded in [true, false] {
                    assert_eq!(app.on_key(ctrl('o')), None);
                    assert_eq!(app.work_details, expanded, "{}: quiet={quiet}", case.title);
                    assert!(matches!(app.overlay, Some(Overlay::Confirm(_))));
                    assert_eq!(app.composer.text(), draft);
                    if let Some(rotation) = &rotation {
                        assert!(!rotation.is_finished());
                    }
                }
                app.on_key(ctrl('c'));
                app.on_key(ctrl('c'));
                if let Some(rotation) = rotation {
                    assert_eq!(
                        rotation.await.expect("cancelled rotation"),
                        RotationDecision::Decline
                    );
                }
            }
        }
    }
}
