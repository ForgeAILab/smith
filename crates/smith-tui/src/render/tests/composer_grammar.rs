#[test]
fn bash_cursor_maps_visible_unicode_lines_and_the_prompt_without_color() {
    use ratatui::backend::Backend as _;

    for width in [44, 80, 100] {
        let mut app = App::new("model", "project");
        app.composer.replace("!é中\nnext");
        for (line, column, stored) in [(0, 0, 1), (0, 1, 2), (0, 3, 3), (1, 2, 6)] {
            app.composer.move_to_visible_position(line, column);
            assert_eq!(app.composer.cursor(), stored);
            assert_eq!(app.composer.visible_cursor_position(), (line, column));
            let mut terminal = Terminal::new(TestBackend::new(width, 16)).unwrap();
            terminal
                .draw(|frame| draw(frame, &app, Theme::new().without_color()))
                .unwrap();
            let screen = screen_text(terminal.backend().buffer());
            let top = screen.lines().position(|row| row.starts_with('─')).unwrap();
            let cursor = terminal.backend_mut().get_cursor_position().unwrap();
            assert_eq!(
                (usize::from(cursor.x), usize::from(cursor.y)),
                (column + 2, top + 1 + line)
            );
            assert!(screen.contains("! é中"), "{screen}");
            assert!(screen.contains("  next"), "{screen}");
            assert!(
                terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
            );
        }
        app.composer.move_to_start();
        app.composer.move_left();
        let (_, cursor) = composer_content(&app, Theme::new(), width);
        assert_eq!(app.composer.cursor(), 0);
        assert_eq!(cursor, Some((0, 0)), "the stored bang is the prompt cell");
        app.on_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let screen = render(&app, width, 16, Theme::new().without_color());
        assert!(screen.contains("> é中"), "{screen}");
        assert!(!screen.contains("bash mode"), "{screen}");
    }
}

#[test]
fn bash_home_and_control_a_backspace_restore_the_normal_prompt() {
    for width in [44, 80, 100] {
        for start in [
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
        ] {
            let mut app = App::new("model", "project");
            app.composer.replace("!ls");
            app.on_key(start);
            assert_eq!(app.composer.cursor(), 1);
            assert_eq!(app.composer.visible_cursor_position(), (0, 0));
            assert_eq!(composer_content(&app, Theme::new(), width).1, Some((2, 0)));
            app.on_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
            assert_eq!(app.composer.text(), "ls");
            let screen = render(&app, width, 16, Theme::new().without_color());
            assert!(screen.lines().any(|line| line == "> ls"), "{screen}");
            assert!(!screen.contains("bash mode"), "{screen}");
            assert_eq!(composer_content(&app, Theme::new(), width).1, Some((2, 0)));
        }
    }
}

#[test]
fn bash_wrapping_and_height_use_the_same_visible_text_and_cursor_cell() {
    for width in [44, 80, 100] {
        let mut app = App::new("model", "project");
        app.composer
            .replace(format!("!{}", "x".repeat(usize::from(width) - 2)));
        app.composer.move_to_start();
        assert_eq!(
            composer_rows(&app, width),
            1,
            "the hidden bang must not add a wrap"
        );
        app.composer.move_to_end();
        let (rows, cursor) = composer_content(&app, Theme::new(), width);
        assert_eq!(rows.len(), 2, "reserve the cursor cell after a full row");
        assert_eq!(cursor, Some((0, 1)));
        assert_eq!(composer_rows(&app, width), 2);
        app.composer
            .replace(format!("!{}", "x".repeat(usize::from(width) + 3)));
        let (rows, cursor) = composer_content(&app, Theme::new(), width);
        assert_eq!(rows.len(), 2);
        assert_eq!(cursor, Some((5, 1)));
    }
}

#[test]
fn bash_pointer_selection_copies_visible_cells_and_keeps_stored_submission() {
    for width in [44, 80, 100] {
        for (draft, visible) in [("!é中", "é中"), ("!!x", "!x")] {
            let mut app = App::new("model", "project");
            app.composer.replace(draft);
            let screen = render_synced(&mut app, width, 16, Theme::new().without_color());
            let row = u16::try_from(
                screen
                    .lines()
                    .position(|line| line.starts_with("! "))
                    .unwrap(),
            )
            .unwrap();
            drag(
                &mut app,
                (2, row),
                (2 + u16::try_from(visible.width()).unwrap(), row),
            );
            let (_, copied) = render_and_copy(&mut app, width, 16);
            assert_eq!(copied.as_deref(), Some(visible));
            assert_eq!(app.composer.text(), draft);
            drag(
                &mut app,
                (0, row),
                (2 + u16::try_from(visible.width()).unwrap(), row),
            );
            let (_, copied) = render_and_copy(&mut app, width, 16);
            assert_eq!(copied.as_deref(), Some(format!("! {visible}").as_str()));
        }
    }
}

