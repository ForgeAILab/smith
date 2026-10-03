// Commands and resources share the same fixed-height column grammar.

use crate::commands;

#[test]
fn list_descriptions_elide_at_word_boundaries_in_display_cells() {
    use crate::render::lists::clip_words;
    for (text, budget, expected) in [
        ("Switch to next model", 14, "Switch to…"),
        ("Switch model", 12, "Switch model"),
        ("unbroken_identifier", 5, "…"),
        ("你好 世界 words", 8, "你好…"),
        ("Switch model", 1, "…"),
        ("Switch model", 0, ""),
    ] {
        let clipped = clip_words(text, budget);
        assert_eq!(clipped, expected);
        assert!(clipped.width() <= budget);
    }
}

#[test]
fn command_columns_and_selected_detail_preserve_five_choices_at_supported_sizes() {
    let mut app = App::new("model", "~/project");
    app.transcript.push_user("Keep this conversation visible");
    app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let commands = &commands::COMMANDS[..5];
    let column = 2
        + commands
            .iter()
            .map(|command| command.name.width() + 1)
            .max()
            .unwrap()
        + 2;
    for (width, height) in [(44, 16), (80, 24), (100, 32)] {
        for theme in [
            Theme::new().without_motion(),
            Theme::new().without_color().without_motion(),
        ] {
            let screen = render(&app, width, height, theme);
            let rows = screen
                .lines()
                .filter(|line| line.starts_with("  /") || line.starts_with("❯ /"))
                .collect::<Vec<_>>();
            assert_eq!(rows.len(), 5, "{screen}");
            for (row, command) in rows.iter().zip(commands) {
                let invocation = format!("/{}", command.name);
                assert!(
                    row.starts_with(&format!(
                        "{}{}",
                        if command.name == "goal" { "❯ " } else { "  " },
                        invocation
                    )),
                    "{screen}"
                );
                let description = row.chars().skip(column).collect::<String>();
                assert!(description.chars().next().unwrap().is_uppercase(), "{row}");
                assert!(
                    command
                        .description
                        .starts_with(description.trim_end_matches('…')),
                    "word was split: {row}"
                );
                assert!(!row.contains('['), "grammar entered a choice row: {row}");
            }
            let lines = screen.lines().collect::<Vec<_>>();
            let selected_y = lines
                .iter()
                .position(|line| line.starts_with("❯ /goal "))
                .unwrap();
            let detail = lines[selected_y + 1];
            assert!(
                detail.starts_with(&format!("{}[OBJECTIVE", " ".repeat(column))),
                "{screen}"
            );
            assert!(!screen.contains("[NAME|default]"), "{screen}");
            assert!(!screen.contains("[PROVIDER/MODEL]"), "{screen}");
            let composer_y = lines.iter().position(|line| *line == "> /").unwrap();
            assert!(selected_y + 1 < composer_y, "{screen}");
            assert!(
                screen.contains("Keep this conversation visible"),
                "{screen}"
            );
            assert!(
                screen
                    .lines()
                    .all(|line| line.width() <= usize::from(width)),
                "{screen}"
            );
        }
    }

    app.apply(&event(RuntimeEvent::TurnStarted));
    app.set_running_tasks(
        (0..8)
            .map(|index| crate::app::RunningTaskSummary {
                task_id: format!("task:{index}"),
                command_hint: "background work".to_owned(),
            })
            .collect(),
    );
    let screen = render(&app, 44, 16, Theme::new().without_color().without_motion());
    assert_eq!(
        screen
            .lines()
            .filter(|line| line.starts_with("  /") || line.starts_with("❯ /"))
            .count(),
        5,
        "{screen}"
    );
    assert!(screen.contains("[OBJECTIVE"), "{screen}");
    assert!(screen.contains("Working"), "{screen}");
}

#[test]
fn command_menu_palette_and_help_keep_the_registry_order() {
    let help = commands::help();
    let guide = help
        .primary
        .iter()
        .chain(&help.advanced)
        .collect::<Vec<_>>();
    assert_eq!(guide.len(), commands::COMMANDS.len());
    for (entry, command) in guide.iter().zip(commands::COMMANDS) {
        assert_eq!(entry.name, command.name);
        assert_eq!(entry.description, command.description);
        assert_eq!(entry.argument_hint, command.argument_hint);
    }
    for palette in [false, true] {
        let mut app = App::new("model", "~/project");
        app.transcript.push_user("Existing conversation");
        app.on_key(if palette {
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)
        } else {
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)
        });
        for selected in 0..commands::COMMANDS.len() {
            let screen = render(&app, 80, 24, Theme::new().without_color());
            let rows = screen
                .lines()
                .filter(|line| line.starts_with("  /") || line.starts_with("❯ /"))
                .collect::<Vec<_>>();
            assert_eq!(rows.len(), 5, "{screen}");
            let names = rows
                .iter()
                .map(|line| {
                    line.trim_start_matches("❯ ")
                        .split_whitespace()
                        .next()
                        .unwrap()
                        .trim_start_matches('/')
                })
                .collect::<Vec<_>>();
            let start = commands::COMMANDS
                .iter()
                .position(|command| command.name == names[0])
                .unwrap();
            assert_eq!(
                names,
                guide[start..start + 5]
                    .iter()
                    .map(|command| command.name.as_str())
                    .collect::<Vec<_>>(),
                "{screen}"
            );
            assert!(
                rows.iter()
                    .any(|row| row
                        .starts_with(&format!("❯ /{} ", commands::COMMANDS[selected].name))),
                "{screen}"
            );
            app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
    }
}

