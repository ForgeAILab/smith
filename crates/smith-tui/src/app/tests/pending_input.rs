// pending input behavior tests.

    #[test]
    fn sending_records_the_message_and_clears_the_composer() {
        let mut app = app();
        type_text(&mut app, "run the tests");
        let submission = expect_whole_submission(app.on_key(key(KeyCode::Enter)));

        assert_eq!(submission.display_text(), "run the tests");
        assert!(app.composer.is_empty());
        assert!(app.transcript.blocks().is_empty());
        app.whole_turn_dispatched(TurnId::new("turn-1"), &submission);
        assert_eq!(
            app.transcript.blocks()[0],
            Block::User {
                text: "run the tests".into()
            }
        );
    }

    #[test]
    fn busy_enter_waits_for_the_matching_steer_commit_before_transcript_history() {
        let mut app = app();
        app.apply(&turn_event("turn-1", RuntimeEvent::TurnStarted));
        type_text(&mut app, "also test cancellation");
        let action = app.on_key(key(KeyCode::Enter));
        let submission = match action {
            Some(Action::Submit {
                submission,
                target:
                    SubmissionTarget::Steer {
                        expected_turn: Some(turn),
                    },
            }) => {
                assert_eq!(turn, TurnId::new("turn-1"));
                submission
            }
            other => panic!("expected a targeted steer, got {other:?}"),
        };
        assert!(app.transcript.blocks().is_empty());

        app.accept_steer(
            SteerReceipt {
                id: SteerId::new("steer-1"),
                turn: TurnId::new("turn-1"),
                ordinal: 1,
            },
            submission,
        );
        assert_eq!(
            app.pending_input_previews()[0].entries,
            ["also test cancellation"]
        );
        app.apply(&turn_event(
            "turn-1",
            RuntimeEvent::TurnSteerCommitted {
                steer: SteerId::new("steer-1"),
                ordinal: 1,
            },
        ));
        app.apply(&turn_event(
            "turn-1",
            RuntimeEvent::TurnSteerCommitted {
                steer: SteerId::new("steer-1"),
                ordinal: 1,
            },
        ));
        assert_eq!(
            app.transcript
                .blocks()
                .iter()
                .filter(|block| matches!(block, Block::User { text } if text == "also test cancellation"))
                .count(),
            1
        );
        assert!(app.pending_input_previews().is_empty());
    }

    #[test]
    fn busy_tab_queues_fifo_and_alt_up_edits_only_the_newest_explicit_turn() {
        let mut app = app();
        app.apply(&turn_event("turn-1", RuntimeEvent::TurnStarted));
        for draft in ["first later turn", "second later turn"] {
            app.composer.replace(draft);
            assert_eq!(app.on_key(key(KeyCode::Tab)), None);
        }
        assert_eq!(
            app.pending_input_previews()[0].entries,
            ["first later turn", "second later turn"]
        );

        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT)),
            None
        );
        assert_eq!(app.composer.text(), "second later turn");
        assert_eq!(
            app.pending_input_previews()[0].entries,
            ["first later turn"]
        );

        app.composer.replace("/status");
        assert_eq!(app.on_key(key(KeyCode::Tab)), None);
        assert_eq!(app.composer.text(), "/status");
        assert_eq!(
            app.pending_input_previews()[0].entries,
            ["first later turn"]
        );
    }

    #[test]
    fn explicit_busy_queue_is_bounded_and_keeps_the_overflow_draft_editable() {
        let mut app = app();
        app.apply(&turn_event("turn-1", RuntimeEvent::TurnStarted));
        for index in 0..MAX_EXPLICIT_QUEUED_TURNS {
            app.composer.replace(format!("queued {index}"));
            app.on_key(key(KeyCode::Tab));
        }
        app.composer.replace("one too many");
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.composer.text(), "one too many");
        assert_eq!(
            app.pending_input.queued_turns.len(),
            MAX_EXPLICIT_QUEUED_TURNS
        );
        assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Error { message } if message.contains("queue is full")
        )));
    }

    #[test]
    fn rejected_steers_dispatch_before_one_explicit_queue_entry_per_success() {
        let mut app = app();
        app.apply(&turn_event("turn-1", RuntimeEvent::TurnStarted));
        app.composer.replace("explicit later");
        app.on_key(key(KeyCode::Tab));

        let rejected = app
            .prepare_ordinary_submission("steer became follow-up")
            .unwrap()
            .unwrap();
        app.reject_steer_for_followup(Some(TurnId::new("turn-1")), rejected);
        app.apply(&turn_event(
            "turn-1",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: false,
            },
        ));
        let first = app.take_ready_submission().expect("rejected follow-up");
        assert_eq!(first.display_text(), "steer became follow-up");
        assert!(app.take_ready_submission().is_none());

        app.whole_turn_dispatched(TurnId::new("turn-2"), &first);
        app.apply(&turn_event(
            "turn-2",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: false,
            },
        ));
        assert_eq!(
            app.take_ready_submission()
                .expect("one explicit queue entry")
                .display_text(),
            "explicit later"
        );
    }

    #[test]
    fn interrupt_for_steer_resends_only_uncommitted_dispositions() {
        let mut app = app();
        app.apply(&turn_event("turn-1", RuntimeEvent::TurnStarted));
        for (ordinal, text) in [(1, "already committed"), (2, "still pending")] {
            let submission = app.prepare_ordinary_submission(text).unwrap().unwrap();
            app.accept_steer(
                SteerReceipt {
                    id: SteerId::new(format!("steer-{ordinal}")),
                    turn: TurnId::new("turn-1"),
                    ordinal,
                },
                submission,
            );
        }
        app.apply(&turn_event(
            "turn-1",
            RuntimeEvent::TurnSteerCommitted {
                steer: SteerId::new("steer-1"),
                ordinal: 1,
            },
        ));
        assert_eq!(app.on_key(key(KeyCode::Esc)), Some(Action::Interrupt));
        app.apply(&turn_event(
            "turn-1",
            RuntimeEvent::TurnSteerDiscarded {
                steer: SteerId::new("steer-2"),
                ordinal: 2,
                reason: agent_runtime_core::steer::SteerDiscardReason::Cancelled,
            },
        ));
        app.apply(&turn_event(
            "turn-1",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Cancelled {
                    reason: CancelReason::UserRequested,
                },
                visible_output: false,
            },
        ));
        assert_eq!(
            app.take_ready_submission()
                .expect("discarded steer resubmission")
                .display_text(),
            "still pending"
        );
    }

    #[test]
    fn non_success_restores_pastes_and_explicit_queue_without_spend() {
        let mut app = app();
        app.apply(&turn_event("turn-1", RuntimeEvent::TurnStarted));
        app.on_paste("one\ntwo\nthree");
        type_text(&mut app, " later");
        app.on_key(key(KeyCode::Tab));
        app.pasted_chunks.clear();

        app.apply(&turn_event(
            "turn-1",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Failed,
                visible_output: false,
            },
        ));
        assert_eq!(app.composer.text(), "[Pasted text #1 +3 lines] later");
        assert_eq!(
            app.expand_pasted(app.composer.text()),
            "one\ntwo\nthree later"
        );
        for _ in " later".chars() {
            app.on_key(key(KeyCode::Left));
        }
        app.on_key(key(KeyCode::Left));
        assert_eq!(app.composer.cursor(), 0);
        assert!(app.take_ready_submission().is_none());
    }

    #[test]
    fn shift_enter_inserts_a_newline_instead_of_sending() {
        let mut app = app();
        type_text(&mut app, "line one");
        let action = app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(action, None);
        assert_eq!(app.composer.text(), "line one\n");
    }

    #[test]
    fn a_large_paste_collapses_to_a_placeholder_and_expands_on_send() {
        let mut app = app();
        app.on_paste("fn main() {\r\n    println!(\"hi\");\r\n}");
        assert_eq!(app.composer.text(), "[Pasted text #1 +3 lines]");

        type_text(&mut app, " explain this");
        let submission = expect_whole_submission(app.on_key(key(KeyCode::Enter)));
        assert_eq!(
            submission.input_without_files(),
            UserInput::text("fn main() {\n    println!(\"hi\");\n} explain this")
        );
        app.whole_turn_dispatched(TurnId::new("turn-1"), &submission);
        // The compact label is an editing token; committed history shows what
        // was actually sent.
        match app.transcript.blocks().last() {
            Some(Block::User { text }) => {
                assert_eq!(text, "fn main() {\n    println!(\"hi\");\n} explain this");
            }
            other => panic!("expected a user block, got {other:?}"),
        }
    }

    #[test]
    fn composer_history_recalls_a_registered_paste_as_one_unit() {
        let mut app = app();
        app.on_paste("one\ntwo\nthree");
        let submission = expect_whole_submission(app.on_key(key(KeyCode::Enter)));
        assert_eq!(submission.display_text(), "[Pasted text #1 +3 lines]");

        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "[Pasted text #1 +3 lines]");
        app.on_key(key(KeyCode::Left));
        assert_eq!(app.composer.cursor(), 0);
    }

    #[test]
    fn a_committed_steer_expands_pasted_text_after_a_compact_preview() {
        let mut app = app();
        app.apply(&turn_event("turn-1", RuntimeEvent::TurnStarted));
        app.on_paste("one\ntwo\nthree");
        let submission = match app.on_key(key(KeyCode::Enter)) {
            Some(Action::Submit { submission, .. }) => submission,
            other => panic!("expected a steer submission, got {other:?}"),
        };
        app.accept_steer(
            SteerReceipt {
                id: SteerId::new("steer-paste"),
                turn: TurnId::new("turn-1"),
                ordinal: 1,
            },
            submission,
        );
        assert_eq!(
            app.pending_input_previews()[0].entries,
            ["[Pasted text #1 +3 lines]"]
        );

        app.apply(&turn_event(
            "turn-1",
            RuntimeEvent::TurnSteerCommitted {
                steer: SteerId::new("steer-paste"),
                ordinal: 1,
            },
        ));
        assert_eq!(
            app.transcript.blocks().last(),
            Some(&Block::User {
                text: "one\ntwo\nthree".into()
            })
        );
    }

    #[test]
    fn short_pastes_insert_inline_and_placeholders_number_upward() {
        let mut app = app();
        app.on_paste("hello world");
        assert_eq!(app.composer.text(), "hello world");
        app.composer.clear();

        app.on_paste("a\nb\nc");
        app.on_paste("d\ne\nf\ng");
        assert_eq!(
            app.composer.text(),
            "[Pasted text #1 +3 lines][Pasted text #2 +4 lines]"
        );
        assert_eq!(app.expand_pasted(app.composer.text()), "a\nb\ncd\ne\nf\ng");
    }

    #[test]
    fn registered_paste_and_image_labels_are_atomic_editing_units() {
        let mut app = app();
        app.on_paste("one\ntwo\nthree");
        app.attach_image("data:image/png;base64,IMAGE".into(), 32, 32);
        let paste = "[Pasted text #1 +3 lines]";
        assert_eq!(
            app.composer.text(),
            "[Pasted text #1 +3 lines][Image #1 32×32]"
        );

        app.on_key(key(KeyCode::Left));
        assert_eq!(app.composer.cursor(), paste.chars().count());
        app.on_key(key(KeyCode::Delete));
        assert_eq!(app.composer.text(), paste);
        app.on_key(key(KeyCode::Backspace));
        assert!(app.composer.is_empty());
    }

    #[test]
    fn a_long_single_line_paste_also_collapses() {
        let mut app = app();
        app.on_paste(&"x".repeat(2_000));
        assert_eq!(app.composer.text(), "[Pasted text #1 +1 line]");
    }

    #[test]
    fn unregistered_placeholder_lookalikes_never_expand() {
        let mut app = app();
        type_text(&mut app, "[Pasted text #9 +4 lines] check");
        assert_eq!(
            expect_whole_submission(app.on_key(key(KeyCode::Enter))).input_without_files(),
            UserInput::text("[Pasted text #9 +4 lines] check")
        );
    }

    #[test]
    fn pasted_chunk_content_is_not_rescanned_for_placeholders() {
        let mut app = app();
        // The pasted content itself contains what will become chunk #2's
        // label; expanding chunk #1 must not expand that inner text.
        app.on_paste("[Pasted text #2 +3 lines]\nliteral\ntail");
        app.on_paste("p\nq\nr");
        let expanded = app.expand_pasted(app.composer.text());
        assert_eq!(expanded, "[Pasted text #2 +3 lines]\nliteral\ntailp\nq\nr");
    }

    #[test]
    fn a_shell_shortcut_expands_pasted_chunks_into_the_command() {
        let mut app = app();
        type_text(&mut app, "!");
        app.on_paste("echo one\necho two\necho three");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::RunShell {
                command: "echo one\necho two\necho three".to_owned()
            })
        );
    }

    #[test]
    fn attached_images_ride_the_send_in_placeholder_order() {
        let mut app = app();
        app.attach_image("data:image/png;base64,FIRST".into(), 800, 600);
        type_text(&mut app, " and ");
        app.attach_image("data:image/png;base64,SECOND".into(), 32, 32);
        assert_eq!(
            app.composer.text(),
            "[Image #1 800×600] and [Image #2 32×32]"
        );

        type_text(&mut app, " compare these");
        let submission = expect_whole_submission(app.on_key(key(KeyCode::Enter)));
        assert_eq!(
            submission.input_without_files().parts,
            vec![
                ContentPart::text("[Image #1 800×600] and [Image #2 32×32] compare these"),
                ContentPart::Image {
                    url: "data:image/png;base64,FIRST".to_owned(),
                    detail: None,
                },
                ContentPart::Image {
                    url: "data:image/png;base64,SECOND".to_owned(),
                    detail: None,
                },
            ]
        );
        app.whole_turn_dispatched(TurnId::new("turn-images"), &submission);
        assert_eq!(
            app.transcript.blocks().last(),
            Some(&Block::User {
                text: "[Image #1 800×600] and [Image #2 32×32] compare these".into()
            })
        );
    }

    #[test]
    fn a_deleted_image_placeholder_detaches_its_image() {
        let mut app = app();
        app.attach_image("data:image/png;base64,GONE".into(), 10, 10);
        app.on_key(key(KeyCode::Left));
        app.on_key(key(KeyCode::Delete));
        type_text(&mut app, "no image after all");
        assert_eq!(
            expect_whole_submission(app.on_key(key(KeyCode::Enter))).input_without_files(),
            UserInput::text("no image after all")
        );
    }

    #[test]
    fn pasted_text_cannot_reattach_a_deleted_image_by_naming_its_label() {
        let mut app = app();
        app.attach_image("data:image/png;base64,GONE".into(), 10, 10);
        app.on_key(key(KeyCode::Left));
        app.on_key(key(KeyCode::Delete));
        app.on_paste("[Image #1 10×10]\nliteral mention\nonly text");

        let submission = expect_whole_submission(app.on_key(key(KeyCode::Enter)));
        assert_eq!(
            submission.input_without_files(),
            UserInput::text("[Image #1 10×10]\nliteral mention\nonly text")
        );
    }

    #[test]
    fn typed_image_paths_and_lookalikes_remain_ordinary_text() {
        let mut app = app();
        let text = "assets/photo.png [Image #1 10×10]";
        type_text(&mut app, text);
        let end = app.composer.cursor();
        app.on_key(key(KeyCode::Left));
        assert_eq!(app.composer.cursor(), end - 1);
        app.on_key(key(KeyCode::Right));

        let submission = expect_whole_submission(app.on_key(key(KeyCode::Enter)));
        assert_eq!(submission.input_without_files(), UserInput::text(text));
    }

    #[test]
    fn image_attachment_is_refused_while_a_modal_owns_the_screen() {
        let mut app = app();
        assert!(app.can_attach_image());
        app.overlay = Some(Overlay::UndoConfirm {
            content: "preview".into(),
        });
        assert!(!app.can_attach_image());
    }

    #[test]
    fn a_busy_turn_can_discover_but_cannot_run_an_idle_command() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(app.on_key(ctrl('p')), None);
        assert!(matches!(app.overlay, Some(Overlay::Palette { .. })));
        type_text(&mut app, "model next");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(app.composer.text(), "/model next");
        assert!(app.overlay.is_none());
        assert!(
            app.transcript.blocks().iter().any(|block| {
                matches!(
                    block, Block::Notice { text, .. }
                    if text.contains("requires an idle turn") && text.contains("draft preserved")
                )
            }),
            "the rejected switch was invisible"
        );
    }

    #[test]
    fn pending_input_blocks_reconfiguration_after_the_active_turn_ends() {
        let mut app = app();
        app.apply(&turn_event("turn-1", RuntimeEvent::TurnStarted));
        app.composer.replace("queued for later");
        assert_eq!(app.on_key(key(KeyCode::Tab)), None);
        app.apply(&turn_event(
            "turn-1",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: false,
            },
        ));
        assert!(!app.is_busy());
        assert!(app.has_pending_input());

        app.composer.replace("/model model-2");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(app.composer.text(), "/model model-2");
        assert!(app.has_pending_input());
    }

    #[test]
    fn sending_a_message_resumes_following() {
        let mut app = app();
        app.sync_scroll_limit(20);
        app.scroll_up(5);
        assert!(!app.following);

        type_text(&mut app, "next");
        app.on_key(key(KeyCode::Enter));
        assert!(app.following);
    }

    #[test]
    fn file_reference_submission_is_typed_and_unresolved_bare_tokens_are_text() {
        let mut app = agent_first_app();
        app.composer.replace("inspect @src/lib.rs");
        let submission = expect_whole_submission(app.on_key(key(KeyCode::Enter)));
        assert_eq!(submission.display_text(), "inspect @src/lib.rs");
        assert_eq!(submission.files(), &["src/lib.rs".to_owned()]);

        let mut unresolved = agent_first_app();
        unresolved.composer.replace("inspect @missing.rs");
        let submission = expect_whole_submission(unresolved.on_key(key(KeyCode::Enter)));
        assert_eq!(submission.display_text(), "inspect @missing.rs");
        assert!(submission.files().is_empty());
        assert!(unresolved.composer.is_empty());
    }