#[test]
fn hint_row_states_and_identity_drop_order_at_44_80_and_100_columns() {
    for theme in [Theme::new(), Theme::new().without_color()] {
        for width in [44, 80, 100] {
            let mut app = App::new("model", "secondary-project");
            app.status.set_agent("dev");
            app.status.approval_mode = Some("ask".to_owned());
            let screen = render(&app, width, 16, theme);
            let footer = screen.lines().last().unwrap();
            assert!(footer.starts_with("  ? for shortcuts"), "{footer}");
            assert!(footer.contains("model · dev · ask"), "{footer}");
            assert!(
                footer.ends_with(if width == 44 { "ask" } else { "unknown ctx" }),
                "{footer}"
            );
            assert_eq!(
                footer.contains("secondary-project"),
                width != 44,
                "{footer}"
            );

            app.composer.replace("ordinary draft");
            let screen = render(&app, width, 16, theme);
            assert!(!screen.lines().last().unwrap().contains("? for shortcuts"));
            app.composer.replace("!ls");
            let screen = render(&app, width, 16, theme);
            assert!(screen.lines().last().unwrap().starts_with("  bash mode"));
            app.composer.clear();
            app.apply(&event(RuntimeEvent::TurnStarted));
            for draft in ["", "steer this"] {
                app.composer.replace(draft);
                let screen = render(&app, width, 16, theme);
                let footer = screen.lines().last().unwrap();
                let hint = if width == 44 {
                    "enter steer · tab queue · esc interrupt"
                } else {
                    "enter to steer · tab to queue · esc to interrupt"
                };
                assert!(footer.starts_with(&format!("  {hint}")), "{footer}");
                assert_eq!(
                    footer.contains("model · dev · ask"),
                    width != 44,
                    "{footer}"
                );
                assert_eq!(
                    footer.contains("secondary-project"),
                    width == 100,
                    "{footer}"
                );
                assert!(footer.width() <= usize::from(width));
            }
        }
    }
}

#[test]
fn shortcuts_are_anchored_and_ephemeral_with_the_same_rows_as_help() {
    for theme in [Theme::new(), Theme::new().without_color()] {
        for (width, height) in [(44, 16), (80, 24), (100, 32)] {
            let mut app = App::new("model", "project");
            app.transcript.push_user("Keep this conversation");
            let before = transcript_lines(&app, theme, width);
            let help = crate::commands::help();
            let guide = render_help_report(&help, width, theme);
            let keys_heading = guide
                .iter()
                .position(|line| line.to_string().trim() == "Keys")
                .unwrap();
            assert_eq!(
                &shortcuts_lines(width, theme)[1..],
                &guide[keys_heading + 1..]
            );
            assert_eq!(
                app.on_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)),
                None
            );
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app, theme)).unwrap();
            let screen = screen_text(terminal.backend().buffer());
            let panel = screen
                .lines()
                .position(|line| line.trim() == "Shortcuts")
                .unwrap();
            let composer = screen
                .lines()
                .enumerate()
                .filter_map(|(row, line)| line.starts_with("> ").then_some(row))
                .last()
                .unwrap();
            assert!(panel < composer, "{screen}");
            assert!(screen.contains("Send a task"), "{screen}");
            assert!(screen.contains("any key closes"), "{screen}");
            assert_eq!(transcript_lines(&app, theme, width), before);
            assert_eq!(app.transcript.blocks().len(), 1);
            assert!(app.composer.is_empty());
            if !theme.uses_color() {
                assert!(
                    terminal
                        .backend()
                        .buffer()
                        .content
                        .iter()
                        .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
                );
            }
            app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert!(!render(&app, width, height, theme).contains("Shortcuts"));
        }
    }
}

#[test]
fn hint_row_keeps_existing_picker_controls_at_all_widths() {
    for width in [44, 80, 100] {
        let mut app = App::new("model", "project");
        app.status.approval_mode = Some("ask".to_owned());
        app.set_resources(crate::app::RuntimeResources {
            files: vec![crate::picker::ResourceEntry::new(
                "file:src/lib.rs",
                "src/lib.rs",
                "file",
            )],
            ..crate::app::RuntimeResources::default()
        });
        app.on_key(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE));
        let screen = render(&app, width, 16, Theme::new().without_color());
        let rows: Vec<_> = screen.lines().collect();
        assert!(
            rows[rows.len() - 2].contains("model · build · ask"),
            "{screen}"
        );
        let controls = rows.last().unwrap();
        assert_eq!(
            controls.trim(),
            if width == 44 {
                "enter choose · esc cancel"
            } else {
                "type to filter · ↑↓ choose · enter confirm · esc cancel"
            }
        );
        assert!(
            !screen.contains("? for shortcuts"),
            "the picker owns text input: {screen}"
        );
    }
}

#[tokio::test]
async fn hint_row_keeps_existing_approval_controls() {
    let mut app = App::new("model", "project");
    app.present_approval(prompt("shell", serde_json::json!({"command": "ls"})).await);
    assert_eq!(
        overlay_hint(&app).as_deref(),
        Some("y allow once · a allow this target · n deny")
    );
    for width in [44, 80, 100] {
        let screen = render(&app, width, 16, Theme::new().without_color());
        assert!(!screen.contains("? for shortcuts"), "{screen}");
        assert!(!screen.contains("bash mode"), "{screen}");
    }
}
