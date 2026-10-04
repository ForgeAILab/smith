// Informational results retain their content while reflowing into columns.

const INFORMATIONAL_CONTROLS: &str = "controllable · switch optional · efforts none · Z.AI Coding Plan thinking switch (catalog-advertised reasoning model)";

fn informational_context_report() -> smith_client::context_report::ContextReport {
    use smith_client::context_report::*;
    ContextReport {
        available_windows: vec![ContextWindow {
            name: "128k".to_owned(),
            active: true,
        }],
        summary: "glm-4.7 · input unknown (not planned yet)".to_owned(),
        usage: ContextUsage::Unavailable,
        categories: vec![
            ContextCategory {
                kind: ContextCategoryKind::System,
                label: "system instructions".to_owned(),
                tokens: 0,
                value: "unknown (not counted yet)".to_owned(),
            },
            ContextCategory {
                kind: ContextCategoryKind::Tool,
                label: "tool schemas".to_owned(),
                tokens: 0,
                value: "unknown (not counted yet)".to_owned(),
            },
        ],
        free_input: ContextCapacity {
            tokens: 124_000,
            value: "124k".to_owned(),
        },
        reserve: ContextCapacity {
            tokens: 4_000,
            value: "4k".to_owned(),
        },
        model_window: "128k total · 124k input budget".to_owned(),
        counting: "unknown (not planned yet)".to_owned(),
        compaction: ContextCompaction::Enabled {
            recovery_target: "74.4k".to_owned(),
        },
        tool_context: "offload above 8192 serialized bytes · artifact pages up to 2048 bytes"
            .to_owned(),
        provider_input: "unknown".to_owned(),
        cache_read: "unknown".to_owned(),
        cache: "state unknown · maintenance calls 0".to_owned(),
        reasoning: "provider default · provider/model default".to_owned(),
        reasoning_controls: INFORMATIONAL_CONTROLS.repeat(3),
    }
}

