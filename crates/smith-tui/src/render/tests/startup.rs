// Startup orientation and local help use the ordinary terminal layout.

#[test]
fn startup_guide_is_readable_without_becoming_transcript_history() {
    let app = App::new("example-model", "~/project");
    let theme = Theme::new().without_color().without_motion();
    for (width, height) in [(100, 32), (80, 24), (44, 16), (40, 10)] {
        let screen = render(&app, width, height, theme);
        insta_like(
            &screen,
            &[
                "Get started",
                "Type a task below and press Enter.",
                "/model",
                "/connect",
                "/help",
            ],
        );
        assert!(screen.lines().all(|line| line.width() <= usize::from(width)));
    }
    assert!(app.transcript.is_empty());
    assert!(app.composer.is_empty());
}

#[test]
fn startup_guide_yields_to_conversation_and_active_work() {
    let theme = Theme::new().without_color();
    let mut conversation = App::new("example-model", "~/project");
    conversation.transcript.push_user("Inspect this project");
    let screen = render(&conversation, 80, 24, theme);
    assert!(screen.contains("Inspect this project"));
    assert!(!screen.contains("Get started"));

    let mut working = App::new("example-model", "~/project");
    working.apply(&event(RuntimeEvent::TurnStarted));
    let screen = render(&working, 80, 24, theme);
    assert!(screen.contains("Working"));
    assert!(!screen.contains("Get started"));
}

fn open_startup_help(app: &mut App, shortcut: bool) {
    if shortcut {
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)),
            None
        );
    } else {
        for character in "/help".chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            None
        );
    }
}

#[test]
fn help_opens_at_its_new_result_after_wrapped_conversation() {
    let theme = Theme::new().without_color();
    for (width, height) in [(100, 32), (80, 24), (44, 16)] {
        for shortcut in [false, true] {
            let mut app = App::new("example-model", "~/project");
            app.transcript.push_user("Earlier conversation ".repeat(100));
            app.transcript.push_text_delta("Earlier answer ".repeat(60).as_str());
            // Suppressed reasoning must not shift the help anchor.
            app.transcript.push_reasoning_delta("Hidden reasoning", false);
            render_synced(&mut app, width, height, theme);
            open_startup_help(&mut app, shortcut);

            let preview = render(&app, width, height, theme);
            assert_eq!(preview.lines().next(), Some("/help"), "{preview}");
            let screen = render_synced(&mut app, width, height, theme);
            assert_eq!(screen.lines().next(), Some("/help"), "{screen}");
            assert!(screen.contains("Getting started"), "{screen}");
            assert!(!screen.contains("Earlier conversation"), "{screen}");
            assert!(!app.following);
            assert!(app.scroll_to_block.is_none());

            app.scroll_down(3);
            let scrolled = render_synced(&mut app, width, height, theme);
            assert_ne!(screen, scrolled);
            app.on_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
            let newest = render_synced(&mut app, width, height, theme);
            assert!(app.following);
            assert!(newest.contains("Start a message with //"), "{newest}");

            app.show_local_result("status", "Fresh local result");
            let subsequent = render_synced(&mut app, width, height, theme);
            assert!(subsequent.contains("Fresh local result"), "{subsequent}");
            assert!(app.following);
        }
    }
}

#[test]
fn help_anchor_survives_a_too_small_frame_and_can_be_overridden() {
    let theme = Theme::new().without_color();
    let mut app = App::new("example-model", "~/project");
    open_startup_help(&mut app, true);
    let small = render_synced(&mut app, 39, 9, theme);
    assert!(small.contains("terminal too small"));
    assert!(app.scroll_to_block.is_some());
    let restored = render_synced(&mut app, 80, 24, theme);
    assert_eq!(restored.lines().next(), Some("/help"), "{restored}");

    open_startup_help(&mut app, true);
    app.on_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
    assert!(app.scroll_to_block.is_none());
    let newest = render_synced(&mut app, 80, 24, theme);
    assert!(newest.contains("Start a message with //"), "{newest}");
    assert!(app.following);
}

#[test]
fn explicit_scroll_cancels_help_anchor_before_a_valid_frame() {
    let mut app = App::new("example-model", "~/project");
    open_startup_help(&mut app, true);
    render_synced(&mut app, 39, 9, Theme::new());
    app.scroll_up(1);
    assert!(app.scroll_to_block.is_none());
}

#[test]
fn command_palette_scrolls_its_five_rows_to_the_selection() {
    let mut app = App::new("example-model", "~/project");
    app.transcript.push_user("Existing conversation stays visible");
    app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    for (width, height) in [(100, 32), (80, 24), (44, 16)] {
        let screen = render(&app, width, height, Theme::new().without_color());
        assert!(screen.contains("Existing conversation stays visible"), "{screen}");
        assert!(screen.contains("› /quit"), "{screen}");
        let menu_rows = screen
            .lines()
            .filter(|line| {
                line.starts_with("  /") || (line.starts_with("› /") && *line != "› /")
            })
            .count();
        assert_eq!(menu_rows, 5, "{screen}");
        assert!(!screen.contains("/help"), "{screen}");
    }
}
