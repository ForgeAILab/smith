use super::*;

#[test]
fn line_editing_table_drives_all_five_fields() {
    use crate::{App, LineInput, Overlay};
    use crossterm::event::Event;
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    let ctrl = |ch| Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL));
    let alt = |ch| Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::ALT));
    let cases = vec![
        (
            "left",
            "alpha beta",
            vec![key(KeyCode::Left)],
            "alpha beta",
            9,
        ),
        (
            "right",
            "alpha beta",
            vec![key(KeyCode::Home), key(KeyCode::Right)],
            "alpha beta",
            1,
        ),
        (
            "home",
            "alpha beta",
            vec![key(KeyCode::Home)],
            "alpha beta",
            0,
        ),
        (
            "end",
            "alpha beta",
            vec![key(KeyCode::Home), key(KeyCode::End)],
            "alpha beta",
            10,
        ),
        ("ctrl+a", "alpha beta", vec![ctrl('a')], "alpha beta", 0),
        (
            "ctrl+e",
            "alpha beta",
            vec![ctrl('a'), ctrl('e')],
            "alpha beta",
            10,
        ),
        ("ctrl+u", "alpha beta", vec![ctrl('u')], "", 0),
        ("ctrl+k", "alpha beta", vec![ctrl('a'), ctrl('k')], "", 0),
        ("ctrl+w", "alpha beta", vec![ctrl('w')], "alpha ", 6),
        ("alt+b", "alpha beta", vec![alt('b')], "alpha beta", 6),
        (
            "alt+f",
            "alpha beta",
            vec![ctrl('a'), alt('f')],
            "alpha beta",
            5,
        ),
        (
            "backspace",
            "alpha beta",
            vec![key(KeyCode::Backspace)],
            "alpha bet",
            9,
        ),
        (
            "delete",
            "alpha beta",
            vec![ctrl('a'), key(KeyCode::Delete)],
            "lpha beta",
            0,
        ),
        (
            "middle paste",
            "alpha beta",
            vec![alt('b'), Event::Paste("Z".into())],
            "alpha Zbeta",
            7,
        ),
        (
            "insert",
            "alpha beta",
            vec![alt('b'), key(KeyCode::Char('Z'))],
            "alpha Zbeta",
            7,
        ),
        (
            "unicode",
            "a中é",
            vec![
                key(KeyCode::Left),
                key(KeyCode::Backspace),
                Event::Paste("界".into()),
            ],
            "a界é",
            2,
        ),
        (
            "empty boundaries",
            "",
            vec![
                key(KeyCode::Left),
                key(KeyCode::Delete),
                ctrl('w'),
                key(KeyCode::Backspace),
            ],
            "",
            0,
        ),
        (
            "whitespace words",
            "one  two  ",
            vec![ctrl('w')],
            "one  ",
            5,
        ),
        (
            "secret correction",
            "sk-tyxo",
            vec![
                key(KeyCode::Left),
                key(KeyCode::Backspace),
                key(KeyCode::Char('p')),
            ],
            "sk-typo",
            6,
        ),
    ];
    for (name, initial, edits, expected, cursor) in cases {
        let mut composer = App::new("model", "project");
        let mut picker = ResourcePicker::new("filter", Vec::new(), "empty");
        let mut plain = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        plain.step = Step::ProviderName;
        plain.picker = None;
        let mut masked = setup_app(
            SetupMode::Credential {
                provider: "zai".into(),
            },
            Vec::new(),
            Vec::new(),
        );
        masked.step = Step::CredentialValue;
        masked.credential_method = Some(CredentialMethod::Config);
        masked.picker = None;
        let mut history = App::new("model", "project");
        history.overlay = Some(Overlay::HistorySearch {
            original: String::new(),
            query: LineInput::default(),
            selected: None,
            matched: None,
        });
        for event in std::iter::once(Event::Paste(initial.into())).chain(edits) {
            match event {
                Event::Key(key) => {
                    composer.on_key(key);
                    picker.on_key(key);
                    plain.on_key(key);
                    masked.on_key(key);
                    history.on_key(key);
                }
                Event::Paste(text) => {
                    composer.on_paste(&text);
                    picker.paste(&text);
                    plain.on_paste(&text);
                    masked.on_paste(&text);
                    history.on_paste(&text);
                }
                _ => unreachable!(),
            }
            let Some(Overlay::HistorySearch { query, .. }) = &history.overlay else {
                panic!("search stays open");
            };
            let reference = (composer.composer.text(), composer.composer.cursor());
            for (field, input) in [
                ("picker", &picker.query),
                ("plain", &plain.input),
                ("masked", &masked.secret.0),
                ("history", query),
            ] {
                assert_eq!((input.text(), input.cursor()), reference, "{name}: {field}");
            }
            assert_eq!(
                masked.secret.0.display_text(),
                "•".repeat(reference.0.chars().count())
            );
            assert!(!format!("{:?}", masked.secret.0).contains(initial) || initial.is_empty());
        }
        assert_eq!(
            (composer.composer.text(), composer.composer.cursor()),
            (expected, cursor),
            "{name}"
        );
        if !expected.is_empty() {
            masked.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            assert_eq!(
                masked.secret.0.text(),
                expected,
                "submission retains corrected secret"
            );
            assert!(
                matches!(masked.submission(), Some(SetupSubmission::ChangeCredential { credential: SetupCredential::StoreInConfig(value), .. }) if value.expose() == expected)
            );
        }
    }
}

