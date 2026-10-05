use super::*;

#[test]
fn setup_steps_start_at_the_left_gutter_without_a_frame() {
    let action = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    let mut authentication = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    choose(&mut authentication, "glm");
    let field = setup_app(SetupMode::AddProvider, Vec::new(), Vec::new());
    let review = glm_environment_review();
    let mut busy = glm_environment_review();
    busy.on_key(key(KeyCode::Enter));
    for app in [action, authentication, field, review, busy] {
        for (width, height) in [(44, 16), (80, 24), (100, 32)] {
            let rendered = render_setup(&app, width, height);
            assert_eq!(rendered.matches("Smith setup").count(), 1, "{rendered}");
            assert!(
                rendered
                    .lines()
                    .next()
                    .expect("title row")
                    .starts_with("  Smith setup"),
                "{rendered}"
            );
            for frame_glyph in ['┌', '┐', '└', '┘', '│'] {
                assert!(!rendered.contains(frame_glyph), "{rendered}");
            }
        }
    }
}

#[test]
fn setup_descriptions_wrap_whole_words_beneath_the_name() {
    let description = "Review endpoint credentials and 中文 model limits before applying this connection. Every word remains available beneath the selected name.";
    let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    let mut entry = app.provider_actions[0].clone();
    entry.detail = description.into();
    app = app.with_provider_actions(vec![entry.clone()]);
    for width in [44, 80] {
        let buffer = render_setup_buffer(&app, width, 32, Theme::new().without_color());
        let rendered = setup_screen(&buffer);
        let rows = setup_body_rows(&buffer);
        let name = rows
            .iter()
            .position(|row| row.trim_end() == format!("❯ 1. {}", entry.label))
            .expect("the name occupies its own line");
        let description_rows = rows[name + 1..]
            .iter()
            .take_while(|row| row.starts_with("     "))
            .filter(|row| !row.trim().is_empty())
            .collect::<Vec<_>>();
        assert!(description_rows.len() > 1, "{rendered}");
        assert_eq!(
            description_rows
                .iter()
                .map(|row| row.trim())
                .collect::<Vec<_>>()
                .join(" "),
            description,
            "description was split or truncated:\n{rendered}"
        );
        for row in &description_rows {
            assert_eq!(
                row.chars()
                    .take_while(|character| *character == ' ')
                    .count(),
                5,
                "descriptions start beneath the numbered label:\n{rendered}"
            );
            for word in row.split_whitespace() {
                assert!(
                    description.split_whitespace().any(|whole| whole == word),
                    "{rendered}"
                );
            }
        }
        let inner = buffer.area;
        for row in 0..description_rows.len() {
            let y = inner.y + u16::try_from(name + 1 + row).expect("description row");
            assert!(
                buffer[(inner.x + 5, y)]
                    .modifier
                    .contains(ratatui::style::Modifier::DIM)
            );
        }
    }
}

#[test]
fn overflowing_setup_entries_scroll_without_displacing_the_footer() {
    let description = "Keep the endpoint credentials and model available for local review before applying the change.";
    for (width, height, footer_rows) in [(44, 16, 2), (80, 24, 1)] {
        let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        let prototype = app.provider_actions[0].clone();
        let entries = (1..=20)
            .map(|index| SetupEntry {
                id: format!("choice-{index:02}"),
                label: format!("Choice {index:02}"),
                detail: description.into(),
                flow: prototype.flow.clone(),
            })
            .collect::<Vec<_>>();
        app = app.with_provider_actions(entries);
        for selected in 1..=20 {
            let buffer = render_setup_buffer(&app, width, height, Theme::new().without_color());
            let rendered = setup_screen(&buffer);
            let rows = setup_body_rows(&buffer);
            let footer = rows[rows.len() - footer_rows..].join("\n");
            for control in ["↑↓ or 1–9 choose", "enter confirm", "esc cancel"] {
                assert!(footer.contains(control), "{width}×{height}: {rendered}");
            }
            assert_eq!(rendered.matches('❯').count(), 1, "{rendered}");
            let name = rows
                .iter()
                .position(|row| row.trim_end() == format!("❯ {selected}. Choice {selected:02}"))
                .expect("the selected entry remains visible");
            let description_rows = rows[name + 1..]
                .iter()
                .take_while(|row| row.starts_with("     "))
                .filter(|row| !row.trim().is_empty())
                .collect::<Vec<_>>();
            assert_eq!(
                description_rows
                    .iter()
                    .map(|row| row.trim())
                    .collect::<Vec<_>>()
                    .join(" "),
                description,
                "{width}×{height} omitted part of the selected description:\n{rendered}"
            );
            let visible = rows
                .iter()
                .filter_map(|row| {
                    row.trim_start_matches("❯ ")
                        .trim()
                        .split_once(". ")
                        .and_then(|(_, label)| label.strip_prefix("Choice "))
                })
                .map(|number| number.parse::<usize>().expect("entry number"))
                .collect::<Vec<_>>();
            assert!(visible.len() < 20, "{rendered}");
            assert!(
                visible.windows(2).all(|pair| pair[0] < pair[1]),
                "{rendered}"
            );
            app.on_key(key(KeyCode::Down));
        }
        let wrapped = render_setup(&app, width, height);
        assert!(wrapped.contains("❯ 1. Choice 01"), "{wrapped}");
        app.on_key(key(KeyCode::Up));
        let last = render_setup(&app, width, height);
        assert!(last.contains("❯ 20. Choice 20"), "{last}");
    }
}

