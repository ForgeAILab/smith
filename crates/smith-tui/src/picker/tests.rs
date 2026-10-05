#[test]
fn step_key_tracks_the_title_without_repainting_filter_or_selection_changes() {
    use crate::Screen;

    let mut picker = ResourcePicker::new(
        "Choose model",
        vec![ResourceEntry::new("model", "Model", "model detail")],
        "No models",
    );
    let initial = picker.step_key();
    picker.on_key(key(KeyCode::Down));
    picker.paste("model");
    assert_eq!(picker.step_key(), initial);
    picker.title = "Choose provider".into();
    assert_ne!(picker.step_key(), initial);
}

use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

#[test]
fn empty_resume_guidance_replaces_the_footer_but_filters_keep_recovery_keys() {
    let mut picker = ResourcePicker::new(
        "Resume session",
        Vec::new(),
        "No sessions to resume in this project · esc exits",
    )
    .with_empty_guidance_keys();
    for width in [44, 80, 100] {
        assert!(picker.footer().rows(width).is_empty());
        let rendered = render_picker(&picker, width, 16);
        assert!(rendered.contains("esc exits"), "{rendered}");
        assert!(!rendered.contains("esc cancel"), "{rendered}");
        assert!(!rendered.contains("enter confirm"), "{rendered}");
        assert!(!rendered.contains("↑↓ choose"), "{rendered}");
        let rows = rendered.lines().collect::<Vec<_>>();
        let guidance_end = rows
            .iter()
            .position(|row| row.contains("esc exits"))
            .expect("empty resume guidance");
        assert!(
            rows[guidance_end + 1..]
                .iter()
                .all(|row| row.trim().is_empty()),
            "empty resume must have no footer row: {rendered}"
        );
    }
    assert_eq!(picker.on_key(key(KeyCode::Esc)), PickerOutcome::Cancelled);
    picker
        .entries
        .push(ResourceEntry::new("session", "Prompt", "session-id"));
    assert!(
        picker
            .footer()
            .rows(80)
            .join(" · ")
            .contains("enter confirm")
    );
    picker.query = "no matches".into();
    for width in [44, 100] {
        assert_eq!(
            picker.footer().rows(width).join(" · "),
            "ctrl+u clear filter · esc cancel"
        );
        let rendered = render_picker(&picker, width, 16);
        assert!(!rendered.contains("enter confirm"), "{rendered}");
        assert!(!rendered.contains("↑↓ choose"), "{rendered}");
    }
}

#[test]
fn empty_session_pickers_keep_the_cancel_hint_without_an_explicit_flag() {
    let picker = ResourcePicker::new(
        "Resume session",
        Vec::new(),
        "No sessions to resume · esc cancel",
    );
    for width in [44, 80] {
        assert_eq!(picker.footer().hint(width), "esc cancel");
    }
}

