// Assistant Markdown uses the same path for attempts, commits, and replay.

const MARKDOWN_CASES: &[(&str, &str)] = &[
    (
        "headings",
        "# First heading\n## Second heading\n### Third heading\n#### Fourth heading\n##### Fifth heading\n###### Sixth heading",
    ),
    (
        "bold",
        "Use **care when retrying the request and cancelling ongoing work** and __bounded backoff__.",
    ),
    (
        "nested emphasis",
        "**bold *italic*** and *italic **bold*** and ***both*** and **bold `*code*`**.",
    ),
    (
        "italic",
        "Use *care when retrying the request and cancelling ongoing work* and _bounded backoff_.",
    ),
    (
        "inline code",
        "Run `cargo test -p smith-tui` with ``a ` literal backtick`` and `**literal code**`.",
    ),
    (
        "fence",
        "```rust\nfn retry() {\n    let message = \"**literal** [docs](https://example.com)\";\n}\n```\nDone.",
    ),
    (
        "open fence",
        "```rust\nfn retry() {\n    let message = \"**literal** [docs](https://example.com)\";\n",
    ),
    (
        "literal fence",
        "```text\n> literal quote\n- literal item\n# literal heading\n| literal | table |\n```",
    ),
    (
        "tilde fence",
        "~~~~text\n~~~ is code, not a closing fence\n~~~~",
    ),
    (
        "unordered lists",
        "- The retry policy keeps cancellation responsive while the provider repeatedly fails.\n  Continuation of the same item with **care**.\n  + A nested item with text long enough to wrap in the narrow terminal pane.\n    * A third level.\n- Next item.",
    ),
    (
        "ordered lists",
        "12. The retry policy keeps cancellation responsive while the provider repeatedly fails.\n    Continuation of the same item.\n    1. A nested item with text long enough to wrap in the narrow terminal pane.\n13. Next item.",
    ),
    (
        "lazy list continuation",
        "- The first item.\nContinuation without source indentation that is still part of the item's paragraph.\n\nA separate paragraph.",
    ),
    (
        "quotes",
        "> The retry policy keeps cancellation responsive while the provider repeatedly fails.\n>\n> > A nested quote with text long enough to wrap in the narrow terminal pane.\n> - A quoted list item with text long enough to wrap in the narrow terminal pane.",
    ),
    (
        "quoted fence",
        "> ```rust\n>     let value = \"**literal**\";\n> ```",
    ),
    (
        "list fence",
        "- ```rust\n      let value = \"**literal**\";\n  ```",
    ),
    (
        "table",
        "| Name | State |\n| :--- | ---: |\n| Retry | **ready** |\n| Cancel | waiting |",
    ),
    (
        "wide table",
        "| Name | Detail |\n| --- | --- |\n| Retry | Cancellation stays responsive during repeated provider failures |\n| Cancel | No work remains |",
    ),
    (
        "quoted table",
        "> | Name | State |\n> | --- | --- |\n> | Retry | ready |",
    ),
    (
        "long table headers",
        "| A heading longer than the entire narrow terminal pane can hold on a single line | State |\n| --- | --- |\n| Retry | ready |",
    ),
    (
        "table without records",
        "| A heading longer than the entire narrow terminal pane can hold on a single line | State |\n| --- | --- |",
    ),
    (
        "table pipes",
        "| Expression | Value |\n| --- | --- |\n| `a | b` | left \\| right |",
    ),
    ("horizontal rules", "Before.\n---\n***\n_ _ _\nAfter."),
    (
        "links",
        "See [the docs](https://example.com/docs) and [https://example.com](https://example.com).",
    ),
    (
        "long link",
        "See [the docs](https://example.com/one/very/long/path/that/does/not/have/any/spaces/and/must/remain/selectable).",
    ),
    (
        "balanced link",
        "See [**the docs**](https://example.com/docs_(retry)) and [unfinished](https://example.com",
    ),
    (
        "unclosed emphasis",
        "**unfinished bold\n*unfinished italic\n__unfinished bold\n_unfinished italic\n`unfinished code",
    ),
    (
        "literal math",
        "2 * 3 * 4\n** spaced**\n**trailing **\n\\*literal stars\\*",
    ),
    (
        "wide glyphs",
        "- 一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十\n> 一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十",
    ),
];