#[test]
fn setup_no_color_preserves_selection_description_and_controls() {
    use ratatui::style::{Color, Modifier};

    let app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    for (width, height) in [(44, 16), (80, 24)] {
        let colored = render_setup_buffer(&app, width, height, Theme::new());
        let plain = render_setup_buffer(&app, width, height, Theme::new().without_color());
        assert_eq!(setup_screen(&colored), setup_screen(&plain));
        assert!(
            plain
                .content
                .iter()
                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
        );
        let inner = plain.area;
        let rows = setup_body_rows(&plain);
        let name = rows
            .iter()
            .position(|row| row.starts_with("❯ "))
            .expect("selection marker");
        let y = inner.y + u16::try_from(name).expect("name row");
        assert!(plain[(inner.x, y)].modifier.contains(Modifier::BOLD));
        assert!(rows[name + 1].starts_with("    "));
        assert!(plain[(inner.x + 5, y + 1)].modifier.contains(Modifier::DIM));
        assert!(rows.iter().any(|row| row.contains("esc cancel")));
    }
}

#[test]
fn setup_filtering_and_empty_guidance_keep_the_footer() {
    let mut app = setup_app(
        SetupMode::AddModel { provider: None },
        vec![
            ResourceEntry::new("google", "Google Gemini", "native endpoint"),
            ResourceEntry::new("zai", "Z.AI", "configured"),
        ],
        Vec::new(),
    );
    app.on_paste("google");
    let filtered = render_setup(&app, 44, 16);
    assert!(filtered.contains("❯ Google Gemini"), "{filtered}");
    assert!(!filtered.contains("Z.AI"), "{filtered}");
    app.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.on_paste("no-such-entry");
    let missing = render_setup(&app, 44, 16);
    for text in [
        "No matches",
        "Ctrl+U clear filter",
        "ctrl+u clear filter",
        "esc cancel",
    ] {
        assert!(missing.contains(text), "{missing}");
    }
    assert!(!missing.contains("enter confirm"), "{missing}");
    assert!(!missing.contains("↑↓ choose"), "{missing}");
    let empty = render_setup(
        &setup_app(SetupMode::FirstRun, Vec::new(), Vec::new()).with_provider_actions(Vec::new()),
        44,
        16,
    );
    assert!(empty.contains("No setup actions are available."), "{empty}");
    assert!(empty.contains("esc cancel"), "{empty}");
}

#[test]
fn setup_picker_states_and_selected_detail_remain_visible() {
    let entries = vec![
        ResourceEntry::new("active", "Active provider", "Native endpoint").active(true),
        ResourceEntry::new(
            "unavailable",
            "Unavailable provider",
            "Full provider metadata",
        )
        .description("Custom endpoint")
        .active(true)
        .disabled("Adapter unavailable in this build"),
    ];
    let mut app = setup_app(SetupMode::AddModel { provider: None }, entries, Vec::new());
    app.on_key(key(KeyCode::Down));
    let rendered = render_setup(&app, 80, 24);
    for text in [
        "Active provider",
        "❯ Unavailable provider",
        "✓ current",
        "unavailable",
        "Custom endpoint",
        "Full provider metadata",
        "Adapter unavailable in this build",
    ] {
        assert!(rendered.contains(text), "{rendered}");
    }
    assert!(matches!(app.on_key(key(KeyCode::Enter)), SetupEffect::None));
    assert_eq!(app.step, Step::ProviderChoice);
}