#[test]
fn command_name_column_tracks_only_the_visible_names() {
    let mut app = App::new("model", "~/project");
    app.transcript.push_user("Existing conversation");
    for character in "/switch".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    let screen = render(&app, 80, 24, Theme::new().without_color());
    let rows = screen
        .lines()
        .filter(|line| line.starts_with("  /") || line.starts_with("❯ /"))
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 3, "{screen}");
    for line in rows {
        assert_eq!(
            line.find("Switch").map(|at| line[..at].width()),
            Some(13),
            "{screen}"
        );
    }
}

#[test]
fn list_columns_keep_dim_descriptions_and_detail_without_color() {
    use crate::render::lists::{detail_line, list_row};
    for theme in [Theme::new(), Theme::new().without_color()] {
        let row = list_row(
            "模型",
            "local · 128k context",
            "✓ current",
            true,
            4,
            44,
            Tone::Accent,
            theme,
        );
        assert_eq!(row.width(), 44);
        assert_eq!(row.spans[0].content, "❯ ");
        assert_eq!(row.spans[3].content, "local · 128k context");
        assert!(row.spans[3].style.add_modifier.contains(Modifier::DIM));
        let detail = detail_line("project config · input 124k", 4, 44, theme);
        assert_eq!(detail.spans[0].content, "        ");
        assert!(detail.spans[1].style.add_modifier.contains(Modifier::DIM));
    }
    let theme = Theme::new().without_color();
    let row = list_row(
        "model",
        "local",
        "✓ current",
        true,
        5,
        44,
        Tone::Accent,
        theme,
    );
    assert!(
        row.spans
            .iter()
            .all(|span| span.style.fg.is_none() && span.style.bg.is_none())
    );
}

#[test]
fn picker_columns_dock_states_and_show_only_the_selected_provenance() {
    use crate::picker::{ResourceEntry, ResourcePicker};
    let entries = (0..10)
        .map(|index| {
            let label = if index == 0 {
                "example-model".to_owned()
            } else {
                format!("model-{index}")
            };
            ResourceEntry::new(
                format!("local/{label}"),
                label,
                format!("source-{index} · input 124k · output 4k"),
            )
            .description("local · 128k context")
            .active(index == 0)
        })
        .collect::<Vec<_>>();
    for (width, height) in [(44, 16), (80, 24)] {
        for theme in [
            Theme::new().without_motion(),
            Theme::new().without_color().without_motion(),
        ] {
            let mut app = App::new("model", "~/project");
            app.overlay = Some(Overlay::ResourcePicker {
                picker: ResourcePicker::new("Choose model", entries.clone(), "run setup"),
                target: crate::app::ResourceTarget::Model,
                restore_on_escape: "/model".to_owned(),
            });
            let screen = render(&app, width, height, theme);
            let lines = screen.lines().collect::<Vec<_>>();
            let selected_y = lines
                .iter()
                .position(|line| line.starts_with("❯ example-model"))
                .unwrap();
            let current = lines[selected_y];
            assert_eq!(current.width(), usize::from(width), "{screen}");
            assert!(current.ends_with("✓ current"), "{screen}");
            assert_eq!(
                current[..current.find("local").unwrap()].width(),
                17,
                "{screen}"
            );
            assert!(
                lines[selected_y + 1].starts_with("                 source-0"),
                "{screen}"
            );
            assert!(!current.contains("source-0"), "{screen}");
            assert_eq!(screen.matches("source-").count(), 1, "{screen}");
            for index in 1..5 {
                let row = lines
                    .iter()
                    .find(|line| line.starts_with(&format!("  model-{index} ")))
                    .unwrap();
                assert_eq!(row.find("local"), Some(17), "{screen}");
            }
            assert!(!screen.contains("model-5"), "{screen}");
            assert!(lines[selected_y - 1].ends_with("1/10"), "{screen}");
            assert_eq!(
                lines[selected_y - 1].width(),
                usize::from(width),
                "{screen}"
            );
            app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
            let moved = render(&app, width, height, theme);
            assert!(moved.contains("❯ model-1"), "{moved}");
            assert!(moved.contains("source-1"), "{moved}");
            assert!(!moved.contains("source-0"), "{moved}");
            assert_eq!(moved.matches("source-").count(), 1, "{moved}");
        }
    }
}