fn informational_text(lines: &[Line<'static>]) -> Vec<String> {
    lines.iter().map(ToString::to_string).collect()
}

fn informational_value_column(line: &Line<'static>) -> usize {
    line.spans
        .iter()
        .take(line.spans.len() - 1)
        .map(|span| span.content.width())
        .sum()
}

#[test]
fn informational_columns_keep_whole_words_and_hanging_values_at_44_and_100() {
    for width in [44, 100] {
        let mut status = status_report();
        status.reasoning_controls = INFORMATIONAL_CONTROLS.repeat(3);
        let context = informational_context_report();
        for (lines, value, next_label) in [
            (
                render_status_card(&status, width, Theme::new()),
                status.reasoning_controls.as_str(),
                Some("  prompt cache"),
            ),
            (
                render_context_report(&context, width, Theme::new()),
                context.reasoning_controls.as_str(),
                None,
            ),
        ] {
            let text = informational_text(&lines);
            assert!(
                lines.iter().all(|line| line.width() <= usize::from(width)),
                "{text:#?}"
            );
            let start = text
                .iter()
                .position(|line| line.starts_with("  reasoning controls  "))
                .unwrap();
            let end = next_label
                .and_then(|label| {
                    text[start + 1..]
                        .iter()
                        .position(|line| line.starts_with(label))
                        .map(|index| start + 1 + index)
                })
                .unwrap_or(lines.len());
            let column = informational_value_column(&lines[start]);
            assert!(end > start + 1, "the value should wrap at {width} columns");
            let words = text[start..end]
                .iter()
                .enumerate()
                .flat_map(|(index, row)| {
                    if index > 0 {
                        assert!(row.starts_with(&" ".repeat(column)), "{row}");
                    }
                    row.chars()
                        .skip(column)
                        .collect::<String>()
                        .split_whitespace()
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                words,
                value
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
                "{width} columns: {text:#?}"
            );
        }
    }
}

#[test]
fn informational_status_and_context_fields_share_each_reports_value_column() {
    for width in [44, 100] {
        let status = render_status_card(&status_report(), width, Theme::new());
        let mut columns = Vec::new();
        for label in [
            "session",
            "profile",
            "provider",
            "model",
            "permission",
            "reasoning controls",
            "project",
            "Git",
            "cost",
        ] {
            let row = status
                .iter()
                .find(|line| line.spans.iter().any(|span| span.content == label))
                .unwrap();
            columns.push(informational_value_column(row));
        }
        assert!(
            columns.iter().all(|column| *column == columns[0]),
            "{columns:?}"
        );
        let context = render_context_report(&informational_context_report(), width, Theme::new());
        let mut columns = Vec::new();
        for label in ["counting", "compaction", "cache", "reasoning controls"] {
            let row = context
                .iter()
                .find(|line| line.spans.iter().any(|span| span.content == label))
                .unwrap();
            columns.push(informational_value_column(row));
        }
        // The legend and the long window label use that same value column,
        // even when a label needs more than one row.
        for value in ["128k (active)", "124k", "4k"] {
            let row = context
                .iter()
                .find(|line| line.spans.last().is_some_and(|span| span.content == value))
                .unwrap();
            columns.push(informational_value_column(row));
        }
        assert!(
            columns.iter().all(|column| *column == columns[0]),
            "{columns:?}"
        );
    }
}

#[test]
fn informational_project_paths_are_shortened_from_the_left_in_display_cells() {
    let mut report = status_report();
    report.project = format!("/{}終点/workspace/api", "a long directory/".repeat(12));
    for width in [44, 100] {
        let lines = render_status_card(&report, width, Theme::new());
        let row = lines
            .iter()
            .find(|line| line.spans.iter().any(|span| span.content == "project"))
            .unwrap();
        let column = informational_value_column(row);
        let path = row.spans.last().unwrap().content.as_ref();
        assert!(path.starts_with('…'), "{path}");
        assert!(path.ends_with("/workspace/api"), "{path}");
        assert!(report.project.ends_with(path.strip_prefix('…').unwrap()));
        assert!(path.width() <= usize::from(width) - column);
        assert!(!lines.iter().any(|line| line.to_string().contains('╭')));
    }
    let lines = render_status_card(&status_report(), 44, Theme::new());
    assert!(
        lines
            .iter()
            .any(|line| line.to_string().ends_with("~/work/api"))
    );
}

#[test]
fn informational_help_uses_registry_order_and_an_aligned_plain_word_key_table() {
    let report = crate::commands::help();
    for width in [44, 100] {
        let lines = render_help_report(&report, width, Theme::new());
        let text = informational_text(&lines);
        assert_eq!(text[0], "  Start here");
        assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
        let start = text.iter().position(|row| row == "  Primary").unwrap();
        let keys = text.iter().position(|row| row == "  Keys").unwrap();
        let commands = lines[start..keys]
            .iter()
            .filter_map(|line| line.spans.iter().find(|span| span.content.starts_with('/')))
            .collect::<Vec<_>>();
        assert_eq!(
            commands
                .iter()
                .map(|span| span.content.trim_start_matches('/'))
                .collect::<Vec<_>>(),
            crate::commands::COMMANDS
                .iter()
                .map(|command| command.name)
                .collect::<Vec<_>>()
        );
        let columns = lines[start..keys]
            .iter()
            .filter(|line| line.spans.iter().any(|span| span.content.starts_with('/')))
            .map(informational_value_column)
            .collect::<Vec<_>>();
        assert!(columns.iter().all(|column| *column == columns[0]));
        for name in commands {
            assert_eq!(name.style.fg, Some(Color::Cyan));
            assert!(
                !name.content.contains('['),
                "argument grammar belongs after the description"
            );
        }
        for command in report.primary.iter().chain(&report.advanced) {
            let name = format!("/{}", command.name);
            let command_start = lines[start..keys]
                .iter()
                .position(|line| line.spans.iter().any(|span| span.content == name))
                .unwrap()
                + start;
            let end = lines[command_start + 1..keys]
                .iter()
                .position(|line| {
                    line.spans.iter().any(|span| span.content.starts_with('/'))
                        || matches!(line.to_string().as_str(), "  Advanced" | "  Keys")
                })
                .map_or(keys, |index| command_start + 1 + index);
            let column = informational_value_column(&lines[command_start]);
            let values = text[command_start..end]
                .iter()
                .map(|row| row.chars().skip(column).collect::<String>())
                .collect::<Vec<_>>();
            let description = values
                .iter()
                .take_while(|value| !value.trim_start().starts_with('['))
                .flat_map(|value| value.split_whitespace())
                .collect::<Vec<_>>();
            assert_eq!(
                description,
                command.description.split_whitespace().collect::<Vec<_>>(),
                "{width} columns: {name}"
            );
            for row in &text[command_start + 1..end] {
                assert!(
                    row.trim().is_empty() || row.starts_with(&" ".repeat(column)),
                    "{row}"
                );
            }
            if !command.argument_hint.is_empty() {
                let hint = lines[command_start + 1..end]
                    .iter()
                    .find(|line| {
                        line.spans
                            .last()
                            .is_some_and(|span| span.content.starts_with('['))
                    })
                    .expect("argument grammar remains available below the description");
                assert!(
                    hint.spans
                        .last()
                        .unwrap()
                        .style
                        .add_modifier
                        .contains(Modifier::DIM)
                );
            }
        }
        let tab = lines[keys..]
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content == "Tab while working")
            })
            .unwrap();
        let column = informational_value_column(tab);
        let tab_index = text
            .iter()
            .position(|row| row.starts_with("  Tab while working  "))
            .unwrap();
        let next = text[tab_index + 1..]
            .iter()
            .position(|row| row.starts_with("  Tab when idle"))
            .unwrap()
            + tab_index
            + 1;
        let description = text[tab_index..next]
            .iter()
            .map(|row| {
                row.chars()
                    .skip(column)
                    .collect::<String>()
                    .trim()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(description, "Queue for after this turn");
        assert!(!text.join("\n").contains("steers"));
        assert!(text.join("\n").contains("Ctrl+A / Ctrl+E"));
    }
    assert!(report.keys.iter().all(|row| {
        row.description
            .chars()
            .next()
            .is_some_and(char::is_uppercase)
    }));
    for (key, action) in [
        ("Enter while working", "Send now"),
        ("Tab while working", "Queue for after this turn"),
        ("Tab when idle", "Next profile (empty draft)"),
        ("Ctrl+O", "Expand or fold detail"),
        ("Esc", "Interrupt or close"),
        ("? on an empty draft", "Show shortcuts"),
        ("Shift+Enter or Alt+Enter", "Insert a newline"),
        ("\\ then Enter", "Replace the backslash with a newline"),
        (
            "Up / Down",
            "Move between draft lines; browse history and delegated agents from the first or last line",
        ),
        (
            "Home / End",
            "Go to draft start or end; when empty, go to oldest or newest output",
        ),
        ("Ctrl+A / Ctrl+E", "Go to line start or end"),
        ("Alt+B / Alt+F", "Move one word left or right"),
        ("Ctrl+W", "Delete the word to the left"),
        ("Ctrl+U", "Delete to line start"),
        ("Ctrl+K", "Delete to line end"),
    ] {
        assert!(
            report
                .keys
                .iter()
                .any(|row| row.key == key && row.description == action)
        );
    }
}

#[test]
fn informational_diagnostics_usage_wraps_at_words_with_a_hanging_indent() {
    use smith_client::diagnostics_report::{DiagnosticsReport, DiagnosticsRow, DiagnosticsSection};

    let usage = "input 0 · cached 0 · writes 0 · output 0 · reasoning 0";
    let mut app = App::new("model", "~/project");
    app.transcript
        .push_local(LocalResult::Diagnostics(Box::new(DiagnosticsReport {
            sections: vec![DiagnosticsSection {
                heading: "Cache".to_owned(),
                rows: vec![
                    DiagnosticsRow::Field {
                        label: "synthetic attempts".to_owned(),
                        value: "1".to_owned(),
                    },
                    DiagnosticsRow::Field {
                        label: "  usage".to_owned(),
                        value: usage.to_owned(),
                    },
                ],
            }],
        })));
    for width in [44, 100] {
        let lines = transcript_lines(&app, Theme::new().without_color(), width);
        let start = lines
            .iter()
            .position(|line| line.to_string().starts_with("    usage"))
            .unwrap();
        let column = informational_value_column(&lines[start]);
        let mut words = Vec::new();
        for (index, row) in lines[start..].iter().enumerate() {
            let text = row.to_string();
            if index > 0 {
                assert!(text.starts_with(&" ".repeat(column)), "{text}");
            }
            words.extend(
                text.chars()
                    .skip(column)
                    .collect::<String>()
                    .split_whitespace()
                    .map(str::to_owned),
            );
        }
        assert_eq!(words, usage.split_whitespace().collect::<Vec<_>>());
        assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
        if width == 44 {
            assert!(lines.len() > start + 1, "the usage should wrap");
        }
    }
}

#[test]
fn informational_diagnostics_paths_shorten_from_the_left_on_one_row() {
    use smith_client::diagnostics_report::{DiagnosticsReport, DiagnosticsRow, DiagnosticsSection};

    let path = "/private/tmp/claude-501/-Volumes-Data-codes-ai-tui/f77870a1-b4e2-491f-8fe4-e811c2bf7241/tui-grammar";
    for label in ["project", "workspace"] {
        let mut app = App::new("model", "~/project");
        app.show_local_report(LocalResult::Diagnostics(Box::new(DiagnosticsReport {
            sections: vec![DiagnosticsSection {
                heading: "Session".to_owned(),
                rows: vec![
                    DiagnosticsRow::Path {
                        label: label.to_owned(),
                        value: path.to_owned(),
                    },
                    DiagnosticsRow::Field {
                        label: "reasoning controls".to_owned(),
                        value: "supported".to_owned(),
                    },
                ],
            }],
        })));
        let lines = transcript_lines(&app, Theme::new().without_color(), 44);
        let text = informational_text(&lines);
        assert_eq!(text.len(), 4, "the path must stay on one row: {text:?}");
        assert_eq!(text[0], "● /diagnostics");
        let column = informational_value_column(&lines[2]);
        let value = text[2].chars().skip(column).collect::<String>();
        assert!(value.starts_with('…'), "{value}");
        assert!(value.ends_with("/tui-grammar"), "{value}");
        assert!(lines.iter().all(|line| line.width() <= 44), "{text:?}");
    }
}

#[test]
fn informational_long_results_open_at_the_top_and_next_blocks_resume_following() {
    use smith_client::diagnostics_report::{DiagnosticsReport, DiagnosticsRow, DiagnosticsSection};

    for (width, height) in [(44, 16), (100, 32)] {
        for command in ["help", "status", "context", "diagnostics"] {
            let theme = Theme::new().without_color();
            let mut app = App::new("model", "~/project");
            app.transcript
                .push_user("Earlier conversation ".repeat(100));
            app.transcript
                .push_reasoning_delta("Hidden reasoning", false);
            render_synced(&mut app, width, height, theme);
            let result = match command {
                "help" => LocalResult::Help(Box::new(crate::commands::help())),
                "status" => {
                    let mut report = status_report();
                    report.reasoning_controls = INFORMATIONAL_CONTROLS.repeat(12);
                    LocalResult::Status(Box::new(report))
                }
                "context" => {
                    let mut report = informational_context_report();
                    report.reasoning_controls = INFORMATIONAL_CONTROLS.repeat(12);
                    LocalResult::Context(Box::new(report))
                }
                _ => LocalResult::Diagnostics(Box::new(DiagnosticsReport {
                    sections: vec![
                        DiagnosticsSection {
                            heading: "Session".to_owned(),
                            rows: vec![
                                DiagnosticsRow::Field {
                                    label: "reasoning controls".to_owned(),
                                    value: INFORMATIONAL_CONTROLS.repeat(12),
                                },
                                DiagnosticsRow::Field {
                                    label: "provider controls".to_owned(),
                                    value: INFORMATIONAL_CONTROLS.repeat(12),
                                },
                            ],
                        },
                        DiagnosticsSection {
                            heading: "Runtime".to_owned(),
                            rows: vec![
                                DiagnosticsRow::Field {
                                    label: "tool controls".to_owned(),
                                    value: INFORMATIONAL_CONTROLS.repeat(12),
                                },
                                DiagnosticsRow::Field {
                                    label: "cache controls".to_owned(),
                                    value: INFORMATIONAL_CONTROLS.repeat(12),
                                },
                            ],
                        },
                    ],
                })),
            };
            app.show_local_report(result);
            let heading = format!("● /{command}");
            let preview = render(&app, width, height, theme);
            assert_eq!(preview.lines().next(), Some(heading.as_str()), "{preview}");
            let screen = render_synced(&mut app, width, height, theme);
            assert_eq!(screen.lines().next(), Some(heading.as_str()), "{screen}");
            assert!(!app.following);
            assert!(app.scroll_to_block.is_none());
            app.on_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
            let scrolled = render_synced(&mut app, width, height, theme);
            assert_ne!(screen, scrolled);
            assert!(!app.following);
            app.transcript
                .push_notice(NoticeKind::Monitor, "Fresh appended block");
            let preview = render(&app, width, height, theme);
            assert!(preview.contains("Fresh appended block"), "{preview}");
            let screen = render_synced(&mut app, width, height, theme);
            assert!(screen.contains("Fresh appended block"), "{screen}");
            assert!(app.following);
            assert!(app.result_scroll_revision.is_none());
        }
    }
}

#[test]
fn informational_no_colour_retains_the_same_text_columns_and_headings() {
    for width in [44, 100] {
        let help = crate::commands::help();
        let status = status_report();
        let context = informational_context_report();
        for (colored, plain) in [
            (
                render_help_report(&help, width, Theme::new()),
                render_help_report(&help, width, Theme::new().without_color()),
            ),
            (
                render_status_card(&status, width, Theme::new()),
                render_status_card(&status, width, Theme::new().without_color()),
            ),
            (
                render_context_report(&context, width, Theme::new()),
                render_context_report(&context, width, Theme::new().without_color()),
            ),
        ] {
            assert_eq!(informational_text(&colored), informational_text(&plain));
            assert!(
                plain
                    .iter()
                    .flat_map(|line| &line.spans)
                    .all(|span| span.style.fg.is_none() && span.style.bg.is_none())
            );
        }
        let help = render_help_report(&help, width, Theme::new().without_color());
        assert!(
            help[0]
                .spans
                .last()
                .unwrap()
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }
}

#[test]
fn informational_typed_values_keep_literal_inline_markup_and_explicit_elision() {
    let mut report = status_report();
    report.reasoning = "fixed: `off` **quiet**".to_owned();
    report.session = "x".repeat(80);
    for width in [44, 100] {
        let lines = render_status_card(&report, width, Theme::new());
        let text = informational_text(&lines).join("\n");
        assert!(text.contains("fixed: `off` **quiet**"), "{text}");
        let session = lines
            .iter()
            .find(|line| line.spans.iter().any(|span| span.content == "session"))
            .unwrap();
        assert!(session.spans.last().unwrap().content.ends_with('…'));
        assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
    }
}