fn markdown_rows(text: &str, width: u16, theme: Theme) -> Vec<Line<'static>> {
    super::super::markdown::render_assistant_lines(text, theme, width, false)
}

#[test]
fn streaming_trailing_table_keeps_its_header_and_earlier_rows_stable() {
    let prefix = "Earlier **paragraph**.\n\n| Name | State |\n| --- | --- |";
    for width in [44, 80, 100] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            let receiving =
                super::super::markdown::render_assistant_lines(prefix, theme, width, true);
            assert_eq!(
                receiving
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
                [
                    "● Earlier paragraph.",
                    "  ",
                    "  Name │ State",
                    "  receiving table…"
                ]
            );
            let placeholder = receiving.last().unwrap();
            assert!(
                placeholder
                    .spans
                    .iter()
                    .all(|span| span.style.add_modifier.contains(Modifier::DIM))
            );
            for appended in [
                "\n| Retry | ready |",
                "\n| Retry | ready |\n|",
                "\n| Retry | ready |\n|   ",
                "\n| Retry | ready |\n| Later row is much wider than the headers | waiting |",
                "\n| Retry | ready |\n| Later row is much wider than the headers | waiting |\n| Partial",
            ] {
                let text = format!("{prefix}{appended}");
                let rows =
                    super::super::markdown::render_assistant_lines(&text, theme, width, true);
                assert_eq!(rows, receiving, "rows arriving at {width}: {text}");
                assert!(rows.iter().all(|row| row.width() <= usize::from(width)));
            }
            let complete = format!("{prefix}\n| Retry | ready |\n| Cancel | waiting |");
            let committed = markdown_rows(&complete, width, theme);
            assert_eq!(committed.len(), 6);
            assert!(
                committed
                    .iter()
                    .any(|row| row.to_string().contains("Retry"))
            );
            assert!(
                committed
                    .iter()
                    .any(|row| row.to_string().contains("Cancel"))
            );
            assert!(
                !committed
                    .iter()
                    .any(|row| row.to_string().contains("receiving table"))
            );
            let followed = format!("{complete}\n\nFollowing paragraph.");
            assert_eq!(
                super::super::markdown::render_assistant_lines(&followed, theme, width, true),
                markdown_rows(&followed, width, theme),
            );
        }
    }
}

#[test]
fn streaming_only_defers_the_trailing_table_and_preserves_its_container() {
    let complete = "| Name | State |\n| --- | --- |\n| Retry | ready |\n\nFollowing paragraph.\n\n";
    let trailing = "> | Child | Status |\n> | --- | --- |\n> | child-1 | running |";
    for width in [44, 80, 100] {
        let theme = Theme::new();
        let rows = super::super::markdown::render_assistant_lines(
            &format!("{complete}{trailing}"),
            theme,
            width,
            true,
        );
        assert!(rows.starts_with(&markdown_rows(complete, width, theme)));
        let text = rows.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(
            text[text.len() - 2..],
            ["  │ Child │ Status", "  │ receiving table…"]
        );
        assert!(text.iter().any(|row| row.contains("Retry")));
        assert!(!text.iter().any(|row| row.contains("child-1")));
    }
}