#[test]
fn picker_state_survives_long_identity_and_unavailable_reason_at_44_columns() {
    use crate::picker::{ResourceEntry, ResourcePicker};
    let mut app = App::new("model", "~/project");
    app.overlay = Some(Overlay::ResourcePicker {
        picker: ResourcePicker::new(
            "Choose model",
            vec![
                ResourceEntry::new(
                    "local/model",
                    "model",
                    "project config · input 124k · output 4k",
                )
                .description("local · 128k context")
                .active(true),
                ResourceEntry::new(
                    "local/long",
                    "a-very-long-model-identity-without-spaces",
                    "catalog advertised limits",
                )
                .description("local · 200k context")
                .disabled("missing enforceable limits for this configured endpoint"),
            ],
            "run setup",
        ),
        target: crate::app::ResourceTarget::Model,
        restore_on_escape: "/model".to_owned(),
    });
    let theme = Theme::new().without_color().without_motion();
    let screen = render(&app, 44, 16, theme);
    for state in ["✓ current", "unavailable"] {
        let row = screen.lines().find(|line| line.ends_with(state)).unwrap();
        assert_eq!(row.width(), 44, "{screen}");
    }
    app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let screen = render(&app, 44, 16, theme);
    assert!(screen.contains("❯ a-very-long"), "{screen}");
    assert!(
        screen
            .lines()
            .any(|line| line.trim_start().starts_with("missing…")),
        "{screen}"
    );
    assert!(screen.lines().all(|line| line.width() <= 44), "{screen}");
    assert_eq!(
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        None
    );
    assert!(matches!(app.overlay, Some(Overlay::ResourcePicker { .. })));
}

#[test]
fn reference_picker_lists_files_before_agents_and_hides_model_metadata() {
    use crate::picker::ResourceEntry;
    let mut app = App::new("model", "~/project");
    app.set_resources(crate::app::RuntimeResources {
        files: vec![ResourceEntry::new(
            "file:src/lib.rs",
            "src/lib.rs",
            "file · 42 bytes",
        )],
        child_agents: vec![ResourceEntry::new(
            "agent:review",
            "review",
            "child profile · hidden-model · input 128k",
        )],
        ..crate::app::RuntimeResources::default()
    });
    app.on_key(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE));
    let theme = Theme::new().without_color().without_motion();
    for selected in ["src/lib.rs", "review"] {
        let screen = render(&app, 44, 16, theme);
        let file = screen.find("src/lib.rs").unwrap();
        let agent = screen.find("review").unwrap();
        assert!(file < agent, "{screen}");
        assert!(
            !screen.contains("hidden-model") && !screen.contains("input 128k"),
            "{screen}"
        );
        let row = screen
            .lines()
            .find(|line| line.starts_with(&format!("❯ {selected}")))
            .unwrap();
        if selected == "review" {
            assert!(row.ends_with("agent"), "{screen}");
        }
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
}

#[test]
fn runtime_selector_commands_share_the_row_and_selected_detail_grammar() {
    use crate::app::{ResourceTarget, RuntimeResources};
    use crate::picker::ResourceEntry;
    for (command, target) in [
        ("model", ResourceTarget::Model),
        ("provider", ResourceTarget::Provider),
        ("profile", ResourceTarget::Profile),
        ("resume", ResourceTarget::Resume),
        ("account", ResourceTarget::Account),
        ("connect", ResourceTarget::Connect),
        ("disconnect", ResourceTarget::Disconnect),
    ] {
        let entries = vec![
            ResourceEntry::new("local/choice", "choice", "project config · selected limits")
                .description("short description")
                .active(true),
        ];
        let mut app = App::new("model", "~/project");
        app.set_resources(RuntimeResources {
            models: entries.clone(),
            providers: entries.clone(),
            profiles: entries.clone(),
            sessions: entries.clone(),
            accounts: entries.clone(),
            connections: entries.clone(),
            disconnections: entries,
            ..RuntimeResources::default()
        });
        for character in format!("/{command}").chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            matches!(app.overlay, Some(Overlay::ResourcePicker { target: actual, .. }) if actual == target)
        );
        let screen = render(&app, 44, 16, Theme::new().without_color());
        let lines = screen.lines().collect::<Vec<_>>();
        let selected = lines
            .iter()
            .position(|line| line.starts_with("❯ choice "))
            .unwrap();
        assert!(
            lines[selected].contains("short description"),
            "{command}: {screen}"
        );
        assert!(
            lines[selected].ends_with("✓ current"),
            "{command}: {screen}"
        );
        assert!(
            lines[selected + 1]
                .trim_start()
                .starts_with("project config"),
            "{command}: {screen}"
        );
    }
}
