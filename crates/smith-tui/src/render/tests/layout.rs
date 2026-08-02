// layout behavior tests.

    #[test]
    fn codex_formatting_uses_quiet_markers_and_semantic_text_styles() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.transcript.push_user("hello");
        app.transcript.push_text_delta(
            "# Heading\nUse `cargo test`, **care**, and [the docs](https://example.com).",
        );
        app.transcript
            .push_reasoning_delta("Checking the workspace.", false);
        app.transcript
            .push_tool_call("c1", "read", None, &["path".to_owned()]);
        app.transcript.complete_tool_call("c1", ToolStatus::Ok);
        app.show_local_result("status", "session: s1\nmodel: gpt-5.3");

        let lines = transcript_lines(&app, Theme::new(), 80);
        let find_line = |needle: &str| {
            lines
                .iter()
                .find(|line| line.spans.iter().any(|span| span.content.contains(needle)))
                .unwrap_or_else(|| panic!("missing `{needle}` in {lines:#?}"))
        };

        let user = find_line("hello");
        assert_eq!(user.spans[0].content, "› ");
        assert!(
            user.spans[0]
                .style
                .add_modifier
                .contains(Modifier::DIM | Modifier::BOLD)
        );
        assert_eq!(user.spans[1].style.fg, None);

        let heading = find_line("Heading");
        assert_eq!(heading.spans[0].content, "• ");
        assert!(heading.spans[0].style.add_modifier.contains(Modifier::DIM));
        assert!(
            heading.spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );

        let prose = find_line("cargo test");
        let code = prose
            .spans
            .iter()
            .find(|span| span.content == "cargo test")
            .expect("inline code span");
        assert_eq!(code.style.fg, Some(Color::Cyan));
        let link = prose
            .spans
            .iter()
            .find(|span| span.content == "the docs")
            .expect("link span");
        assert_eq!(link.style.fg, Some(Color::Cyan));
        assert!(link.style.add_modifier.contains(Modifier::UNDERLINED));

        assert!(
            lines.iter().all(|line| line
                .spans
                .iter()
                .all(|span| !span.content.contains("Checking the workspace."))),
            "closed reasoning must not render as assistant prose: {lines:#?}"
        );

        let tool = find_line("read(path · details unavailable)");
        assert_eq!(tool.spans[0].style.fg, Some(Color::Green));
        assert!(tool.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert!(tool.spans[1].style.add_modifier.contains(Modifier::BOLD));

        let command = find_line("/status");
        assert_eq!(command.spans[0].style.fg, Some(Color::Magenta));
        let border = find_line("╭");
        assert!(border.spans[0].style.add_modifier.contains(Modifier::DIM));
        let status = find_line("session:");
        assert!(status.spans[1].style.add_modifier.contains(Modifier::DIM));
        assert_eq!(status.spans[2].style.fg, None);
    }

    #[test]
    fn successful_and_non_success_terminals_stay_visible_at_all_widths() {
        for (width, height) in [(44, 14), (74, 24), (120, 32)] {
            let theme = Theme::new().without_color().without_motion();

            let mut completed = App::new("gpt-5.3", "~/work/api");
            completed.apply(&event_at(Timestamp(1_000), RuntimeEvent::TurnStarted));
            completed.transcript.push_text_delta("Committed answer.");
            completed.apply(&event_at(
                Timestamp(1_842),
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ));
            let completed_screen = render(&completed, width, height, theme);
            assert!(
                completed_screen.contains("Committed answer."),
                "{width}x{height}: {completed_screen}"
            );
            assert!(
                completed_screen.contains("Worked for 842ms"),
                "{width}x{height}: {completed_screen}"
            );
            assert!(
                !completed_screen.contains("reasoning only")
                    && !completed_screen.contains("Working…"),
                "{width}x{height}: {completed_screen}"
            );

            let mut interrupted = App::new("gpt-5.3", "~/work/api");
            interrupted.apply(&event(RuntimeEvent::TurnStarted));
            interrupted.apply(&event(RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Cancelled {
                    reason: CancelReason::UserRequested,
                },
                visible_output: false,
            }));
            let interrupted_screen = render(&interrupted, width, height, theme);
            assert!(
                interrupted_screen.contains("Interrupted"),
                "{width}x{height}: {interrupted_screen}"
            );
        }
    }

    #[test]
    fn speculative_text_streams_unlabelled_and_survives_the_commit() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::TextDelta {
            request: RequestId::new("request-draft"),
            attempt: AttemptId::new("attempt-draft"),
            text: "tentative answer".into(),
        }));

        let speculative = render(&app, 74, 12, Theme::new());
        assert!(speculative.contains("tentative answer"), "{speculative}");
        assert!(!speculative.contains("draft ·"), "{speculative}");
        assert!(app.transcript.is_empty());

        app.apply(&event(RuntimeEvent::ProviderAttemptOutputCommitted {
            request: RequestId::new("request-draft"),
            attempt: AttemptId::new("attempt-draft"),
        }));
        let committed = render(&app, 74, 12, Theme::new());
        assert!(committed.contains("tentative answer"), "{committed}");
        assert!(!committed.contains("draft ·"), "{committed}");
    }

    #[test]
    fn a_tiny_terminal_refuses_rather_than_half_renders() {
        let app = conversation();
        let screen = render(&app, 30, 8, Theme::new());
        assert!(screen.contains("terminal too small"), "{screen}");
        assert!(!screen.contains("retry policy"), "{screen}");
    }

    #[test]
    fn agent_first_idle_snapshots_are_accessible_at_supported_sizes() {
        let mut app = App::new("glm-5.2", "api:main");
        app.status.switch_model(Some("zai".to_owned()), "glm-5.2");
        app.status.set_agent("build");
        let theme = Theme::new().without_color().without_motion();

        for (width, height) in [(44, 14), (74, 24), (120, 32)] {
            let screen = render(&app, width, height, theme);
            assert!(screen.contains("build"), "{width}×{height}:\n{screen}");
            assert!(screen.contains("glm-5.2"), "{width}×{height}:\n{screen}");
            assert!(screen.contains("? ctx"), "{width}×{height}:\n{screen}");
            assert!(
                !screen.contains("Tab agents"),
                "{width}×{height}:\n{screen}"
            );
            assert!(
                !screen.contains("Ctrl+P commands"),
                "{width}×{height}:\n{screen}"
            );
            assert!(
                screen
                    .lines()
                    .all(|line| line.width() <= usize::from(width)),
                "{width}×{height} overflowed:\n{screen}"
            );
            assert!(!screen.contains('\u{1b}'), "ANSI leaked:\n{screen:?}");
            assert_eq!(
                screen.matches("zai/glm-5.2").count(),
                1,
                "identity became permanent chrome:\n{screen}"
            );
        }

        let normal = render(&app, 74, 24, theme);
        insta_like(&normal, &["build · zai/glm-5.2 · api:main · ? ctx"]);

        app.apply(&event(RuntimeEvent::TurnStarted));
        let reduced_motion = render(&app, 74, 24, theme);
        assert!(reduced_motion.contains("Working"), "{reduced_motion}");
        assert!(
            reduced_motion.contains(glyph::STILL),
            "reduced motion did not use the static activity marker:\n{reduced_motion}"
        );
    }

    #[test]
    fn resizing_recomputes_the_layout_without_corrupting_history() {
        let app = conversation();
        for (width, height) in [(74, 16), (44, 12), (120, 40), (74, 16)] {
            let screen = render(&app, width, height, Theme::new());
            assert!(
                screen.contains("gpt-5.3"),
                "{width}×{height} lost the footer:\n{screen}"
            );
        }
    }

    #[test]
    fn the_paused_follow_indicator_appears_only_when_scrolled_back() {
        let mut app = conversation();
        assert!(!render_synced(&mut app, 74, 10, Theme::new()).contains("following paused"));
        app.scroll_up(3);
        let paused = render_synced(&mut app, 74, 10, Theme::new());
        assert!(paused.contains("following paused"), "{paused}");
        assert!(paused.contains("End/Ctrl+L newest"), "{paused}");
    }

    #[test]
    fn following_keeps_the_newest_word_wrapped_rows_visible() {
        // Words just over half the width wrap one-per-row, so word wrapping
        // emits far more rows than a character-wrap estimate; a drifting
        // estimate under-scrolls and clips exactly the newest rows.
        let mut app = App::new("gpt-5.3", "~/work/api");
        let word = "a".repeat(23);
        for _ in 0..4 {
            app.transcript
                .push_user(format!("{word} ").repeat(8).trim_end().to_owned());
        }
        app.transcript.push_notice("marker", "newest-entry");
        assert!(app.following);

        let screen = render(&app, 44, 14, Theme::new().without_color());
        assert!(screen.contains("newest-entry"), "{screen}");
    }
