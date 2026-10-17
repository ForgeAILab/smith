//! The default-model choice is the `/model` picker, embedded or standalone.

use super::*;

/// Rows in the `/model` grammar: label, `provider · N context`, then facts.
fn model_rows() -> Vec<ResourceEntry> {
    [
        ("acme/alpha", "alpha", "acme", "200k", false),
        ("acme/bravo", "bravo", "acme", "64k", false),
        (
            "remote/example-model",
            "example-model",
            "remote",
            "128k",
            true,
        ),
        (
            "remote/second-model",
            "second-model",
            "remote",
            "32k",
            false,
        ),
        ("zeta/one", "one", "zeta", "1M", false),
        ("zeta/two", "two", "zeta", "1M", false),
        ("zeta/three", "three", "zeta", "272k", false),
    ]
    .into_iter()
    .map(|(id, label, provider, context, active)| {
        ResourceEntry::new(
            id,
            label,
            format!("{context} context · 4k output ceiling · {id} · user config"),
        )
        .description(format!("{provider} · {context} context"))
        .active(active)
    })
    .collect()
}

fn change_default_entry() -> SetupEntry {
    SetupEntry {
        id: "change-default".into(),
        label: "Change default".into(),
        detail: "choose a configured provider/model pair".into(),
        flow: SetupFlow::ChangeDefault,
    }
}

fn default_model_choice() -> SetupApp {
    let mut app = setup_app(SetupMode::Menu, Vec::new(), model_rows())
        .with_provider_actions(vec![change_default_entry()]);
    choose(&mut app, "change-default");
    assert_eq!(app.step, Step::ModelChoice);
    app
}

fn rows_of(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    setup_screen(buffer).lines().map(str::to_owned).collect()
}

#[test]
fn default_model_choice_opens_on_the_current_model_and_keeps_a_remembered_pick() {
    let mut app = default_model_choice();
    assert_eq!(
        app.picker
            .as_ref()
            .and_then(ResourcePicker::selected_entry)
            .map(|entry| entry.id.as_str()),
        Some("remote/example-model")
    );
    // Enter keeps the current model, exactly as `/model` would.
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.step, Step::Review);
    assert_eq!(
        (app.provider.as_str(), app.model.as_str()),
        ("remote", "second-model")
    );

    // Back returns to the choice this flow made, not to the current model.
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.step, Step::ModelChoice);
    assert_eq!(
        app.picker
            .as_ref()
            .and_then(ResourcePicker::selected_entry)
            .map(|entry| entry.id.as_str()),
        Some("remote/second-model")
    );
}

#[test]
fn standalone_default_model_choice_is_the_model_picker_at_the_bottom() {
    let app = default_model_choice();
    for (width, height) in [(44, 16), (80, 24), (100, 32)] {
        let buffer = render_setup_buffer(
            &app,
            width,
            height,
            Theme::new().without_color().without_motion(),
        );
        let rows = rows_of(&buffer);
        let text = rows.join("\n");
        // The single controls row; at 44 columns Enter and Escape lead, as
        // they do beneath `/model`.
        let last = rows.last().expect("controls row");
        for control in ["↑↓ choose", "enter confirm", "esc back"] {
            assert!(last.contains(control), "{text}");
        }
        assert!(rows[rows.len() - 2].trim().is_empty(), "{text}");
        let title = rows
            .iter()
            .position(|row| row.starts_with("  Smith setup · Choose default model"))
            .expect("title row");
        // Anchored at the bottom: nothing above the pane, and five choices
        // plus the selected row's detail between the title and the controls.
        assert!(
            rows[..title].iter().all(|row| row.trim().is_empty()),
            "{text}"
        );
        assert_eq!(rows.len() - title, 1 + 5 + 1 + 1 + 1, "{text}");
        assert!(rows[title].trim_end().ends_with("3/7"), "{text}");
        let selected = rows
            .iter()
            .position(|row| row.starts_with("❯ example-model"))
            .expect("current model is selected");
        // The description column elides at 44 columns, as it does in `/model`.
        assert!(rows[selected].contains("remote ·"), "{text}");
        assert!(rows[selected].trim_end().ends_with("✓ current"), "{text}");
        assert!(
            rows[selected + 1].starts_with("  4k output ceiling"),
            "{text}"
        );
        for frame_glyph in ['┌', '┐', '└', '┘', '│'] {
            assert!(!text.contains(frame_glyph), "{text}");
        }
    }
}