#[test]
fn assistant_markdown_fences_show_only_dim_labels_and_indented_code() {
    let cases: &[(&str, &[&str])] = &[
        (
            "``` python\n  **literal**\n```\nDone.",
            &["● python", "      **literal**", "  Done."],
        ),
        (
            "``` python\n  **literal**\n# still code",
            &["● python", "      **literal**", "    # still code"],
        ),
        (
            "```\n  **literal**\n```\nDone.",
            &["●     **literal**", "  Done."],
        ),
        (
            "```\n  **literal**\n# still code",
            &["●     **literal**", "    # still code"],
        ),
        (
            "~~~python\n**literal**\n~~~",
            &["● python", "    **literal**"],
        ),
        ("~~~\n**literal**\n~~~", &["●   **literal**"]),
        ("```\n```", &[]),
    ];
    for width in [44, 100] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            for (source, expected) in cases {
                let rows = markdown_rows(source, width, theme);
                assert_eq!(
                    rows.iter().map(ToString::to_string).collect::<Vec<_>>(),
                    *expected,
                    "{source:?} at {width}"
                );
                for span in rows.iter().flat_map(|row| &row.spans) {
                    if span.content == "python" {
                        assert!(span.style.add_modifier.contains(Modifier::DIM));
                    }
                    if span.content.contains("**literal**") {
                        assert!(!span
                            .style
                            .add_modifier
                            .intersects(Modifier::BOLD | Modifier::ITALIC));
                    }
                }
                assert!(rows.iter().all(|row| row.width() <= usize::from(width)));
            }
        }
    }
}

#[test]
fn assistant_markdown_constructs_have_visible_structure_without_color() {
    let cases: &[(&str, &[&str])] = &[
        ("# Heading\n## Subheading", &["● Heading", "  Subheading"]),
        (
            "**bold** and *italic* and `code`",
            &["● bold and italic and code"],
        ),
        (
            "2 * 3 * 4\n**unfinished\n*unfinished\n`unfinished",
            &[
                "● 2 * 3 * 4",
                "  **unfinished",
                "  *unfinished",
                "  `unfinished",
            ],
        ),
        (
            "** spaced**\n**trailing **\n\\*literal\\*",
            &["● ** spaced**", "  **trailing **", "  *literal*"],
        ),
        (
            "```rust\nfn retry() {\n    work();\n}\n```",
            &["● rust", "    fn retry() {", "        work();", "    }"],
        ),
        (
            "```rust\n**literal**\n[docs](url)",
            &["● rust", "    **literal**", "    [docs](url)"],
        ),
        (
            "```text\n> literal\n- item\n# heading\n```",
            &[
                "● text",
                "    > literal",
                "    - item",
                "    # heading",
            ],
        ),
        (
            "**bold *italic*** and *italic **bold*** and ***both***",
            &["● bold italic and italic bold and both"],
        ),
        ("*a **b***", &["● a b"]),
        ("**a *b***", &["● a b"]),
        ("***a** b*", &["● a b"]),
        ("***a* b**", &["● a b"]),
        ("2 * 3 * 4", &["● 2 * 3 * 4"]),
        ("**x", &["● **x"]),
        ("~~~~text\n~~~\n~~~~", &["● text", "    ~~~"]),
        (
            "- one\n  + two\n    * three\n- four",
            &["● - one", "    - two", "      - three", "  - four"],
        ),
        (
            "12. one\n    1. two\n       three\n13. four",
            &["● 12. one", "      1. two", "         three", "  13. four"],
        ),
        (
            "> quote\n> > nested\n> - item",
            &["● │ quote", "  │ │ nested", "  │ - item"],
        ),
        (
            "> ```rust\n>     work();\n> ```",
            &["● │ rust", "  │       work();"],
        ),
        (
            "- ```rust\n      work();\n  ```",
            &["● - rust", "          work();"],
        ),
        (
            "[docs](https://example.com)",
            &["● docs (https://example.com)"],
        ),
        (
            "[https://example.com](https://example.com)",
            &["● https://example.com"],
        ),
        (
            "[**docs**](https://example.com/a_(b))",
            &["● docs (https://example.com/a_(b))"],
        ),
        (
            "| Name | State |\n| --- | --- |\n| Retry | ready |",
            &["● Name  │ State", "  ──────┼──────", "  Retry │ ready"],
        ),
        (
            "| Expr | Value |\n| --- | --- |\n| `a | b` | left \\| right |",
            &[
                "● Expr  │ Value",
                "  ──────┼─────────────",
                "  a | b │ left | right",
            ],
        ),
    ];
    for width in [44, 100] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            for (source, expected) in cases {
                let rows = markdown_rows(source, width, theme);
                assert_eq!(
                    rows.iter().map(ToString::to_string).collect::<Vec<_>>(),
                    *expected,
                    "{source:?} at {width}"
                );
                assert!(rows.iter().all(|line| line.width() <= usize::from(width)));
            }
            for rule in ["---", "***", "_ _ _"] {
                let rows = markdown_rows(rule, width, theme);
                assert_eq!(
                    rows[0].to_string(),
                    format!("● {}", "─".repeat(usize::from(width) - 2))
                );
                assert!(
                    rows[0]
                        .spans
                        .last()
                        .unwrap()
                        .style
                        .add_modifier
                        .contains(Modifier::DIM)
                );
            }
        }
    }
}