#[test]
fn field_cursors_follow_edits_and_remain_visible_in_narrow_viewports() {
    use ratatui::backend::Backend;
    for masked in [false, true] {
        let mut app = setup_app(
            SetupMode::Credential {
                provider: "zai".into(),
            },
            Vec::new(),
            Vec::new(),
        );
        app.step = Step::CredentialValue;
        app.picker = None;
        app.credential_method = Some(if masked {
            CredentialMethod::Config
        } else {
            CredentialMethod::Environment
        });
        app.on_paste("abcdefghij".repeat(8).as_str());
        app.on_key(key(KeyCode::Left));
        let mut terminal = Terminal::new(TestBackend::new(44, 10)).unwrap();
        terminal
            .draw(|frame| draw_setup(frame, &app, Theme::new()))
            .unwrap();
        let position = terminal.backend_mut().get_cursor_position().unwrap();
        assert_eq!(position.x, 43);
        assert!(position.y < 10);
        app.on_key(key(KeyCode::Home));
        app.on_key(key(KeyCode::Right));
        terminal
            .draw(|frame| draw_setup(frame, &app, Theme::new()))
            .unwrap();
        assert_eq!(terminal.backend_mut().get_cursor_position().unwrap().x, 3);
        if masked {
            let rendered = render_setup(&app, 44, 24);
            assert!(!rendered.contains("abc"));
            assert!(rendered.contains('•'));
        }
    }
    let mut picker = ResourcePicker::new(
        "A title that leaves little room for a filter",
        Vec::new(),
        "empty",
    );
    picker.paste(&"query".repeat(20));
    picker.on_key(key(KeyCode::Left));
    let mut terminal = Terminal::new(TestBackend::new(44, 10)).unwrap();
    terminal
        .draw(|frame| crate::draw_resource_picker(frame, frame.area(), &picker, Theme::new()))
        .unwrap();
    assert!(terminal.backend_mut().get_cursor_position().unwrap().x < 44);
    let mut app = crate::App::new("model", "project");
    app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    app.on_paste("query");
    app.on_key(key(KeyCode::Left));
    terminal
        .draw(|frame| crate::draw(frame, &app, Theme::new()))
        .unwrap();
    assert_eq!(terminal.backend_mut().get_cursor_position().unwrap().x, 22);
}