#[test]
fn embedded_default_model_choice_matches_the_session_model_picker() {
    let theme = Theme::new().without_color().without_motion();
    for (width, height) in [(44, 16), (100, 32)] {
        let mut session = crate::App::new("example-model", "~/work/api");
        session.set_resources(crate::RuntimeResources {
            models: model_rows(),
            ..crate::RuntimeResources::default()
        });
        let backdrop = crate::App::new("example-model", "~/work/api");
        for character in "/model".chars() {
            session.on_key(key(KeyCode::Char(character)));
        }
        session.on_key(key(KeyCode::Enter));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| crate::render::draw(frame, &session, theme))
            .expect("draw");
        let model = rows_of(terminal.backend().buffer());
        let setup = default_model_choice();
        terminal
            .draw(|frame| {
                crate::render::draw_with_screen(frame, &backdrop, &setup, theme);
            })
            .expect("draw");
        let embedded = rows_of(terminal.backend().buffer());
        let context = format!("{}\n---\n{}", model.join("\n"), embedded.join("\n"));

        let pane = |rows: &[String], title: &str| {
            let start = rows
                .iter()
                .position(|row| row.starts_with(title))
                .expect("picker title");
            let end = start
                + rows[start..]
                    .iter()
                    .position(|row| row.starts_with('─'))
                    .expect("composer rule");
            (start, rows[start + 1..end].to_vec())
        };
        let (model_title, model_rows) = pane(&model, "  Choose model");
        let (setup_title, setup_rows) = pane(&embedded, "  Smith setup · Choose default model");
        // Same choices, the same row text, and the same selected detail.
        assert_eq!(model_rows, setup_rows, "{context}");
        assert_eq!(
            model[model_title].split_whitespace().last(),
            embedded[setup_title].split_whitespace().last(),
            "{context}"
        );
        // Identity above the controls, as `/model` lays them out; Escape
        // returns to the earlier setup step instead of cancelling.
        let tail = |rows: &[String]| rows[rows.len() - 2..].to_vec();
        assert_eq!(tail(&model)[0], tail(&embedded)[0], "{context}");
        assert_eq!(
            tail(&model)[1].replace("esc cancel", "esc back").trim_end(),
            tail(&embedded)[1].trim_end(),
            "{context}"
        );
        assert!(
            !embedded.join("\n").contains("? for shortcuts"),
            "{context}"
        );
    }
}

#[test]
fn default_model_choice_error_stays_visible_in_the_compact_pane() {
    let mut app = setup_app(
        direct_provider_mode("google"),
        Vec::new(),
        vec![ResourceEntry::new(
            "gemini-unpriced",
            "Gemini Unpriced",
            "catalog limits",
        )],
    );
    choose(&mut app, "environment");
    for character in "GEMINI_API_KEY".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.step, Step::ModelChoice);
    // No limits were supplied for this catalog row, so confirming it fails.
    app.on_key(key(KeyCode::Enter));
    assert_eq!(app.step, Step::ModelChoice);
    let rendered = render_setup(&app, 80, 24);
    assert!(
        rendered.contains("error: the selected catalog model has no enforceable limits"),
        "{rendered}"
    );
    assert!(rendered.contains("❯ Gemini Unpriced"), "{rendered}");
    assert_eq!(
        app.content_height(80),
        crate::picker::compact_picker_rows(
            app.picker.as_ref().expect("picker"),
            80,
            app.error.as_deref()
        )
    );
}