#[test]
fn assistant_markdown_styles_survive_wrapping_and_no_color() {
    for width in [44, 100] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            for (source, modifier) in [
                (
                    "**alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu**",
                    Modifier::BOLD,
                ),
                (
                    "*alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu*",
                    Modifier::ITALIC,
                ),
                (
                    "# alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu",
                    Modifier::BOLD | Modifier::UNDERLINED,
                ),
                (
                    "[alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu](https://example.com)",
                    Modifier::UNDERLINED,
                ),
                (
                    "`alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu`",
                    Modifier::DIM,
                ),
            ] {
                let rows = markdown_rows(source, width, theme);
                for span in rows.iter().flat_map(|row| &row.spans).filter(|span| {
                    span.content.contains("alpha") || span.content.contains("lambda")
                }) {
                    assert!(
                        span.style.add_modifier.contains(modifier),
                        "{source} at {width}: {span:#?}"
                    );
                }
            }
            for (source, modifiers) in [
                (
                    "*a **b***",
                    [Modifier::ITALIC, Modifier::BOLD | Modifier::ITALIC],
                ),
                (
                    "**a *b***",
                    [Modifier::BOLD, Modifier::BOLD | Modifier::ITALIC],
                ),
                (
                    "***a** b*",
                    [Modifier::BOLD | Modifier::ITALIC, Modifier::ITALIC],
                ),
                (
                    "***a* b**",
                    [Modifier::BOLD | Modifier::ITALIC, Modifier::BOLD],
                ),
            ] {
                let rows = markdown_rows(source, width, theme);
                for (text, modifier) in ["a", "b"].into_iter().zip(modifiers) {
                    let span = rows[0]
                        .spans
                        .iter()
                        .find(|span| span.content.contains(text))
                        .unwrap();
                    assert_eq!(
                        span.style.add_modifier & (Modifier::BOLD | Modifier::ITALIC),
                        modifier,
                        "{source} at {width}: {span:#?}"
                    );
                }
            }
            let link = markdown_rows("[docs](https://example.com)", width, theme);
            let label = link[0]
                .spans
                .iter()
                .find(|span| span.content == "docs")
                .unwrap();
            assert!(label.style.add_modifier.contains(Modifier::UNDERLINED));
            let target = link[0]
                .spans
                .iter()
                .find(|span| span.content.contains("(https://example.com)"))
                .unwrap();
            assert!(target.style.add_modifier.contains(Modifier::DIM));
            assert!(!target.style.add_modifier.contains(Modifier::UNDERLINED));
            let literal = markdown_rows("2 * 3 * 4 and **unfinished", width, theme);
            assert!(
                literal[0]
                    .spans
                    .iter()
                    .filter(|span| span.content.contains('*'))
                    .all(|span| !span
                        .style
                        .add_modifier
                        .intersects(Modifier::BOLD | Modifier::ITALIC))
            );
        }
    }
}

