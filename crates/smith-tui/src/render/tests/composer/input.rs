use super::*;

#[test]
fn composer_rules_prompt_and_placeholder_survive_without_color_at_all_widths() {
    for width in [44, 80, 100] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            for draft in ["", "keep this draft", "first\nsecond\nlast"] {
                let mut app = App::new("model", "project");
                app.composer.replace(draft);
                let mut terminal =
                    Terminal::new(TestBackend::new(width, 16)).expect("a test terminal");
                terminal
                    .draw(|frame| draw(frame, &app, theme))
                    .expect("a frame");
                let buffer = terminal.backend().buffer();
                let screen = screen_text(buffer);
                let rows: Vec<_> = screen.lines().collect();
                let top = rows.iter().position(|row| row.starts_with('─')).unwrap();
                let bottom = rows.iter().rposition(|row| row.starts_with('─')).unwrap();
                let rule = "─".repeat(usize::from(width));
                assert_eq!(rows[top], rule);
                assert_eq!(rows[bottom], rule);
                assert_eq!(rows.iter().filter(|row| **row == rule).count(), 2);
                assert_eq!(bottom - top, draft.split('\n').count() + 1);
                assert!(rows[top + 1].starts_with("> "), "{screen}");
                if draft.is_empty() {
                    assert_eq!(rows[top + 1], "> Ask Smith to do anything");
                } else if draft.contains('\n') {
                    assert_eq!(&rows[top + 1..bottom], ["> first", "  second", "  last"]);
                }
                assert!(rows[bottom + 1].contains("model"), "{screen}");
                for y in [top, bottom] {
                    for x in 0..width {
                        let cell = &buffer[(x, u16::try_from(y).unwrap())];
                        assert!(cell.modifier.contains(Modifier::DIM));
                        assert_eq!(cell.fg, Color::Reset);
                        assert_eq!(cell.bg, Color::Reset);
                    }
                }
                if !theme.uses_color() {
                    assert!(
                        buffer
                            .content
                            .iter()
                            .all(|cell| { cell.fg == Color::Reset && cell.bg == Color::Reset })
                    );
                }
            }
        }
    }
}

#[test]
fn leading_bang_changes_only_the_composer_prompt_and_adds_the_bash_hint() {
    for width in [44, 80, 100] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            let mut app = App::new("model", "project");
            for draft in ["!", "!ls", "!!literal"] {
                app.composer.replace(draft);
                let screen = render(&app, width, 16, theme);
                assert!(
                    screen
                        .lines()
                        .any(|line| line == format!("! {}", &draft[1..]).trim_end()),
                    "{screen}"
                );
                assert!(
                    screen.lines().last().unwrap().starts_with("  bash mode"),
                    "{screen}"
                );
                assert_eq!(app.composer.text(), draft);
            }
            for draft in ["", "ask !ls", " !ls"] {
                app.composer.replace(draft);
                let screen = render(&app, width, 16, theme);
                assert!(!screen.contains("bash mode"), "{screen}");
                assert!(
                    screen.lines().any(|line| line.starts_with("> ")),
                    "{screen}"
                );
            }
            app.composer.replace("!ls");
            assert_eq!(
                app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                Some(crate::app::Action::RunShell {
                    command: "ls".to_owned()
                })
            );
            app.composer.replace("!!literal");
            match app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
                Some(crate::app::Action::Submit { submission, .. }) => {
                    assert_eq!(submission.display_text(), "!literal");
                }
                other => panic!("expected a literal bang submission, got {other:?}"),
            }
        }
    }
}

#[test]
fn cursor_stays_inside_the_rules_after_line_and_draft_navigation() {
    use ratatui::backend::Backend as _;
    for width in [44, 80, 100] {
        let mut app = App::new("model", "project");
        app.composer.replace("écho\n中\nlast line");
        for event in [
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        ] {
            app.on_key(event);
            let mut terminal = Terminal::new(TestBackend::new(width, 16)).expect("a test terminal");
            terminal
                .draw(|frame| draw(frame, &app, Theme::new().without_color()))
                .expect("a frame");
            let screen = screen_text(terminal.backend().buffer());
            let top = screen
                .lines()
                .position(|line| line.starts_with('─'))
                .unwrap();
            let position = terminal
                .backend_mut()
                .get_cursor_position()
                .expect("a cursor");
            let (line, column) = app.composer.cursor_position();
            assert_eq!(
                (usize::from(position.x), usize::from(position.y)),
                (column + 2, top + 1 + line),
                "{screen}"
            );
        }
    }
}

#[test]
fn a_conversation_renders_its_transcript_composer_and_footer() {
    let app = conversation();
    let screen = render(&app, 74, 16, Theme::new());
    insta_like(
        &screen,
        &[
            "gpt-5.3",
            "> explain the retry policy",
            "● The retry policy classifies failures.",
            "● Read(src/retry.rs)",
            "Ask Smith to do anything",
        ],
    );
}

#[test]
fn registered_paste_and_image_labels_keep_their_compact_accented_surface() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.on_paste("one\ntwo\nthree");
    app.attach_image("data:image/png;base64,IMAGE".into(), 32, 32);

    let mut terminal = Terminal::new(TestBackend::new(80, 14)).expect("a test terminal");
    terminal
        .draw(|frame| draw(frame, &app, Theme::new()))
        .expect("a frame");
    let buffer = terminal.backend().buffer();
    let row = (0..buffer.area.height)
        .find(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, *y)].symbol())
                .collect::<String>()
                .contains("[Pasted text #1 +3 lines][Image #1 32×32]")
        })
        .expect("composer attachment row");
    let rendered = (0..buffer.area.width)
        .map(|x| buffer[(x, row)].symbol())
        .collect::<String>();
    let paste_x =
        u16::try_from(rendered.find("[Pasted").expect("paste label")).expect("paste position fits");
    let image_x =
        u16::try_from(rendered.find("[Image").expect("image label")).expect("image position fits");
    assert_eq!(buffer[(paste_x, row)].fg, Color::Cyan);
    assert_eq!(buffer[(image_x, row)].fg, Color::Cyan);
}

/// A Chinese draft reaches the right edge at half the character count, so
/// the composer wraps far sooner than an English one. The cursor has to
/// follow it onto the wrapped row instead of walking off the surface.
#[test]
fn the_cursor_follows_a_wrapped_chinese_draft_onto_the_next_row() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    let draft = "请解释一下重试策略的实现方式和它的退避曲线";
    app.composer.replace(draft);

    use ratatui::backend::Backend as _;
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).expect("a test terminal");
    terminal
        .draw(|frame| draw(frame, &app, Theme::new().without_color()))
        .expect("a frame");
    let position = terminal
        .backend_mut()
        .get_cursor_position()
        .expect("a cursor position");
    let buffer = terminal.backend().buffer().clone();

    let rows: Vec<String> = (0..buffer.area.height)
        .map(|y| {
            crate::selection::glyph_bounds(&buffer, buffer.area, y)
                .map(|(x, _)| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect();
    let composer_row = rows
        .iter()
        .rposition(|row| row.starts_with('>'))
        .expect("the composer row");
    // `> ` plus nineteen characters fills the forty columns, so the
    // draft's last two characters wrap onto the row below it.
    assert_eq!(
        rows[composer_row],
        "> 请解释一下重试策略的实现方式和它的退避"
    );
    assert_eq!(rows[composer_row + 1], "曲线");
    assert_eq!(
        (position.x, usize::from(position.y)),
        (4, composer_row + 1),
        "the cursor left the wrapped text:\n{}",
        rows.join("\n")
    );
}