#[test]
fn selected_session_id_stays_on_one_row_at_44_columns() {
    for id in [
        "session-6465-42d4-be69-180feb01a926",
        "session-2042b4df-6465-42d4-be69-180feb01a926",
        "会話-2042b4df-6465-42d4-be69-180feb01a926-too-long",
    ] {
        let picker = ResourcePicker::new(
            "Resume session",
            vec![ResourceEntry::new(id, "Explain lib.rs", id).description("2 min ago · 1 turn")],
            "empty",
        );
        let rows = picker_lines(&picker, 10, 44, Theme::new());
        assert_eq!(rows.len(), 2, "{rows:?}");
        let detail = rows[1].to_string();
        if id.width() <= 42 {
            assert_eq!(detail, format!("  {id}"));
        } else {
            assert!(detail.starts_with("  "), "{detail}");
            assert!(detail.ends_with('…'), "{detail}");
            assert!(
                id.starts_with(detail.trim_start().trim_end_matches('…')),
                "{detail}"
            );
            assert!(detail.width() <= 44, "{detail}");
        }
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn render_picker(picker: &ResourcePicker, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| picker.draw(frame, frame.area(), Theme::new().without_color()))
        .expect("draw");
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn fixed_choices_confirm_digits_and_ignore_filter_input() {
    let mut picker = ResourcePicker::choices(
        "Method",
        vec![
            ResourceEntry::new("one", "One", "detail"),
            ResourceEntry::new("two", "Two", "detail").disabled("unavailable"),
            ResourceEntry::new("three", "Three", "detail"),
        ],
        "empty",
    );
    for character in ['a', '2', '4', '9', '0'] {
        assert_eq!(
            picker.on_key(key(KeyCode::Char(character))),
            PickerOutcome::Pending
        );
    }
    picker.paste("letters and digits 3");
    assert!(picker.query.is_empty());
    assert_eq!(picker.filtered_indices(), vec![0, 1, 2]);
    assert_eq!(
        picker.selected, 0,
        "invalid and disabled digits leave selection alone"
    );
    assert_eq!(
        picker.on_key(key(KeyCode::Char('3'))),
        PickerOutcome::Selected("three".into())
    );
    assert_eq!(picker.selected, 2);
    let screen = render_picker(&picker, 44, 16);
    assert!(screen.contains("❯ 3. Three"), "{screen}");
    assert!(!screen.contains("type to filter"), "{screen}");
    let mut inventory = ResourcePicker::new("Inventory", picker.entries, "empty");
    inventory.on_key(key(KeyCode::Char('3')));
    assert_eq!(inventory.query, "3", "inventory digits remain filter text");
}

#[test]
fn standalone_title_and_footer_are_unframed_and_content_sized() {
    let picker = ResourcePicker::choices(
        "Choose a method",
        vec![ResourceEntry::new("one", "First method", "selected detail")],
        "empty",
    );
    for (width, height) in [(44, 16), (100, 32)] {
        let screen = render_picker(&picker, width, height);
        let rows = screen.lines().collect::<Vec<_>>();
        assert!(rows[0].starts_with("  Choose a method"), "{screen}");
        assert!(rows[1].trim().is_empty());
        assert!(rows[2].starts_with("❯ 1. First method"));
        assert!(rows[3].starts_with("     selected detail"));
        for glyph in ['┌', '┐', '└', '┘', '│', '╭', '╮', '╰', '╯'] {
            assert!(!screen.contains(glyph), "{screen}");
        }
        let footer = rows
            .iter()
            .position(|row| row.contains("enter confirm"))
            .expect("footer");
        assert_eq!(footer, 5, "one blank row separates the content and footer");
        assert!(rows[footer].starts_with("  "));
        assert!(rows[footer..].join("\n").contains("esc cancel"));
        assert!(rows[footer..].join("\n").contains("↑↓ or 1–1 choose"));
    }
}

#[test]
fn embedded_inventory_keeps_five_choices_and_reaches_the_last_entry() {
    let mut picker = ResourcePicker::new(
        "Connect OpenRouter · Choose model",
        (0..30)
            .map(|index| {
                ResourceEntry::new(index.to_string(), format!("resource-{index}"), "metadata")
            })
            .collect(),
        "empty",
    );
    for (width, height) in [(44, 16), (100, 32)] {
        let mut app = crate::App::new("gpt-5.3", "~/work/api");
        app.transcript.push_user("retained transcript");
        app.composer.insert_str("retained draft");
        for selected in [0, 29] {
            picker.selected = selected;
            let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
            terminal
                .draw(|frame| {
                    crate::render::draw_with_screen(
                        frame,
                        &app,
                        &picker,
                        Theme::new().without_color(),
                    );
                })
                .expect("draw");
            let buffer = terminal.backend().buffer();
            let text = (0..height)
                .map(|y| {
                    (0..width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(
                text.lines().filter(|row| row.contains("resource-")).count(),
                COMPACT_VISIBLE_ENTRIES,
                "{text}"
            );
            for expected in [
                "retained transcript".to_owned(),
                "retained draft".to_owned(),
                format!("❯ resource-{selected}"),
                format!("{}/30", selected + 1),
                "enter confirm".to_owned(),
                "esc cancel".to_owned(),
            ] {
                assert!(text.contains(&expected), "{text}");
            }
        }
    }
}

#[test]
fn scroll_position_and_description_column_use_the_complete_filtered_list() {
    let entries = (0..20)
        .map(|index| {
            ResourceEntry::new(
                index.to_string(),
                if index == 19 {
                    "longer label".to_owned()
                } else {
                    format!("row-{index}")
                },
                "metadata",
            )
        })
        .collect();
    let mut picker = ResourcePicker::new("Inventory", entries, "empty");
    for compact in [false, true] {
        let before = entry_view(&picker, 5, 80, Theme::new(), compact);
        assert!(before.scrolling);
        let column = before.lines[0]
            .to_string()
            .find("metadata")
            .expect("description");
        picker.selected = 19;
        let after = entry_view(&picker, 5, 80, Theme::new(), compact);
        let last = after
            .lines
            .iter()
            .find(|line| line.to_string().contains("longer label"))
            .expect("last entry")
            .to_string();
        assert_eq!(
            last.find("metadata"),
            Some(column),
            "scrolling keeps columns stable"
        );
        let screen = render_picker(&picker, 44, 8);
        assert!(
            screen
                .lines()
                .next()
                .expect("heading")
                .trim_end()
                .ends_with("20/20"),
            "{screen}"
        );
        assert!(
            screen.contains("enter confirm") && screen.contains("esc cancel"),
            "{screen}"
        );
        picker.selected = 0;
    }
}

#[test]
fn shared_footer_prioritizes_enter_and_escape_at_44_columns() {
    for choices in [None, Some(4)] {
        let footer = ScreenFooter::List {
            choices,
            back: true,
        };
        let rows = footer.rows(44);
        assert!(rows[0].starts_with("enter confirm · esc back"));
        assert!(rows.iter().all(|row| row.width() + 2 <= 44));
        assert!(footer.rows(100)[0].starts_with("↑↓"));
    }
    assert_eq!(
        ScreenFooter::Field { back: true }.rows(44),
        ["enter continue · esc back"]
    );
    assert_eq!(
        ScreenFooter::Review {
            back: true,
            scroll: None
        }
        .rows(44),
        ["enter confirm · esc back"]
    );
    assert_eq!(
        ScreenFooter::Progress { back: true }.rows(44),
        ["esc back · ctrl+c cancel"]
    );
    assert_eq!(
        ScreenFooter::Progress { back: false }.rows(44),
        ["esc cancel"]
    );
}

#[test]
fn picker_distinguishes_back_from_whole_flow_cancel_and_ignores_releases() {
    let mut picker = ResourcePicker::choices("Method", Vec::new(), "empty").with_back(true);
    assert_eq!(
        picker.on_event(ScreenEvent::Key(key(KeyCode::Esc))),
        Step::Outcome(FlowOutcome::Back)
    );
    assert_eq!(
        picker.on_event(ScreenEvent::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        ))),
        Step::Outcome(FlowOutcome::Cancelled)
    );
    assert_eq!(
        picker.on_key(KeyEvent::new_with_kind(
            KeyCode::Esc,
            KeyModifiers::NONE,
            KeyEventKind::Release
        )),
        PickerOutcome::Pending
    );
    picker = picker.with_back(false);
    assert_eq!(
        picker.on_event(ScreenEvent::Key(key(KeyCode::Esc))),
        Step::Outcome(FlowOutcome::Cancelled)
    );
}

#[test]
fn filtering_selection_and_cancellation_are_pure() {
    let mut picker = ResourcePicker::new(
        "Models",
        vec![
            ResourceEntry::new("zai/glm", "zai/glm", "GLM"),
            ResourceEntry::new("router/gpt", "router/gpt", "OpenRouter"),
        ],
        "run setup",
    );
    assert_eq!(
        picker.on_key(key(KeyCode::Char('g'))),
        PickerOutcome::Pending
    );
    assert_eq!(picker.filtered_indices(), vec![0, 1]);
    assert_eq!(
        picker.on_key(key(KeyCode::Char('l'))),
        PickerOutcome::Pending
    );
    assert_eq!(picker.filtered_indices(), vec![0]);
    assert_eq!(
        picker.on_key(key(KeyCode::Enter)),
        PickerOutcome::Selected("zai/glm".into())
    );
    assert_eq!(picker.on_key(key(KeyCode::Esc)), PickerOutcome::Cancelled);
}

#[test]
fn disabled_and_empty_entries_cannot_be_selected() {
    let mut picker = ResourcePicker::new(
        "Providers",
        vec![ResourceEntry::new("broken", "broken", "").disabled("missing model")],
        "run setup",
    );
    assert_eq!(picker.on_key(key(KeyCode::Enter)), PickerOutcome::Pending);
    picker.query = "absent".into();
    assert_eq!(picker.on_key(key(KeyCode::Enter)), PickerOutcome::Pending);
}

#[test]
fn filtering_keeps_selected_detail_and_short_description_searchable() {
    let mut picker = ResourcePicker::new(
        "Choose model",
        vec![
            ResourceEntry::new("local/model", "model", "project config · input 124k")
                .description("local · 128k context"),
        ],
        "run setup",
    );
    for query in ["project config", "input 124k", "128k context"] {
        picker.query = query.to_owned().into();
        assert_eq!(picker.filtered_indices(), [0]);
        assert_eq!(
            picker.on_key(key(KeyCode::Enter)),
            PickerOutcome::Selected("local/model".to_owned())
        );
    }
}

#[test]
fn selected_detail_omits_facts_already_in_the_description() {
    for (entry, expected, repeated) in [
        (
            ResourceEntry::new("0", "1", "env:FIRST · 25% used").description("25% used"),
            "env:FIRST",
            vec!["25% used"],
        ),
        (
            ResourceEntry::new(
                "dev",
                "dev",
                "build · use main · zai/glm-5.3 · coding · rev r1",
            )
            .description("build · coding"),
            "use main · zai/glm-5.3 · rev r1",
            vec!["build", "coding"],
        ),
    ] {
        assert_eq!(entry.selected_detail(), expected);
        let picker = ResourcePicker::new("Choose resource", vec![entry], "no resources");
        for width in [44, 100] {
            for theme in [Theme::new(), Theme::new().without_color()] {
                let lines = picker_entry_lines(&picker, 2, width, theme);
                assert_eq!(lines.len(), 2);
                let text = lines
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n");
                for fact in &repeated {
                    assert_eq!(text.matches(*fact).count(), 1, "{text}");
                }
                assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
            }
        }
    }
    let same = ResourceEntry::new("one", "one", "short · description");
    assert!(same.selected_detail().is_empty());
    assert_eq!(
        same.disabled("missing credential").selected_detail(),
        "missing credential"
    );
}

#[test]
fn empty_inventory_and_unmatched_filter_have_distinct_guidance() {
    let empty = ResourcePicker::new(
        "Models",
        Vec::new(),
        "No local model is selectable · run smith setup add-model",
    );
    let empty_lines = picker_lines(
        &empty,
        3,
        44,
        Theme::from_env().without_color().without_motion(),
    );
    let empty_text = empty_lines
        .iter()
        .map(|line| line.to_string().trim().to_owned())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(empty_text.contains("run smith setup add-model"));
    assert!(!empty_text.contains("No matches"));

    let mut filtered = ResourcePicker::new(
        "Models",
        vec![ResourceEntry::new("local/model", "local/model", "local")],
        "No local model is selectable · run smith setup add-model",
    );
    filtered.query = "does-not-exist".to_owned().into();
    filtered.selected = 4;
    let filtered_lines = picker_lines(
        &filtered,
        3,
        44,
        Theme::from_env().without_color().without_motion(),
    );
    let filtered_text = filtered_lines
        .iter()
        .map(|line| line.to_string().trim().to_owned())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(filtered_text.contains("No matches"), "{filtered_text}");
    assert!(
        filtered_text.contains("Ctrl+U clear filter"),
        "{filtered_text}"
    );

    assert_eq!(
        filtered.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL,)),
        PickerOutcome::Pending
    );
    assert!(filtered.query.is_empty());
    assert_eq!(filtered.selected, 0);
    assert_eq!(
        filtered.selected_entry().map(|entry| entry.id.as_str()),
        Some("local/model")
    );
}

#[test]
fn state_labels_stay_at_the_right_and_unavailable_stays_disabled() {
    let long_detail =
        "advertised capabilities, context window, output ceiling, and request budget ".repeat(4);
    let mut picker = ResourcePicker::new(
        "Models",
        vec![
            ResourceEntry::new("local/model", "model", long_detail.clone())
                .description("local")
                .active(true),
            ResourceEntry::new("broken/model", "broken", long_detail)
                .description("local")
                .disabled("missing limits"),
        ],
        "run setup",
    );

    let mut terminal = Terminal::new(TestBackend::new(44, 30)).expect("terminal");
    terminal
        .draw(|frame| {
            draw_resource_picker(
                frame,
                frame.area(),
                &picker,
                Theme::from_env().without_color().without_motion(),
            );
        })
        .expect("draw");
    let rendered = (0..terminal.backend().buffer().area.height)
        .map(|y| {
            (0..terminal.backend().buffer().area.width)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let current = rendered.find("current").expect("current state is visible");
    let metadata = rendered.find("advertised").expect("metadata is visible");
    assert!(current < metadata, "{rendered}");
    assert!(
        rendered.contains("unavailable: missing limits"),
        "{rendered}"
    );

    picker.query = "broken".to_owned().into();
    assert_eq!(picker.on_key(key(KeyCode::Enter)), PickerOutcome::Pending);
}

#[test]
fn narrow_no_color_picker_keeps_active_disabled_and_controls_textual() {
    let mut picker = ResourcePicker::new(
        "Models",
        vec![
            ResourceEntry::new("zai/glm", "zai/glm", "trusted").active(true),
            ResourceEntry::new("broken", "broken", "local").disabled("missing limits"),
            ResourceEntry::new("router/gpt", "router/gpt", "explicit"),
        ],
        "run setup",
    );
    let mut terminal = Terminal::new(TestBackend::new(40, 8)).expect("terminal");
    let mut render_picker = |picker: &ResourcePicker| {
        terminal
            .draw(|frame| {
                draw_resource_picker(
                    frame,
                    frame.area(),
                    picker,
                    Theme::from_env().without_color().without_motion(),
                );
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let rendered = render_picker(&picker);
    assert!(rendered.contains("✓ current"), "{rendered}");
    assert!(
        rendered
            .lines()
            .any(|line| line.contains("broken") && line.trim_end().ends_with("unavailable")),
        "{rendered}"
    );
    assert!(!rendered.contains("missing limits"), "{rendered}");
    assert!(rendered.contains("enter confirm"), "{rendered}");
    assert!(rendered.contains('❯'), "{rendered}");

    picker.on_key(key(KeyCode::Down));
    let rendered = render_picker(&picker);
    assert!(
        rendered
            .lines()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|rows| { rows[0].contains("❯ broken") && rows[1].contains("missing limits") }),
        "{rendered}"
    );
}

#[test]
fn hundreds_of_catalog_entries_remain_bounded_searchable_and_deterministic() {
    let entries = (0..600)
        .map(|index| {
            let id = format!("router/vendor/model-{index:04}");
            let detail = if index == 599 {
                "OpenRouter · tools+reasoning+vision"
            } else {
                "OpenRouter · tools"
            };
            let entry = ResourceEntry::new(&id, format!("Model {index:04}"), detail);
            if index == 400 {
                entry.disabled("catalog model does not support tool calling")
            } else {
                entry
            }
        })
        .collect();
    let mut picker = ResourcePicker::new("Models", entries, "run setup");

    picker.query = "vision".to_owned().into();
    assert_eq!(picker.filtered_indices(), [599]);
    assert_eq!(
        picker.on_key(key(KeyCode::Enter)),
        PickerOutcome::Selected("router/vendor/model-0599".to_owned())
    );

    picker.query.clear();
    picker.selected = 599;
    let lines = picker_lines(
        &picker,
        6,
        80,
        Theme::from_env().without_color().without_motion(),
    );
    assert!(lines.len() <= 6, "rendering is bounded to the viewport");
    let rendered = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("Model 0599"), "{rendered}");
    assert!(!rendered.contains("Model 0000"), "{rendered}");
    assert_eq!(picker.on_key(key(KeyCode::Esc)), PickerOutcome::Cancelled);
}