#[test]
fn assistant_markdown_wrapping_keeps_list_indents_and_quote_bars() {
    let words = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega";
    for width in [44, 100] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            for (source, first, continuation) in [
                (format!("- {words}"), "● - ", "    "),
                (format!("12. {words}"), "● 12. ", "      "),
                (format!("    - {words}"), "●     - ", "        "),
                (format!("> {words}"), "● │ ", "  │ "),
                (format!("> > {words}"), "● │ │ ", "  │ │ "),
                (format!("> - {words}"), "● │ - ", "  │   "),
                (format!("- > {words}"), "● - │ ", "    │ "),
            ] {
                let rows = markdown_rows(&source, width, theme);
                assert!(rows.len() > 1, "the example must wrap at {width}");
                assert!(rows[0].to_string().starts_with(first));
                let mut parts = vec![rows[0].to_string()[first.len()..].to_owned()];
                for row in &rows[1..] {
                    let text = row.to_string();
                    assert!(
                        text.starts_with(continuation),
                        "{source} at {width}: {text}"
                    );
                    assert!(row.width() <= usize::from(width));
                    parts.push(text[continuation.len()..].to_owned());
                }
                assert_eq!(parts.join(" "), words);
            }
            let chinese = "一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十";
            let rows = markdown_rows(&format!("- {chinese}"), width, theme);
            let body = rows
                .iter()
                .map(|row| row.to_string().chars().skip(4).collect::<String>())
                .collect::<String>();
            assert_eq!(body, chinese, "wide glyphs at {width}");
            assert!(rows.iter().all(|row| row.width() <= usize::from(width)));
        }
    }
}

#[test]
fn assistant_markdown_tables_stack_only_when_the_columns_do_not_fit() {
    let source = "| Name | Detail |\n| --- | --- |\n| Retry | Cancellation stays responsive during repeated provider failures |\n| Cancel | No work remains |";
    for theme in [Theme::new(), Theme::new().without_color()] {
        let narrow = markdown_rows(source, 44, theme);
        let text = narrow.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(text[0], "● Name: Retry");
        assert!(text[1].starts_with("  Detail: Cancellation"));
        assert!(
            text[2].starts_with("          "),
            "values hang under the text: {text:#?}"
        );
        assert!(text.iter().any(|line| line == "  Name: Cancel"));
        assert!(text.iter().any(|line| line == "  Detail: No work remains"));
        assert!(!text.concat().contains('│'));
        assert!(narrow.iter().all(|row| row.width() <= 44));
        let wide = markdown_rows(source, 100, theme);
        assert_eq!(wide.len(), 4);
        assert!(wide[0].to_string().starts_with("● Name   │ Detail"));
        assert!(
            wide[2]
                .to_string()
                .contains("Cancellation stays responsive during repeated provider failures")
        );
        assert!(wide.iter().all(|row| row.width() <= 100));
    }
}

#[test]
fn assistant_markdown_links_keep_the_whole_target_when_wrapped() {
    let target = "https://example.com/a/very/long/path/without/any/spaces/that/stays/selectable/at/every/width";
    for width in [44, 100] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            let rows = markdown_rows(&format!("[docs]({target})"), width, theme);
            let body = rows
                .iter()
                .map(|row| row.to_string().chars().skip(2).collect::<String>())
                .collect::<String>();
            assert!(body.contains(target), "{body}");
            assert!(rows.iter().all(|row| row.width() <= usize::from(width)));
        }
    }
}

#[test]
fn assistant_markdown_bounds_deep_indentation_without_losing_wide_text() {
    for width in [44, 100] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            let rows = markdown_rows(&format!("{}- 一二三四五", " ".repeat(120)), width, theme);
            assert!(rows.iter().all(|row| row.width() <= usize::from(width)));
            let text = rows.iter().map(ToString::to_string).collect::<String>();
            for character in "一二三四五".chars() {
                assert_eq!(text.matches(character).count(), 1);
            }
        }
    }
}

#[test]
fn assistant_markdown_renders_every_partial_construct_through_the_commit_path() {
    for (name, source) in MARKDOWN_CASES {
        let mut app = App::new("m", "p");
        app.apply(&event(RuntimeEvent::TurnStarted));
        let mut buffer = String::new();
        for character in source.chars() {
            buffer.push(character);
            app.apply(&event(RuntimeEvent::TextDelta {
                request: RequestId::new("r"),
                attempt: AttemptId::new("a"),
                text: character.to_string(),
            }));
            let mut committed = App::new("m", "p");
            committed.transcript.push_text_delta(&buffer);
            for width in [44, 100] {
                for theme in [Theme::new(), Theme::new().without_color()] {
                    let live = transcript_lines(&app, theme, width);
                    assert_eq!(
                        live,
                        transcript_lines(&committed, theme, width),
                        "partial {name} at {width}: {buffer:?}"
                    );
                    assert!(
                        live.iter().all(|row| row.width() <= usize::from(width)),
                        "partial {name}: {live:#?}"
                    );
                }
            }
        }
    }
}

