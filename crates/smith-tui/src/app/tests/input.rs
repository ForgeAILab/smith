// input behavior tests.

    #[test]
    fn slash_draft_clears_a_refused_command_before_typing_the_next_one() {
        let mut app = app();
        type_text(&mut app, "/agentx");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(app.composer.text(), "/agentx");
        assert!(matches!(
            &app.overlay,
            Some(Overlay::Palette { error: Some(error), .. })
                if error.contains("unknown command `/agentx`")
        ));

        assert_eq!(app.on_key(ctrl('u')), None);
        assert!(app.composer.is_empty());
        assert!(app.overlay.is_none());
        type_text(&mut app, "/agent");
        assert_eq!(app.composer.text(), "/agent");
        assert!(matches!(
            app.overlay,
            Some(Overlay::Palette { error: None, .. })
        ));
    }

    #[test]
    fn slash_draft_left_right_and_delete_edit_at_character_boundaries() {
        let mut app = app();
        type_text(&mut app, "/agent café");
        for (code, cursor, text) in [
            (KeyCode::Left, 10, "/agent café"),
            (KeyCode::Left, 9, "/agent café"),
            (KeyCode::Right, 10, "/agent café"),
            (KeyCode::Delete, 10, "/agent caf"),
        ] {
            assert_eq!(app.on_key(key(code)), None);
            assert_eq!(app.composer.cursor(), cursor);
            assert_eq!(app.composer.text(), text);
            assert!(matches!(
                app.overlay,
                Some(Overlay::Palette { error: None, .. })
            ));
        }
    }

    #[test]
    fn slash_draft_closes_the_palette_when_delete_removes_the_slash() {
        let mut app = app();
        type_text(&mut app, "/agent");
        app.on_key(key(KeyCode::Home));
        app.on_key(key(KeyCode::Delete));
        assert_eq!(app.composer.text(), "agent");
        assert!(app.overlay.is_none());
        app.on_key(key(KeyCode::Char('x')));
        assert_eq!(app.composer.text(), "xagent");
    }

    #[test]
    fn shortcuts_open_only_on_an_empty_draft_and_leave_no_history() {
        for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
            let mut app = app();
            app.following = false;
            assert_eq!(
                app.on_key(KeyEvent::new(KeyCode::Char('?'), modifiers)),
                None
            );
            assert!(matches!(app.overlay, Some(Overlay::Shortcuts)));
            assert!(app.transcript.is_empty());
            assert!(app.composer.is_empty());
            assert!(
                !app.following,
                "opening shortcuts must not jump the transcript"
            );
            assert_eq!(app.on_key(key(KeyCode::Esc)), None);
            app.on_key(key(KeyCode::Up));
            assert!(
                app.composer.is_empty(),
                "shortcuts must not enter input history"
            );
            type_text(&mut app, "draft?");
            assert_eq!(app.composer.text(), "draft?");
            assert!(app.overlay.is_none());
        }
    }

    #[test]
    fn shortcuts_closing_keys_keep_their_normal_action_except_escape() {
        for closing in [
            key(KeyCode::Char('x')),
            key(KeyCode::Char('/')),
            key(KeyCode::Char('@')),
            key(KeyCode::Char('!')),
            key(KeyCode::Enter),
            key(KeyCode::Tab),
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            let mut direct = agent_first_app();
            let expected = direct.on_key(closing);
            let mut panel = agent_first_app();
            panel.on_key(key(KeyCode::Char('?')));
            assert_eq!(panel.on_key(closing), expected, "{closing:?}");
            assert!(!matches!(panel.overlay, Some(Overlay::Shortcuts)));
            assert_eq!(panel.composer.text(), direct.composer.text());
            assert_eq!(panel.work_details, direct.work_details);
            assert_eq!(
                panel.ctrl_c_exit_hint_active(),
                direct.ctrl_c_exit_hint_active()
            );
            assert!(panel.transcript.is_empty());
        }
        let mut busy = app();
        busy.apply(&event(RuntimeEvent::TurnStarted));
        busy.on_key(key(KeyCode::Char('?')));
        assert_eq!(busy.on_key(key(KeyCode::Esc)), None);
        assert!(busy.overlay.is_none());
        assert_eq!(busy.status.activity, Activity::Working);
        assert_eq!(busy.on_key(key(KeyCode::Esc)), Some(Action::Interrupt));

        let mut second_question = app();
        second_question.on_key(key(KeyCode::Char('?')));
        second_question.on_key(key(KeyCode::Char('?')));
        assert!(second_question.overlay.is_none());
        assert_eq!(second_question.composer.text(), "?");
    }

    #[test]
    fn shortcuts_survive_live_output_but_are_never_replayed() {
        let events = [
            event(RuntimeEvent::TurnStarted),
            event(RuntimeEvent::TextDelta {
                request: RequestId::new("request-1"),
                attempt: AttemptId::new("attempt-1"),
                text: "live output".to_owned(),
            }),
        ];
        let mut live = app();
        live.apply(&events[0]);
        live.on_key(key(KeyCode::Char('?')));
        live.apply(&events[1]);
        assert!(matches!(live.overlay, Some(Overlay::Shortcuts)));
        let mut replayed = app();
        for event in &events {
            replayed.apply_recovered(event);
        }
        assert!(replayed.overlay.is_none());
        assert_eq!(live.transcript.blocks(), replayed.transcript.blocks());
    }

    #[test]
    fn bash_vertical_movement_and_line_editing_use_visible_columns() {
        let mut app = app();
        app.composer.replace("!abcd\nabcd");
        app.composer.move_to_visible_position(0, 2);
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.composer.visible_cursor_position(), (1, 2));
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.cursor(), 3);
        assert_eq!(app.composer.visible_cursor_position(), (0, 2));
        app.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(app.composer.text(), "!cd\nabcd");
        assert_eq!(app.composer.cursor(), 1);
        app.on_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert_eq!(app.composer.cursor(), 3);
        app.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        app.on_key(key(KeyCode::Backspace));
        assert_eq!(app.composer.text(), "cd\nabcd");
        assert!(!app.composer.is_bash_mode());
    }

    #[test]
    fn bash_prompt_mapping_keeps_paste_and_image_labels_atomic() {
        let mut app = app();
        app.composer.replace("!");
        app.on_paste("one\ntwo\nthree");
        app.attach_image("data:image/png;base64,IMAGE".into(), 32, 32);
        app.on_key(key(KeyCode::Home));
        assert_eq!(app.composer.cursor(), 1);
        app.on_key(key(KeyCode::Right));
        assert_eq!(
            app.composer.cursor(),
            1 + "[Pasted text #1 +3 lines]".chars().count()
        );
        app.on_key(key(KeyCode::Backspace));
        assert_eq!(app.composer.text(), "![Image #1 32×32]");
        assert_eq!(app.composer.cursor(), 1);
        app.on_key(key(KeyCode::Backspace));
        assert_eq!(app.composer.text(), "[Image #1 32×32]");
        assert!(!app.composer.is_bash_mode());
        app.on_key(key(KeyCode::Delete));
        assert!(app.composer.is_empty());
    }

    #[test]
    fn pasting_from_shortcuts_closes_the_panel_and_keeps_the_paste_placeholder() {
        let mut app = app();
        app.on_key(key(KeyCode::Char('?')));
        app.on_paste("one\ntwo\nthree");
        assert!(app.overlay.is_none());
        assert_eq!(app.composer.text(), "[Pasted text #1 +3 lines]");
        assert!(app.transcript.is_empty());
    }

    #[test]
    fn a_blank_composer_sends_nothing() {
        let mut app = app();
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        type_text(&mut app, "   ");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(app.transcript.is_empty());
    }

    #[test]
    fn escape_interrupts_a_running_turn_and_otherwise_clears_input() {
        let mut app = app();
        type_text(&mut app, "draft");
        assert_eq!(app.on_key(key(KeyCode::Esc)), None);
        assert!(app.composer.is_empty());

        app.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(app.on_key(key(KeyCode::Esc)), Some(Action::Interrupt));
        assert_eq!(app.status.activity, Activity::Interrupting);
    }

    #[test]
    fn accepted_inputs_share_history_while_rejected_input_stays_scratch() {
        let mut app = app();

        type_text(&mut app, "first prompt");
        assert_eq!(
            expect_whole_submission(app.on_key(key(KeyCode::Enter))).display_text(),
            "first prompt"
        );
        type_text(&mut app, "/status");
        assert!(matches!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Command(HostCommand::Status))
        ));
        type_text(&mut app, "!cargo test");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::RunShell {
                command: "cargo test".to_owned()
            })
        );

        type_text(&mut app, "/model missing");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(app.composer.text(), "/model missing");
        assert!(app.overlay.is_none());

        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "!cargo test");
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "/status");
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "first prompt");
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.composer.text(), "/model missing");
    }

    #[test]
    fn keyboard_cursor_movement_keeps_the_composer_editable() {
        let mut app = app();
        type_text(&mut app, "copy");
        app.on_key(key(KeyCode::Left));
        app.on_key(key(KeyCode::Left));
        app.on_key(key(KeyCode::Char('-')));
        assert_eq!(app.composer.text(), "co-py");
    }

    #[test]
    fn draft_line_arrows_reach_history_only_at_the_first_or_last_line() {
        let mut app = app();
        for entry in ["old first\nold last", "new first\nnew middle\nnew last"] {
            app.composer.replace(entry);
            app.composer.record_current();
        }
        let scratch = "draft first\ndraft middle\ndraft last";
        app.composer.replace(scratch);
        app.sync_scroll_limit(40);

        for line in [1, 0] {
            assert_eq!(app.on_key(key(KeyCode::Up)), None);
            assert_eq!(app.composer.cursor_position().0, line);
            assert_eq!(app.composer.text(), scratch);
            assert!(!app.composer.is_recalling());
        }
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "new first\nnew middle\nnew last");
        for line in [1, 0] {
            app.on_key(key(KeyCode::Up));
            assert_eq!(app.composer.cursor_position().0, line);
            assert_eq!(app.composer.text(), "new first\nnew middle\nnew last");
        }
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "old first\nold last");
        app.on_key(ctrl('a'));
        app.on_key(key(KeyCode::Up));
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.composer.text(), "old first\nold last");
        assert_eq!(app.composer.cursor_position().0, 1);
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.composer.text(), "new first\nnew middle\nnew last");
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.composer.text(), scratch);
        assert!(!app.composer.is_recalling());
        assert!(app.following);
        assert_eq!(app.scroll_back, 0);
    }

    #[test]
    fn line_editing_keys_use_unicode_safe_line_and_word_boundaries() {
        let mut app = app();
        app.composer.replace("header\ncafé 中文 tail\nfooter");
        app.composer.move_to_position(1, 2);
        app.on_key(ctrl('a'));
        assert_eq!(app.composer.cursor_position(), (1, 0));
        app.on_key(ctrl('e'));
        assert_eq!(app.composer.cursor_position(), (1, 14));

        app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        assert_eq!(app.composer.cursor_position(), (1, 10));
        app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        assert_eq!(app.composer.cursor_position(), (1, 5));
        app.on_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT));
        assert_eq!(app.composer.cursor_position(), (1, 9));
        app.on_key(ctrl('w'));
        assert_eq!(app.composer.text(), "header\ncafé  tail\nfooter");
        assert_eq!(app.composer.cursor_position(), (1, 5));
        app.on_key(ctrl('u'));
        assert_eq!(app.composer.text(), "header\n tail\nfooter");
        assert_eq!(app.composer.cursor_position(), (1, 0));
        app.on_key(ctrl('k'));
        assert_eq!(app.composer.text(), "header\n\nfooter");
        assert_eq!(app.composer.cursor_position(), (1, 0));
        app.on_key(ctrl('u'));
        app.on_key(ctrl('k'));
        assert_eq!(app.composer.text(), "header\n\nfooter");
    }

    #[test]
    fn new_line_editing_keys_are_harmless_on_an_empty_draft() {
        let mut app = app();
        app.sync_scroll_limit(40);
        app.scroll_up(10);
        for event in [
            ctrl('a'),
            ctrl('e'),
            ctrl('w'),
            ctrl('u'),
            ctrl('k'),
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
        ] {
            assert_eq!(app.on_key(event), None);
            assert!(app.composer.is_empty());
            assert_eq!(app.composer.cursor(), 0);
            assert_eq!(app.scroll_back, 10);
            assert!(app.overlay.is_none());
        }
        assert_eq!(app.on_key(ctrl('b')), Some(Action::BackgroundShell));
    }

    #[test]
    fn resource_picker_keeps_its_ctrl_u_filter_binding() {
        let mut app = app();
        type_text(&mut app, "/model");
        app.on_key(key(KeyCode::Enter));
        type_text(&mut app, "missing");
        assert!(
            matches!(&app.overlay, Some(Overlay::ResourcePicker { picker, .. })
            if picker.query == "missing")
        );
        app.on_key(ctrl('u'));
        assert!(
            matches!(&app.overlay, Some(Overlay::ResourcePicker { picker, .. })
            if picker.query.is_empty())
        );
        assert!(app.composer.is_empty());
    }

    #[test]
    fn home_and_end_use_draft_edges_when_non_empty_and_transcript_edges_when_empty() {
        let mut app = app();
        app.sync_scroll_limit(40);
        app.scroll_up(10);
        for draft in ["first\nsecond\nlast", " \n "] {
            app.composer.replace(draft);
            app.on_key(key(KeyCode::Home));
            assert_eq!(app.composer.cursor(), 0);
            assert_eq!(app.scroll_back, 10);
            app.on_key(key(KeyCode::End));
            assert_eq!(app.composer.cursor(), app.composer.len());
            assert_eq!(app.scroll_back, 10);
            assert!(!app.following);
        }
        app.composer.clear();
        app.on_key(key(KeyCode::Home));
        assert_eq!(app.scroll_back, 40);
        app.on_key(key(KeyCode::End));
        assert_eq!(app.scroll_back, 0);
        assert!(app.following);
    }

    #[test]
    fn newline_keys_edit_the_draft_without_submitting_or_recording_history() {
        for modifiers in [KeyModifiers::SHIFT, KeyModifiers::ALT] {
            let mut app = app();
            app.composer.replace("café tail");
            app.composer.move_to_position(0, 4);
            assert_eq!(app.on_key(KeyEvent::new(KeyCode::Enter, modifiers)), None);
            assert_eq!(app.composer.text(), "café\n tail");
            assert_eq!(app.composer.cursor_position(), (1, 0));
            assert!(!app.composer.recall_previous());
        }
        let mut app = app();
        app.composer.replace("café\\ tail");
        app.composer.move_to_position(0, 5);
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(app.composer.text(), "café\n tail");
        assert_eq!(app.composer.cursor_position(), (1, 0));
        assert!(!app.composer.recall_previous());

        app.composer.replace("\\");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(app.composer.text(), "\n");
        app.composer.replace("!printf hello\\");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(app.composer.text(), "!printf hello\n");
    }

    #[test]
    fn word_and_line_keys_keep_registered_paste_and_image_labels_atomic() {
        let mut app = app();
        app.on_paste("one\ntwo\nthree");
        app.attach_image("data:image/png;base64,IMAGE".into(), 32, 32);
        let labels = app.composer.text().to_owned();
        let paste = "[Pasted text #1 +3 lines]";
        let image = "[Image #1 32×32]";
        assert_eq!(labels, format!("{paste}{image}"));
        for expected in [paste.chars().count(), 0] {
            app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
            assert_eq!(app.composer.cursor(), expected);
        }
        for expected in [paste.chars().count(), labels.chars().count()] {
            app.on_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT));
            assert_eq!(app.composer.cursor(), expected);
        }
        app.on_key(ctrl('w'));
        assert_eq!(app.composer.text(), paste);
        app.on_key(ctrl('w'));
        assert!(app.composer.is_empty());

        app.composer.replace(format!("{labels} tail"));
        app.on_key(key(KeyCode::Home));
        app.on_key(ctrl('k'));
        assert!(app.composer.is_empty());
        app.composer.replace(format!("head {labels}"));
        app.on_key(ctrl('u'));
        assert!(app.composer.is_empty());
    }

    #[test]
    fn arrow_history_restores_a_non_empty_composer_scratch() {
        let mut app = app();
        type_text(&mut app, "completed input");
        app.on_key(key(KeyCode::Enter));
        type_text(&mut app, "work in progress");

        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "completed input");
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.composer.text(), "work in progress");
    }

    #[test]
    fn arrow_keys_only_navigate_composer_history() {
        let mut app = app();
        app.sync_scroll_limit(40);

        app.on_key(key(KeyCode::Up));
        app.on_key(key(KeyCode::Down));

        assert!(app.following);
        assert_eq!(app.scroll_back, 0);
    }

    #[test]
    fn page_keys_scroll_the_transcript_without_touching_the_draft() {
        let mut app = app();
        app.sync_scroll_limit(40);
        type_text(&mut app, "keep drafting");

        app.on_key(key(KeyCode::PageUp));
        assert_eq!(app.composer.text(), "keep drafting");
        assert_eq!(app.scroll_back, 10);
        assert!(!app.following);

        app.on_key(key(KeyCode::PageDown));
        assert_eq!(app.composer.text(), "keep drafting");
        assert_eq!(app.scroll_back, 0);
        assert!(app.following);
    }

    #[test]
    fn the_mouse_wheel_scrolls_the_transcript_without_touching_the_draft() {
        let mut app = app();
        app.sync_scroll_limit(40);
        type_text(&mut app, "keep drafting");

        assert_eq!(
            app.on_mouse(mouse(MouseEventKind::ScrollUp, 0, 0)),
            MouseOutcome::Redraw
        );
        assert_eq!(app.composer.text(), "keep drafting");
        assert_eq!(app.scroll_back, 3);
        assert!(!app.following);

        assert_eq!(
            app.on_mouse(mouse(MouseEventKind::ScrollDown, 0, 0)),
            MouseOutcome::Redraw
        );
        assert_eq!(app.scroll_back, 0);
        assert!(app.following);
    }

    #[test]
    fn a_left_drag_selects_and_asks_the_host_to_copy_on_release() {
        let mut app = app();

        assert_eq!(
            app.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 4, 2)),
            MouseOutcome::Redraw
        );
        assert_eq!(
            app.on_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 9, 3)),
            MouseOutcome::Redraw
        );
        assert_eq!(
            app.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 9, 3)),
            MouseOutcome::CopySelection
        );

        let selection = app.selection.expect("a finished selection stays painted");
        assert!(!selection.dragging());
        assert_eq!(selection.span_on_row(2, Rect::new(0, 0, 20, 6)), Some((4, 20)));
        assert_eq!(selection.span_on_row(3, Rect::new(0, 0, 20, 6)), Some((0, 10)));
    }

    #[test]
    fn a_release_past_the_last_drag_report_still_selects_through_it() {
        // A quick flick: the terminal sends the press and the release but
        // never a drag position in between. Reading only drags would copy
        // nothing at all.
        let mut app = app();
        app.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 3, 0));

        assert_eq!(
            app.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 11, 0)),
            MouseOutcome::CopySelection
        );
        let selection = app.selection.expect("the release defines the far end");
        assert_eq!(
            selection.span_on_row(0, Rect::new(0, 0, 20, 4)),
            Some((3, 12))
        );
    }

    #[test]
    fn a_click_that_never_dragged_dismisses_the_highlight_without_copying() {
        let mut app = app();
        app.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 4, 2));
        app.on_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 9, 3));
        app.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 9, 3));
        assert!(app.selection.is_some());

        assert_eq!(
            app.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 1, 1)),
            MouseOutcome::Redraw
        );
        assert_eq!(
            app.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 1, 1)),
            MouseOutcome::Redraw
        );
        assert!(app.selection.is_none(), "a bare click clears the highlight");
    }

    #[test]
    fn scrolling_drops_a_selection_that_would_otherwise_mark_moved_text() {
        let mut app = app();
        app.sync_scroll_limit(40);
        app.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 1));
        app.on_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 8, 1));
        app.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 8, 1));
        assert!(app.selection.is_some());

        app.on_mouse(mouse(MouseEventKind::ScrollUp, 0, 0));

        assert!(app.selection.is_none());
    }

    #[test]
    fn a_drag_whose_press_was_never_seen_has_no_anchor_to_grow_from() {
        let mut app = app();

        assert_eq!(
            app.on_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 5, 5)),
            MouseOutcome::Ignored
        );
        assert_eq!(
            app.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 5, 5)),
            MouseOutcome::Ignored
        );
        assert!(app.selection.is_none());
    }

    #[test]
    fn a_right_button_press_is_left_to_the_terminal() {
        let mut app = app();

        assert_eq!(
            app.on_mouse(mouse(MouseEventKind::Down(MouseButton::Right), 3, 3)),
            MouseOutcome::Ignored
        );
        assert!(app.selection.is_none());
    }

    #[test]
    fn ctrl_r_search_cycles_accepts_and_cancels_losslessly() {
        let mut app = app();
        for input in ["fix older", "unrelated", "fix newest"] {
            type_text(&mut app, input);
            app.on_key(key(KeyCode::Enter));
        }
        type_text(&mut app, "scratch draft");

        app.on_key(ctrl('r'));
        type_text(&mut app, "FIX");
        assert!(matches!(
            &app.overlay,
            Some(Overlay::HistorySearch { matched, .. })
                if matched.as_deref() == Some("fix newest")
        ));
        app.on_key(ctrl('r'));
        assert!(matches!(
            &app.overlay,
            Some(Overlay::HistorySearch { matched, .. })
                if matched.as_deref() == Some("fix older")
        ));
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.composer.text(), "scratch draft");

        app.on_key(ctrl('r'));
        type_text(&mut app, "fix");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(app.overlay.is_none());
        assert_eq!(app.composer.text(), "fix newest");

        app.on_key(ctrl('r'));
        type_text(&mut app, "missing");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(&app.overlay, Some(Overlay::HistorySearch { .. })));
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.composer.text(), "fix newest");
    }

    #[test]
    fn ctrl_c_from_reverse_search_stashes_the_original_draft() {
        let mut app = app();
        type_text(&mut app, "recover through search");
        app.on_key(ctrl('r'));
        type_text(&mut app, "query");

        assert_eq!(app.on_key(ctrl('c')), None);
        assert!(app.overlay.is_none());
        assert!(app.composer.is_empty());
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "recover through search");
    }

    #[test]
    fn ctrl_r_does_not_steal_an_existing_overlay() {
        let mut app = app();
        type_text(&mut app, "/model");
        app.on_key(key(KeyCode::Enter));
        assert!(matches!(
            &app.overlay,
            Some(Overlay::ResourcePicker {
                target: ResourceTarget::Model,
                ..
            })
        ));

        app.on_key(ctrl('r'));
        assert!(matches!(
            &app.overlay,
            Some(Overlay::ResourcePicker {
                target: ResourceTarget::Model,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn denying_marks_the_tool_row_denied() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("c1"),
            name: "shell".into(),
            argument_keys: vec!["command".into()],
            argument_fingerprint: fingerprint("arguments"),
            arguments: None,
        }));
        app.present_approval(prompt("shell").await);
        elapse_prompt_guard(&mut app);
        app.on_key(key(KeyCode::Char('n')));

        match &app.transcript.blocks()[0] {
            Block::Tool {
                status,
                display,
                protected_summary,
                ..
            } => {
                assert_eq!(*status, ToolStatus::Denied);
                assert!(display.is_none());
                assert_eq!(protected_summary, "arguments hidden");
            }
            other => panic!("expected a tool block, got {other:?}"),
        }
    }

    #[test]
    fn scrolling_up_pauses_following_until_the_user_returns() {
        let mut app = app();
        app.sync_scroll_limit(30);
        assert!(app.following);

        app.on_key(key(KeyCode::PageUp));
        assert!(!app.following);
        assert_eq!(app.scroll_back, 10);

        app.on_key(key(KeyCode::End));
        assert!(app.following);
        assert_eq!(app.scroll_back, 0);

        app.on_key(key(KeyCode::Home));
        assert!(!app.following);
        assert_eq!(app.scroll_back, 30);

        app.on_key(ctrl('l'));
        assert!(app.following);
        assert_eq!(app.scroll_back, 0);
    }

    #[test]
    fn scrolling_without_overflow_keeps_following_newest() {
        let mut app = app();

        app.on_key(key(KeyCode::PageUp));
        app.on_key(key(KeyCode::Home));

        assert!(app.following);
        assert_eq!(app.scroll_back, 0);
    }

    #[test]
    fn paused_scrolling_keeps_the_visible_offset_when_content_grows() {
        let mut app = app();
        app.sync_scroll_limit(20);
        app.scroll_up(5);

        app.sync_scroll_limit(30);

        assert!(!app.following);
        assert_eq!(app.scroll_back, 15);
    }

    #[test]
    fn tab_never_changes_regions_and_completes_without_execution() {
        let mut app = app();
        app.on_key(key(KeyCode::Tab));
        assert!(app.composer.is_empty());

        type_text(&mut app, "/sta");
        assert!(matches!(app.overlay, Some(Overlay::Palette { .. })));
        assert_eq!(app.on_key(key(KeyCode::Tab)), None);
        assert_eq!(app.composer.text(), "/status");
        assert!(app.overlay.is_none());
        assert!(app.transcript.is_empty(), "completion must not execute");

        app.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(app.composer.text(), "/status");
    }

    #[test]
    fn dismissing_ctrl_p_restores_the_original_draft() {
        let mut app = app();
        type_text(&mut app, "keep this");
        app.on_key(ctrl('p'));
        assert!(app.composer.text().starts_with('/'));
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.composer.text(), "keep this");
        assert!(app.overlay.is_none());
    }

    #[test]
    fn context_planning_telemetry_becomes_bounded_status_state() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::ContextPlanned {
            context: fingerprint("context"),
            cache_plan: fingerprint("cache"),
            segment_count: 2,
            totals: BTreeMap::from([
                (SegmentKind::new("history"), 1_500),
                (SegmentKind::new("tool_schema"), 500),
            ]),
            input_tokens: 2_000,
            input_budget_tokens: 10_000,
            reserved_tokens: 2_000,
            confidence: EstimationConfidence::Estimated,
        }));

        let plan = app.status.context_plan.as_ref().expect("context plan");
        assert_eq!(plan.fingerprint, fingerprint("context").as_str());
        assert_eq!(plan.cache_fingerprint, fingerprint("cache").as_str());
        assert_eq!(plan.input_tokens, 2_000);
        assert_eq!(plan.input_budget_tokens, 10_000);
        assert_eq!(plan.reserved_tokens, 2_000);
        assert_eq!(plan.segment_count, 2);
        assert_eq!(plan.totals["history"], 1_500);
        assert_eq!(plan.render_footer(), "~80% ctx");
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut app = app();
        let mut release = key(KeyCode::Char('x'));
        release.kind = KeyEventKind::Release;
        app.on_key(release);
        assert!(app.composer.is_empty());
    }

    #[test]
    fn ctrl_o_and_details_share_the_same_toggle_without_changing_the_draft() {
        let mut app = app();
        app.composer.replace("keep this draft");
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            None
        );
        assert!(app.work_details);
        assert_eq!(app.composer.text(), "keep this draft");
        app.composer.replace("/details");
        app.on_key(key(KeyCode::Enter));
        assert!(!app.work_details);
        assert!(app.composer.is_empty());
        app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert!(app.work_details);
    }

    #[tokio::test]
    async fn ctrl_o_expands_while_a_guarded_approval_is_open_without_answering() {
        let mut app = app();
        app.present_approval(prompt("shell").await);
        assert_eq!(app.pending_approval_count(), 1);
        app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert!(app.work_details);
        assert_eq!(app.pending_approval_count(), 1);
        assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
        app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert!(!app.work_details);
        assert_eq!(app.pending_approval_count(), 1);
    }