#[test]
fn step_keys_change_on_navigation_and_return_but_not_field_input() {
    let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    let welcome = app.step_key();
    app.on_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
    let authentication = app.step_key();
    assert_ne!(authentication, welcome);
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.step_key(), authentication);
    app.on_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE));
    let field = app.step_key();
    assert_ne!(field, authentication);
    app.on_paste("ZAI_API_KEY");
    app.on_event(ScreenEvent::Tick);
    app.on_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(app.step_key(), field);
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_ne!(app.step_key(), field);
    assert_ne!(app.step_key(), authentication, "re-entered authentication");
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_ne!(app.step_key(), welcome, "re-entered welcome");
}

#[test]
fn escape_from_key_field_restores_the_chosen_credential_method() {
    for method in ["keychain", "config", "environment"] {
        let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        choose(&mut app, "glm");
        choose(&mut app, method);
        assert_eq!(app.step, Step::CredentialValue);
        assert!(matches!(app.on_key(key(KeyCode::Esc)), SetupEffect::None));
        assert_eq!(app.step, Step::CredentialMethod);
        assert_eq!(
            app.picker
                .as_ref()
                .and_then(ResourcePicker::selected_entry)
                .map(|entry| entry.id.as_str()),
            Some(method)
        );
        assert!(matches!(app.on_key(key(KeyCode::Esc)), SetupEffect::None));
        assert_eq!(app.step, Step::Action);
        assert!(matches!(app.on_key(key(KeyCode::Esc)), SetupEffect::Cancel));
    }
}

#[test]
fn environment_variable_name_survives_back_and_forward() {
    for back in [KeyCode::Esc, KeyCode::BackTab] {
        for submitted in [false, true] {
            let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
            app.on_key(key(KeyCode::Char('1')));
            app.on_key(key(KeyCode::Char('4')));
            for character in "ZAI_API_KEY".chars() {
                app.on_key(key(KeyCode::Char(character)));
            }
            if submitted {
                app.on_key(key(KeyCode::Enter));
                assert_eq!(app.step, Step::Review);
                app.on_key(key(back));
                assert_eq!(app.input, "ZAI_API_KEY");
                app.on_paste("_CORRECTED");
            }
            let expected = app.input.text().to_owned();
            app.on_key(key(back));
            assert_eq!(app.step, Step::CredentialMethod);
            app.on_key(key(back));
            assert_eq!(app.step, Step::Action);
            app.on_key(key(KeyCode::Char('1')));
            app.on_key(key(KeyCode::Char('4')));
            assert_eq!(app.step, Step::CredentialValue);
            assert_eq!(app.input.text(), expected);
            assert!(render_setup(&app, 100, 32).contains(&expected));
            app.on_key(key(KeyCode::Enter));
            assert_eq!(app.step, Step::Review);
            let SetupEffect::Submit {
                submission: SetupSubmission::QuickGlm { credential },
                ..
            } = app.on_key(key(KeyCode::Enter))
            else {
                panic!("restored environment reference reaches submission");
            };
            assert!(matches!(credential, SetupCredential::Environment(name) if name == expected));
        }
    }
}

#[test]
fn returning_to_a_secret_field_never_restores_the_key_or_variable_name() {
    for method in ["keychain", "config"] {
        let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        choose(&mut app, "glm");
        choose(&mut app, "environment");
        app.on_paste("ZAI_API_KEY");
        app.on_key(key(KeyCode::Esc));
        choose(&mut app, method);
        assert!(app.input.is_empty());
        app.on_paste("sk-never-restored");
        app.on_key(key(KeyCode::Enter));
        app.on_key(key(KeyCode::Esc));
        assert!(app.secret.is_empty());
        app.on_key(key(KeyCode::Esc));
        choose(&mut app, method);
        assert!(app.input.is_empty());
        assert!(app.secret.is_empty());
        assert!(!render_setup(&app, 100, 32).contains("sk-never-restored"));
    }
}