#[test]
fn assistant_markdown_appends_after_hard_newlines_without_rewrapping_completed_prose() {
    let cases = [
        (
            "# Heading\nCompleted **bold** paragraph.\n",
            "Next paragraph.",
        ),
        ("**unfinished\n", "bold**"),
        (
            "- A completed item with enough text to wrap at forty-four columns and keep its indentation.\n",
            "- Next item.",
        ),
        (
            "> A completed quote with enough text to wrap at forty-four columns and keep its bar.\n",
            "> Next line.",
        ),
        ("```rust\n    let value = \"**literal**\";\n", "```\nDone."),
    ];
    for width in [44, 100] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            for (prefix, appended) in cases {
                let before = markdown_rows(prefix, width, theme);
                let after = markdown_rows(&format!("{prefix}{appended}"), width, theme);
                assert!(
                    after.starts_with(&before),
                    "completed lines at {width}: {prefix:?}"
                );
            }
        }
    }
}

#[test]
fn assistant_markdown_matches_stream_commit_journal_and_history_at_each_width_and_theme() {
    for (name, text) in MARKDOWN_CASES {
        // These equality checks cover completed constructs. Trailing tables
        // intentionally differ while streaming and have dedicated tests.
        let completed = format!("{text}\n\nAfter the table.");
        let text = if name.contains("table") {
            completed.as_str()
        } else {
            *text
        };
        let events = [
            event(RuntimeEvent::TurnStarted),
            event(RuntimeEvent::TextDelta {
                request: RequestId::new("markdown-request"),
                attempt: AttemptId::new("markdown-attempt"),
                text: text.to_owned(),
            }),
            event(RuntimeEvent::ProviderAttemptOutputCommitted {
                request: RequestId::new("markdown-request"),
                attempt: AttemptId::new("markdown-attempt"),
            }),
        ];
        let mut streamed = App::new("m", "p");
        for event in &events[..2] {
            streamed.apply(event);
        }
        let mut committed = App::new("m", "p");
        for event in &events {
            committed.apply(event);
        }
        let serialized = serde_json::to_vec(&events).expect("journal events");
        let recovered: Vec<EventEnvelope> =
            serde_json::from_slice(&serialized).expect("replayable journal");
        let mut journal = App::new("m", "p");
        for event in &recovered {
            journal.apply_recovered(event);
        }
        let mut history = App::new("m", "p");
        history
            .transcript
            .replace_from_history(&[Message::assistant(vec![ContentPart::Text {
                text: text.to_owned(),
            }])]);
        for width in [44, 80, 100] {
            let mut colored_text = None;
            for theme in [Theme::new(), Theme::new().without_color()] {
                let live = transcript_lines(&streamed, theme, width);
                assert_eq!(
                    live,
                    transcript_lines(&committed, theme, width),
                    "{name}: commit at {width}"
                );
                assert_eq!(
                    live,
                    transcript_lines(&journal, theme, width),
                    "{name}: journal at {width}"
                );
                assert_eq!(
                    live,
                    transcript_lines(&history, theme, width),
                    "{name}: history at {width}"
                );
                assert!(
                    live.iter().all(|line| line.width() <= usize::from(width)),
                    "{name} at {width}: {live:#?}"
                );
                assert_eq!(
                    live.iter()
                        .filter(|line| line.to_string().starts_with("● "))
                        .count(),
                    1,
                    "{name}: one role marker"
                );
                for span in live.iter().flat_map(|line| &line.spans) {
                    assert_eq!(span.style.bg, None, "{name}: no background fill");
                    assert!(
                        !span.content.contains('\u{1b}'),
                        "{name}: no OSC without detection"
                    );
                    if !theme.uses_color() {
                        assert_eq!(span.style.fg, None, "{name}: no hue");
                    } else {
                        assert!(
                            matches!(
                                span.style.fg,
                                None | Some(
                                    Color::Black
                                        | Color::Red
                                        | Color::Green
                                        | Color::Yellow
                                        | Color::Blue
                                        | Color::Magenta
                                        | Color::Cyan
                                        | Color::Gray
                                        | Color::DarkGray
                                        | Color::LightRed
                                        | Color::LightGreen
                                        | Color::LightYellow
                                        | Color::LightBlue
                                        | Color::LightMagenta
                                        | Color::LightCyan
                                        | Color::White
                                )
                            ),
                            "{name}: named ANSI colours only"
                        );
                    }
                }
                let text = live.iter().map(ToString::to_string).collect::<Vec<_>>();
                if let Some(colored) = &colored_text {
                    assert_eq!(colored, &text, "{name}: monochrome keeps structure");
                } else {
                    colored_text = Some(text);
                }
                assert_eq!(
                    render(&streamed, width, 64, theme),
                    render(&committed, width, 64, theme),
                    "{name}: drawn attempt and commit at {width}"
                );
            }
        }
    }
}