#[test]
fn embedded_review_scrolls_inside_the_session_pane() {
    for (width, height) in [(44, 16), (100, 32)] {
        let mut review = glm_environment_review().with_title("Connect OpenRouter");
        review.review_collisions(format!("{}\nreview-final", "reviewed change\n".repeat(60)));
        let mut backdrop = crate::App::new("gpt-5.3", "~/work/api");
        backdrop.transcript.push_user("retained transcript");
        backdrop.composer.insert_str("retained draft");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                .expect("terminal");
        let mut region = Rect::default();
        let mut saw_review_end = false;
        for _ in 0..80 {
            terminal
                .draw(|frame| {
                    region = crate::render::draw_with_screen(
                        frame,
                        &backdrop,
                        &review,
                        Theme::new().without_color(),
                    );
                })
                .expect("draw");
            let buffer = terminal.backend().buffer();
            for y in region.y..region.bottom() {
                let row = (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>();
                saw_review_end |= row.contains("review-final");
            }
            review.on_key(key(KeyCode::PageDown));
        }
        let buffer = terminal.backend().buffer();
        let rows = (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let text = rows.join("\n");
        for expected in [
            "retained transcript",
            "retained draft",
            "Connect OpenRouter",
            "enter confirm",
            "esc back",
        ] {
            assert!(text.contains(expected), "{text}");
        }
        // The merged review now follows its collision preview with Writes and
        // the warning. Its last preview row must be reachable within the pane.
        assert!(saw_review_end, "{text}");
        let scroll = review.review_scroll.get();
        assert_eq!(scroll.offset, scroll.limit);
        assert!(!text.contains("Smith setup"), "{text}");
        assert!(!text.contains("? for shortcuts"), "{text}");
        assert!(
            rows.last()
                .expect("hint row")
                .starts_with("  enter confirm · esc back"),
            "{text}"
        );
        assert_eq!(backdrop.composer.text(), "retained draft");
    }
}

#[test]
fn setup_welcome_and_intro_precede_the_list_at_every_width() {
    let app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    for width in [20, 44, 100] {
        let rendered = render_setup(&app, width, 24);
        let rows = rendered.lines().collect::<Vec<_>>();
        assert!(rows[0].starts_with("  Smith setup"), "{rendered}");
        assert!(rows[1].starts_with("  Nothing"), "{rendered}");
        let choice = rows
            .iter()
            .position(|row| row.starts_with("❯ "))
            .expect("first choice");
        let intro = rows[1..choice]
            .iter()
            .map(|row| row.trim())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(intro, "Nothing is sent to a provider until setup finishes.");
        assert!(!rendered.contains("Shift+Tab"), "{rendered}");
        assert!(!rendered.contains("no agent session"), "{rendered}");
        let plain = render_setup_buffer(&app, width, 24, Theme::new().without_color());
        assert!(
            plain[(2, 1)]
                .modifier
                .contains(ratatui::style::Modifier::DIM)
        );
    }
    assert!(
        render_setup(&app, 100, 24)
            .contains("Smith setup · Welcome · choose how to connect a model")
    );
}

#[test]
fn credential_methods_name_the_provider_and_field_help_starts_at_the_gutter() {
    let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
    choose(&mut app, "glm");
    choose(&mut app, "existing-keychain");
    app.on_key(key(KeyCode::Esc));
    let rendered = render_setup(&app, 100, 24);
    assert!(rendered.contains("keychain:smith/zai"), "{rendered}");
    assert!(!rendered.contains("<provider>"), "{rendered}");
    choose(&mut app, "keychain");
    let rendered = render_setup(&app, 44, 16);
    let rows = rendered.lines().collect::<Vec<_>>();
    assert!(rows[2].starts_with("  Stored"), "{rendered}");
    assert!(rows[3].starts_with("  "), "{rendered}");
    assert!(
        !rows[3].trim().is_empty(),
        "help wraps at the gutter: {rendered}"
    );
}

#[test]
fn review_labels_align_and_keep_compact_limits_and_all_reviewed_facts() {
    let mut app = glm_environment_review();
    let home = std::env::var_os("HOME").expect("test HOME");
    app = app.with_destination(
        Path::new(&home)
            .join(".smith/config.toml")
            .to_string_lossy(),
    );
    let rows = app.review_lines();
    let labels = [
        "Provider",
        "API key",
        "Model",
        "Requests",
        "GLM replies",
        "Default",
        "Writes",
    ];
    assert_eq!(rows.len(), labels.len());
    for (row, label) in rows.iter().zip(labels) {
        assert_eq!(&row[..13], format!("{label:11}  "));
    }
    assert_eq!(
        rows[0],
        "Provider     zai · https://api.z.ai/api/coding/paas/v4"
    );
    assert_eq!(rows[1], "API key      env:ZAI_API_KEY");
    assert_eq!(
        rows[2],
        "Model        glm-5.2 · 1M context · 1M input · 131k output · trusted catalog v5"
    );
    assert_eq!(rows[3], "Requests     32.7k output · 32.7k reserved");
    assert!(
        rows[4]
            .ends_with("an answer sent only as reasoning is shown as the reply; thinking stays on")
    );
    assert_eq!(rows[5], "Default      profile glm");
    assert_eq!(
        rows[6],
        "Writes       ~/.smith/config.toml, then checks the configuration"
    );
    let text = rows.join("\n");
    for old in [
        "1000000",
        "131072",
        "32768",
        "pending action",
        "local preflight",
    ] {
        assert!(!text.contains(old), "{text}");
    }
    assert_eq!(
        review_destination(
            "/tmp/injected-home/.smith/config.toml",
            Some(Path::new("/tmp/injected-home"))
        ),
        "~/.smith/config.toml"
    );
    assert_eq!(
        review_destination(
            "/tmp/injected-home-other/config.toml",
            Some(Path::new("/tmp/injected-home"))
        ),
        "/tmp/injected-home-other/config.toml"
    );
    assert_eq!(
        review_destination("~/.smith/config.toml", None),
        "~/.smith/config.toml"
    );
    let rendered = render_setup(&app, 100, 32);
    assert!(
        rendered.contains("Review · nothing is written until you confirm"),
        "{rendered}"
    );
    assert!(!rendered.contains("Review your choices"), "{rendered}");
}

#[test]
fn setup_review_values_hang_indent_and_wrap_words_at_44_and_80_columns() {
    let destination = format!("/tmp/{}config.toml", "reviewed-directory/".repeat(12));
    let app = glm_environment_review().with_destination(destination.clone());
    let labels = [
        "Provider",
        "API key",
        "Model",
        "Requests",
        "GLM replies",
        "Default",
        "Writes",
    ];
    let value_column = 15;
    for width in [44, 80] {
        let rendered = render_setup(&app, width, 64);
        let rows = rendered.lines().collect::<Vec<_>>();
        assert!(!rendered.contains("Review your choices"), "{rendered}");
        assert!(rows[2].starts_with("  Provider"), "{rendered}");
        let starts = labels
            .iter()
            .map(|label| {
                rows.iter()
                    .position(|row| row.starts_with(&format!("  {label:11}  ")))
                    .expect("labelled review row")
            })
            .collect::<Vec<_>>();
        for (index, start) in starts.iter().copied().enumerate() {
            let end = starts.get(index + 1).copied().unwrap_or(rows.len());
            for row in rows[start + 1..end]
                .iter()
                .take_while(|row| !row.trim().is_empty())
            {
                assert!(row.starts_with(&" ".repeat(value_column)), "{rendered}");
                assert!(!row[value_column..].starts_with(' '), "{rendered}");
            }
        }
        let model_rows = &rows[starts[2]..starts[3]];
        assert!(model_rows.len() > 1, "{rendered}");
        assert_eq!(
            model_rows
                .iter()
                .map(|row| row[value_column..].trim_end())
                .collect::<Vec<_>>()
                .join(" "),
            "glm-5.2 · 1M context · 1M input · 131k output · trusted catalog v5",
            "{rendered}"
        );
        if width == 80 {
            assert_eq!(model_rows.last().expect("model continuation").trim(), "v5");
        }
        let value_width = usize::from(width) - value_column;
        for chunk in 0..3 {
            let row = rows[starts[6] + chunk];
            assert_eq!(
                &row[value_column..],
                &destination[chunk * value_width..(chunk + 1) * value_width],
                "long path must use the full value width: {rendered}"
            );
        }
        let writes = rows[starts[6]..]
            .iter()
            .take_while(|row| !row.trim().is_empty())
            .map(|row| row[value_column..].trim_end())
            .collect::<Vec<_>>();
        assert_eq!(
            writes
                .concat()
                .split_once(',')
                .expect("destination comma")
                .0,
            destination,
            "long path must retain every character: {rendered}"
        );
        assert!(
            writes.join(" ").contains("then checks the configuration"),
            "{rendered}"
        );
    }
}

#[test]
fn setup_review_scrolls_to_the_last_wrapped_row_with_fixed_footer_keys() {
    for (width, height) in [(44, 16), (80, 24)] {
        for collisions in [false, true] {
            for down in [KeyCode::Down, KeyCode::PageDown] {
                let mut app = glm_environment_review();
                if collisions {
                    let mut preview = (0..60)
                        .map(|line| format!("merge line {line}: replace reviewed value"))
                        .collect::<Vec<_>>();
                    preview.push("merge-final".to_owned());
                    app.review_collisions(preview.join("\n"));
                } else {
                    app = app.with_destination(format!(
                        "/tmp/{}/config.toml",
                        "reviewed-directory/".repeat(90),
                    ));
                }
                let initial = render_setup(&app, width, height);
                let initial_scroll = app.review_scroll.get();
                assert!(initial_scroll.limit > 0, "{initial}");
                assert!(initial.contains("↑↓/PgUp/PgDn review · 1/"), "{initial}");
                let error = app.error.clone();
                app.on_key(key(down));
                let expected = if down == KeyCode::Down {
                    1
                } else {
                    initial_scroll.page
                };
                assert_eq!(
                    app.review_scroll.get().offset,
                    expected.min(initial_scroll.limit)
                );
                for _ in 0..initial_scroll.limit {
                    let buffer =
                        render_setup_buffer(&app, width, height, Theme::new().without_color());
                    let rows = setup_body_rows(&buffer);
                    let footer = rows[rows.len() - 2..].join("\n");
                    assert!(footer.contains("enter confirm"), "{footer}");
                    assert!(footer.contains("esc back"), "{footer}");
                    app.on_key(key(down));
                }
                let last = render_setup(&app, width, height);
                let scroll = app.review_scroll.get();
                assert_eq!(scroll.offset, scroll.limit);
                assert_eq!(app.error, error, "scrolling must preserve merge warnings");
                assert!(
                    last.split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .contains("then checks the configuration"),
                    "{last}"
                );
                if collisions {
                    assert!(last.contains("merge-final"), "{last}");
                    assert!(last.contains("only those values."), "{last}");
                }
                assert!(last.contains(&format!("{}/{}", scroll.limit + 1, scroll.limit + 1)));
                app.on_key(key(KeyCode::PageUp));
                assert_eq!(
                    app.review_scroll.get().offset,
                    scroll.limit.saturating_sub(scroll.page)
                );
                let offset = app.review_scroll.get().offset;
                app.on_key(key(KeyCode::Up));
                assert_eq!(app.review_scroll.get().offset, offset.saturating_sub(1));
                for _ in 0..scroll.limit {
                    app.on_key(key(KeyCode::PageUp));
                }
                assert_eq!(app.review_scroll.get().offset, 0);
                let top = render_setup(&app, width, height);
                assert!(
                    top.lines().any(|row| row.starts_with("  Provider")),
                    "{top}"
                );
                assert!(matches!(
                    app.on_key(key(KeyCode::Enter)),
                    SetupEffect::Submit { allow_collisions, .. } if allow_collisions == collisions
                ));
            }
        }
    }
}

#[test]
fn setup_review_scroll_clamps_on_resize_and_resets_after_back_or_new_preview() {
    let mut app = glm_environment_review();
    app.review_collisions("merge line\n".repeat(60));
    render_setup(&app, 44, 16);
    for _ in 0..100 {
        app.on_key(key(KeyCode::PageDown));
    }
    let narrow = app.review_scroll.get();
    render_setup(&app, 80, 24);
    let wide = app.review_scroll.get();
    assert!(wide.limit < narrow.limit);
    assert_eq!(wide.offset, wide.limit);
    app.review_collisions("replacement preview");
    assert_eq!(app.review_scroll.get().offset, 0);
    render_setup(&app, 44, 16);
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::BackTab));
    assert_eq!(app.review_scroll.get().offset, 0);
    assert!(app.collision_preview.is_none());
    assert!(matches!(
        app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        SetupEffect::Cancel
    ));
}