#[test]
fn ctrl_c_cancels_review_and_busy_steps_without_submitting() {
    let mut review = glm_environment_review();
    assert!(matches!(
        review.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        SetupEffect::Cancel
    ));
    review.step = Step::Busy;
    assert!(matches!(
        review.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        SetupEffect::Cancel
    ));
}

#[test]
fn numbered_setup_choices_ignore_letters_and_select_with_digits() {
    let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    app.on_paste("google");
    app.on_key(key(KeyCode::Char('g')));
    assert!(app.picker.as_ref().expect("action picker").query.is_empty());
    app.on_key(key(KeyCode::Char('1')));
    assert_eq!(app.step, Step::CredentialMethod);
    app.on_key(key(KeyCode::Char('3')));
    assert_eq!(app.step, Step::CredentialValue);
    assert_eq!(app.credential_method, Some(CredentialMethod::Config));
}

#[test]
fn bound_cuts_on_a_character_boundary_instead_of_panicking() {
    // 重 is three bytes, so a 1_024-byte budget lands inside the 342nd
    // character — exactly where String::truncate would panic. Chinese
    // error text and collision previews reach this path for real.
    let long = "重".repeat(400);
    let bounded = bound(long, 1_024);
    assert!(bounded.ends_with('…'), "{bounded}");
    assert_eq!(bounded.trim_end_matches('…').chars().count(), 341);
}

#[test]
fn back_restores_custom_non_secret_fields_and_clears_secret_input() {
    let provider = "router";
    let endpoint = "https://example.test/v1";
    let model = "custom-model";
    let secret = "sk-never-restored";
    let mut app = setup_app(SetupMode::AddProvider, Vec::new(), Vec::new());

    for character in provider.chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    for character in endpoint.chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    choose(&mut app, "keychain");
    for character in secret.chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    for character in model.chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    assert!(matches!(
        app.on_key(key(KeyCode::Enter)),
        SetupEffect::ResolveModelLimits { .. }
    ));
    app.apply_resolved_limits(None);
    for character in "64000".chars() {
        app.on_key(key(KeyCode::Char(character)));
    }
    app.on_key(key(KeyCode::Enter));
    choose(&mut app, "normal");
    choose(&mut app, "yes");
    assert_eq!(app.step, Step::Review);

    app.review_collisions("[providers.router]\n- endpoint = \"old\"\n+ endpoint = \"new\"");
    assert!(app.allow_collisions);

    // Back immediately revokes the collision approval, then the actual
    // setup history exposes the retained non-secret fields in order.
    app.on_key(key(KeyCode::BackTab));
    assert_eq!(app.step, Step::DefaultChoice);
    assert!(app.collision_preview.is_none());
    assert!(!app.allow_collisions);
    app.on_key(key(KeyCode::BackTab));
    assert_eq!(app.step, Step::ResponseBehavior);
    app.on_key(key(KeyCode::BackTab));
    assert_eq!(app.step, Step::ContextTokens);
    assert_eq!(app.input, "64000");
    app.on_key(key(KeyCode::BackTab));
    assert_eq!(app.step, Step::ModelName);
    assert_eq!(app.input, model);
    app.on_key(key(KeyCode::BackTab));
    assert_eq!(app.step, Step::CredentialValue);
    assert!(app.input.is_empty());
    assert!(app.secret.is_empty());
    app.on_key(key(KeyCode::BackTab));
    assert_eq!(app.step, Step::CredentialMethod);
    assert!(app.secret.is_empty());
    app.on_key(key(KeyCode::BackTab));
    assert_eq!(app.step, Step::Endpoint);
    assert_eq!(app.input, endpoint);
    app.on_key(key(KeyCode::BackTab));
    assert_eq!(app.step, Step::ProviderName);
    assert_eq!(app.input, provider);
}