#[test]
fn child_markdown_uses_the_same_renderer_before_and_after_commit() {
    for (name, text) in MARKDOWN_CASES {
        let completed = format!("{text}\n\nAfter the table.");
        let text = if name.contains("table") {
            completed.as_str()
        } else {
            *text
        };
        let mut app = App::new("m", "p");
        app.apply_child(
            "child-a",
            &event(RuntimeEvent::TextDelta {
                request: RequestId::new("child-request"),
                attempt: AttemptId::new("child-attempt"),
                text: text.to_owned(),
            }),
        );
        app.inspect_child("child-a");
        let before: Vec<_> = [44, 100]
            .into_iter()
            .flat_map(|width| {
                [Theme::new(), Theme::new().without_color()]
                    .into_iter()
                    .map(move |theme| (width, theme))
            })
            .map(|(width, theme)| (width, theme, transcript_lines(&app, theme, width)))
            .collect();
        app.apply_child(
            "child-a",
            &event(RuntimeEvent::ProviderAttemptOutputCommitted {
                request: RequestId::new("child-request"),
                attempt: AttemptId::new("child-attempt"),
            }),
        );
        for (width, theme, live) in before {
            assert_eq!(
                live,
                transcript_lines(&app, theme, width),
                "{name}: child commit at {width}"
            );
            let root = super::super::markdown::render_assistant_lines(text, theme, width, true);
            assert!(live.ends_with(&root), "{name}: child shares root Markdown");
        }
    }
}

#[test]
fn assistant_markdown_commit_keeps_an_existing_open_blocks_boundary() {
    for width in [44, 100] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            let mut app = App::new("m", "p");
            app.transcript.push_text_delta("# Heading\n**first");
            app.apply(&event(RuntimeEvent::TextDelta {
                request: RequestId::new("r"),
                attempt: AttemptId::new("a"),
                text: " and last**".to_owned(),
            }));
            let live = transcript_lines(&app, theme, width);
            assert_eq!(
                live.iter().map(ToString::to_string).collect::<Vec<_>>(),
                ["● Heading", "  first and last"]
            );
            app.apply(&event(RuntimeEvent::ProviderAttemptOutputCommitted {
                request: RequestId::new("r"),
                attempt: AttemptId::new("a"),
            }));
            assert_eq!(live, transcript_lines(&app, theme, width));
        }
    }
}

#[test]
fn discarded_markdown_leaves_no_attempt_output() {
    let mut app = App::new("m", "p");
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(RuntimeEvent::TextDelta {
        request: RequestId::new("r"),
        attempt: AttemptId::new("a"),
        text: "```rust\n**not a committed answer**".to_owned(),
    }));
    app.apply(&event(RuntimeEvent::ProviderAttemptOutputDiscarded {
        request: RequestId::new("r"),
        attempt: AttemptId::new("a"),
    }));
    for width in [44, 100] {
        let rows = transcript_lines(&app, Theme::new(), width);
        let text = rows
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("discarded speculative output"));
        assert!(!text.contains("not a committed answer"));
        assert!(!text.contains("```"));
    }
}
