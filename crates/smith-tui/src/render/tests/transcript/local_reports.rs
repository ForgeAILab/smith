use super::*;

#[test]
fn timeline_child_snapshots_render_labelled_turn_counts_once() {
    use smith_client::agent_report::turns_label;
    use smith_client::timeline_report::{TimelineEntry, TimelineReport, render_plain};

    for (used, maximum, expected_turns) in [
        (1, None, "1 turn"),
        (2, None, "2 turns"),
        (3, Some(5), "3/5 turns"),
    ] {
        let report = TimelineReport::Entries(vec![TimelineEntry::ChildSnapshot {
            child: "child-1".to_owned(),
            session: "session-1".to_owned(),
            durability: "durable".to_owned(),
            state: "idle".to_owned(),
            resumable: false,
            turns: turns_label(used, maximum),
        }]);
        let expected = format!(
            "child child-1 · session session-1 · durable · idle · no exact checkpoint · {expected_turns}"
        );
        assert_eq!(render_plain(&report), expected);

        let mut app = App::new("model", "project");
        app.show_local_report(LocalResult::Timeline(Box::new(report)));
        let rendered = transcript_lines(&app, Theme::new().without_color(), 240)
            .iter()
            .skip(1)
            .map(|row| row.to_string().trim_start().to_owned())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(rendered, expected);
    }
}

#[test]
fn local_results_render_inline_across_supported_sizes() {
    use smith_client::diff_report::{DiffLine, DiffLineKind, DiffOutcome, DiffReport};

    let mut app = App::new("gpt-5.3", "~/work/api");
    app.show_local_report(LocalResult::Diff(Box::new(DiffReport {
        title: "diff · all uncommitted".to_owned(),
        outcome: DiffOutcome::Patch(vec![DiffLine {
            kind: DiffLineKind::Context,
            text: "No changes in this scope.\nBinary file exists; content omitted.".to_owned(),
        }]),
    })));
    assert!(app.overlay.is_none());
    for (width, height) in [(44, 12), (74, 20), (120, 30)] {
        let screen = render(&app, width, height, Theme::new().without_color());
        assert!(screen.contains("/diff · all uncommitted"), "{screen}");
        assert!(screen.contains("No changes"), "{screen}");
        assert!(screen.contains("Binary file"), "{screen}");
        assert!(screen.contains("> Ask Smith to do anything"), "{screen}");
    }
}

#[test]
fn typed_diff_kinds_choose_style_without_title_or_prefix_parsing() {
    use smith_client::diff_report::{DiffLine, DiffLineKind, DiffOutcome, DiffReport};

    let theme = Theme::new();
    for (kind, text, tone) in [
        (
            DiffLineKind::Addition,
            "-removal-looking content",
            Tone::Success,
        ),
        (
            DiffLineKind::Removal,
            "+addition-looking content",
            Tone::Danger,
        ),
        (DiffLineKind::Metadata, "plain file header", Tone::Dim),
        (DiffLineKind::Hunk, "plain hunk header", Tone::Code),
        (
            DiffLineKind::Context,
            "@@ hunk-looking content",
            Tone::Default,
        ),
    ] {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.show_local_report(LocalResult::Diff(Box::new(DiffReport {
            title: "arbitrary title".to_owned(),
            outcome: DiffOutcome::Patch(vec![DiffLine {
                kind,
                text: text.to_owned(),
            }]),
        })));
        let lines = transcript_lines(&app, theme, 8);
        assert_eq!(lines[0].to_string(), "/arbitrary title");
        let body = &lines[1..];
        assert_eq!(
            body.iter().map(ToString::to_string).collect::<String>(),
            text
        );
        assert!(body.len() > 1, "expected a wrapped line");
        assert!(body.iter().all(|line| {
            line.width() <= 8
                && line
                    .spans
                    .iter()
                    .all(|span| span.style == theme.style(tone))
        }));
    }
}

#[test]
fn typed_diff_empty_and_error_states_keep_their_markers_and_wrapping() {
    use smith_client::diff_report::{DiffOutcome, DiffReport};

    for (outcome, marker, message) in [
        (DiffOutcome::Empty, "●", DiffReport::EMPTY_MESSAGE),
        (
            DiffOutcome::Error("Git inspection is unavailable.".to_owned()),
            "■",
            "Git inspection is unavailable.",
        ),
    ] {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.show_local_report(LocalResult::Diff(Box::new(DiffReport {
            title: "diff".to_owned(),
            outcome,
        })));
        let lines = transcript_lines(&app, Theme::new().without_color(), 12);
        let body = &lines[1..];
        assert!(body[0].to_string().starts_with(&format!("{marker} ")));
        assert!(
            body.iter()
                .skip(1)
                .all(|line| line.to_string().starts_with("  "))
        );
        assert_eq!(
            body.iter()
                .map(|line| line.to_string().chars().skip(2).collect::<String>())
                .collect::<String>(),
            message,
        );
        assert!(body.iter().all(|line| line.width() <= 12));
    }
}

#[test]
fn typed_recovery_notices_and_errors_keep_the_previous_transcript_presentation() {
    use smith_client::recovery_report::{RecoveryAction, RecoveryApplied, RecoveryReport};

    for report in [
        RecoveryReport::PreviewError {
            action: RecoveryAction::Undo,
            message: "no Smith turn has attributable changes".to_owned(),
        },
        RecoveryReport::PreviewError {
            action: RecoveryAction::Redo,
            message: "no exact redo candidate exists".to_owned(),
        },
        RecoveryReport::PreviewError {
            action: RecoveryAction::Revert,
            message: "Git-backed change inspection is unavailable outside a Git worktree"
                .to_owned(),
        },
        RecoveryReport::RevertUsage,
        RecoveryReport::Applied(RecoveryApplied::Undo),
        RecoveryReport::Applied(RecoveryApplied::Redo),
        RecoveryReport::Applied(RecoveryApplied::Revert {
            scope: "path#1".to_owned(),
        }),
        RecoveryReport::ApplyError {
            action: RecoveryAction::Undo,
            message: "undo refused\nmore detail".to_owned(),
        },
        RecoveryReport::ApplyError {
            action: RecoveryAction::Redo,
            message: "redo refused\nmore detail".to_owned(),
        },
        RecoveryReport::ApplyError {
            action: RecoveryAction::Revert,
            message: "revert refused\nmore detail".to_owned(),
        },
        RecoveryReport::Cancelled(RecoveryAction::Undo),
        RecoveryReport::Cancelled(RecoveryAction::Redo),
        RecoveryReport::Cancelled(RecoveryAction::Revert),
    ] {
        let content = smith_client::recovery_report::render_plain(&report);
        let mut legacy = App::new("gpt-5.3", "~/work/api");
        match &report {
            RecoveryReport::PreviewError {
                action: RecoveryAction::Redo,
                ..
            } => {
                legacy.show_local_report(LocalResult::Message(Box::new(
                    smith_client::message_report::MessageReport::Error {
                        title: "redo".to_owned(),
                        message: content,
                    },
                )));
            }
            RecoveryReport::Applied(_) | RecoveryReport::Cancelled(_) => {
                legacy
                    .transcript
                    .push_notice(report.action().notice_kind(), content);
            }
            _ => legacy.transcript.push_error(content),
        }
        let mut typed = App::new("gpt-5.3", "~/work/api");
        typed
            .transcript
            .push_local(LocalResult::Recovery(Box::new(report)));
        for width in [44, 100] {
            for theme in [Theme::new(), Theme::new().without_color()] {
                assert_eq!(
                    transcript_lines(&typed, theme, width),
                    transcript_lines(&legacy, theme, width)
                );
                assert_eq!(
                    render(&typed, width, 24, theme),
                    render(&legacy, width, 24, theme)
                );
            }
        }
    }
}

#[test]
fn typed_recovery_previews_preserve_unstyled_source_and_ignore_display_prefixes() {
    use smith_client::diff_report::{DiffLine, DiffLineKind};
    use smith_client::recovery_report::{RevertOrigin, RevertPreview};

    let patch = vec![
        DiffLine {
            kind: DiffLineKind::Addition,
            text: "origin: unknown\r\n".to_owned(),
        },
        DiffLine {
            kind: DiffLineKind::Context,
            text: "\n".to_owned(),
        },
        DiffLine {
            kind: DiffLineKind::Metadata,
            text: "+source without a final newline".to_owned(),
        },
    ];
    assert_eq!(
        render_recovery_patch(&patch),
        vec![
            Line::from("origin: unknown"),
            Line::default(),
            Line::from("+source without a final newline"),
        ]
    );
    let report = RevertPreview {
        scope: "redo#1".to_owned(),
        fingerprint: "exact-preview".to_owned(),
        origin: RevertOrigin::Smith,
        patch,
    };
    assert_eq!(
        render_revert_preview(&report),
        vec![
            Line::from("origin: Smith"),
            Line::default(),
            Line::from("origin: unknown"),
            Line::default(),
            Line::from("+source without a final newline"),
        ]
    );
}

#[test]
fn typed_review_notices_and_errors_keep_the_previous_transcript_presentation() {
    use smith_client::review_report::{ReviewReport, ReviewStartReport};

    for report in [
        ReviewReport::Empty,
        ReviewReport::Error(
            "Git-backed change inspection is unavailable outside a Git worktree\nmore detail"
                .to_owned(),
        ),
        ReviewReport::Start(ReviewStartReport::Unavailable),
        ReviewReport::Start(ReviewStartReport::Started {
            child: "child-1".to_owned(),
        }),
        ReviewReport::Start(ReviewStartReport::Queued {
            child: "child-2".to_owned(),
        }),
        ReviewReport::Start(ReviewStartReport::AtCapacity {
            running: 2,
            limit: 2,
        }),
        ReviewReport::Start(ReviewStartReport::Failed("provider unavailable".to_owned())),
    ] {
        let content = smith_client::review_report::render_plain(&report);
        let mut legacy = App::new("gpt-5.3", "~/work/api");
        if matches!(
            &report,
            ReviewReport::Empty
                | ReviewReport::Start(
                    ReviewStartReport::Started { .. } | ReviewStartReport::Queued { .. }
                )
        ) {
            legacy.transcript.push_notice(NoticeKind::Review, content);
        } else {
            legacy.transcript.push_error(content);
        }
        let mut typed = App::new("gpt-5.3", "~/work/api");
        typed
            .transcript
            .push_local(LocalResult::Review(Box::new(report)));
        for width in [44, 100] {
            for theme in [Theme::new(), Theme::new().without_color()] {
                assert_eq!(
                    transcript_lines(&typed, theme, width),
                    transcript_lines(&legacy, theme, width),
                );
                assert_eq!(
                    render(&typed, width, 24, theme),
                    render(&legacy, width, 24, theme),
                );
            }
        }
    }
}

#[test]
fn typed_review_confirmation_does_not_recover_structure_from_source_text() {
    use smith_client::diff_report::{DiffLine, DiffLineKind};
    use smith_client::review_report::{ReviewPreview, ReviewReport};

    let lines = render_review_preview(&ReviewPreview {
        scope: "path#1".to_owned(),
        title: "an arbitrary inspection title".to_owned(),
        patch: vec![
            DiffLine {
                kind: DiffLineKind::Addition,
                text: "origin: unknown\r\n".to_owned(),
            },
            DiffLine {
                kind: DiffLineKind::Metadata,
                text: "+source with a misleading prefix".to_owned(),
            },
        ],
    });
    assert_eq!(
        lines,
        [
            "scope: an arbitrary inspection title",
            "provider-backed: yes",
            "workspace authority: read-only",
            ReviewReport::AUTHORITY_MESSAGE,
            "",
            "origin: unknown",
            "+source with a misleading prefix",
        ]
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>(),
    );
}

#[test]
fn wrapped_local_result_continuations_keep_the_content_indent() {
    let mut report = status_report();
    report.session = "a long session description that needs several lines".to_owned();
    let lines = render_status_card(&report, 44, Theme::new().without_color());
    let screen = lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        screen.lines().all(|line| line.starts_with("  ")),
        "a wrapped continuation escaped the local-result indent:\n{screen}"
    );
    assert!(
        screen
            .lines()
            .filter(|line| line.starts_with(&" ".repeat(22)))
            .count()
            >= 2,
        "{screen}"
    );
}

#[test]
fn typed_status_stays_bounded_across_supported_widths() {
    let mut app = App::new("glm-4.7", "~/work/api");
    let mut report = status_report();
    report.usage = "~98% input left (~1.1k used / 68.9k budget)".to_owned();
    app.show_local_report(LocalResult::Status(Box::new(report)));

    for width in [44, 74, 120] {
        let lines = transcript_lines(&app, Theme::new().without_color(), width);
        let screen = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("/status"), "{width} columns:\n{screen}");
        assert!(screen.contains("~98% input"), "{width} columns:\n{screen}");
        assert!(
            lines.iter().all(|line| line.width() <= usize::from(width)),
            "{width} columns overflowed:\n{screen}"
        );
    }
}

#[test]
fn typed_diagnostics_preserves_wrapping_and_inline_text() {
    use smith_client::diagnostics_report::{DiagnosticsReport, DiagnosticsRow, DiagnosticsSection};

    let report = DiagnosticsReport {
        sections: vec![
            DiagnosticsSection {
                heading: "Session".to_owned(),
                rows: vec![
                    DiagnosticsRow::Field {
                        label: "profile".to_owned(),
                        value: "dev".to_owned(),
                    },
                    DiagnosticsRow::Field {
                        label: "goal".to_owned(),
                        value: "Fix `tool:read` and `write`\nFollow **these steps**".to_owned(),
                    },
                ],
            },
            DiagnosticsSection {
                heading: "Context".to_owned(),
                rows: vec![
                    DiagnosticsRow::Field {
                        label: "context".to_owned(),
                        value: "one complete sentence with words that wrap cleanly".to_owned(),
                    },
                    DiagnosticsRow::Field {
                        label: "  tool schema".to_owned(),
                        value: "~500".to_owned(),
                    },
                    DiagnosticsRow::Line("**literal: text** and `inline code`".to_owned()),
                    DiagnosticsRow::Line("Free **text** with `inline code`".to_owned()),
                ],
            },
        ],
    };
    let mut app = App::new("example-model", "~/work/api");
    app.show_local_report(LocalResult::Diagnostics(Box::new(report)));
    for width in [44, 80, 100] {
        let theme = Theme::new();
        let typed = transcript_lines(&app, theme, width);
        let text = typed.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(text[0], "● /diagnostics");
        assert_eq!(text[1], "  Session");
        assert_eq!(text[2], "  profile        dev");
        assert_eq!(text[3], "  goal           Fix `tool:read` and `write`");
        assert_eq!(text[4], "                 Follow **these steps**");
        assert_eq!(text[5], "");
        assert_eq!(text[6], "  Context");
        assert!(
            text.iter().any(|line| line == "    tool schema  ~500"),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|line| line == "  literal: text and inline code"),
            "{text:?}"
        );
        assert!(
            text.iter()
                .any(|line| line == "  Free text with inline code"),
            "{text:?}"
        );
        if width == 44 {
            assert_eq!(text[7], "  context        one complete sentence with");
            assert_eq!(text[8], "                 words that wrap cleanly");
        } else {
            assert_eq!(
                text[7],
                "  context        one complete sentence with words that wrap cleanly"
            );
        }
        assert!(
            typed.iter().all(|line| line.width() <= usize::from(width)),
            "{text:?}"
        );
        assert!(
            typed[1]
                .spans
                .iter()
                .any(|span| span.style == theme.style(Tone::Heading))
        );
        assert!(
            typed[3]
                .spans
                .iter()
                .skip(3)
                .all(|span| span.style == theme.style(Tone::Default))
        );
        let free = typed
            .iter()
            .find(|line| line.to_string().contains("literal: text"))
            .unwrap();
        assert!(
            free.spans
                .iter()
                .any(|span| span.style.add_modifier.contains(Modifier::BOLD))
        );
    }
}

#[test]
fn shell_reports_keep_free_text_presentation_and_explicit_outcomes() {
    use crate::transcript::Block;
    use smith_client::local_result::LocalResultState;
    use smith_client::shell_report::ShellReport;

    let mut app = App::new("example-model", "~/work/api");
    app.show_local_report(LocalResult::Shell(Box::new(ShellReport::new(
        "session: `literal`\n**free text** with `code`\n@@ not a patch heading\n+not an addition",
        false,
    ))));
    let theme = Theme::new();
    let lines = transcript_lines(&app, theme, 100);
    assert_eq!(
        lines.iter().map(ToString::to_string).collect::<Vec<_>>(),
        [
            "/shell",
            "session: literal",
            "free text with code",
            "@@ not a patch heading",
            "+not an addition",
        ],
    );
    assert!(
        lines[1]
            .spans
            .iter()
            .any(|span| span.style == theme.style(Tone::Code))
    );
    assert!(
        lines[4]
            .spans
            .iter()
            .all(|span| span.style == theme.style(Tone::Default))
    );

    for (output, is_error, marker, expected, state) in [
        (
            "failure: `literal`",
            true,
            "■",
            "failure: `literal`",
            LocalResultState::Error,
        ),
        (" \n", false, "●", "No output.", LocalResultState::Empty),
        (" \n", true, "●", "No output.", LocalResultState::Empty),
    ] {
        let mut app = App::new("example-model", "~/work/api");
        app.show_local_report(LocalResult::Shell(Box::new(ShellReport::new(
            output, is_error,
        ))));
        let Block::Local(result) = &app.transcript.blocks()[0] else {
            panic!("expected a shell report");
        };
        assert_eq!(result.state(), state);
        let lines = transcript_lines(&app, theme, 100);
        assert_eq!(lines[0].to_string(), "/shell");
        assert_eq!(lines[1].to_string(), format!("{marker} {expected}"));
    }
}

#[test]
fn message_reports_do_not_select_the_status_card_by_title() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.show_local_report(LocalResult::Message(Box::new(
        smith_client::message_report::MessageReport::Notice {
            title: "status".to_owned(),
            message: "session: text stays text".to_owned(),
        },
    )));
    let screen = render(&app, 74, 24, Theme::new().without_color());
    assert!(screen.contains("session: text stays text"), "{screen}");
    assert!(!screen.contains('╭'), "{screen}");
}

#[test]
fn focused_context_view_keeps_the_grid_and_legend_inline() {
    let mut app = App::new("glm-4.7", "~/work/api");
    use smith_client::context_report::{
        ContextCapacity, ContextCategory, ContextCategoryKind, ContextCompaction, ContextReport,
        ContextUsage,
    };
    app.show_local_report(LocalResult::Context(Box::new(ContextReport {
        available_windows: Vec::new(),
        summary: "glm-4.7 · ~2k / 123.9k input tokens · ~98% left".to_owned(),
        usage: ContextUsage::Estimated,
        categories: vec![
            ContextCategory {
                kind: ContextCategoryKind::System,
                label: "system instructions".to_owned(),
                tokens: 200,
                value: "~200 (0.1%)".to_owned(),
            },
            ContextCategory {
                kind: ContextCategoryKind::Tool,
                label: "tool schemas".to_owned(),
                tokens: 500,
                value: "~500 (0.4%)".to_owned(),
            },
            ContextCategory {
                kind: ContextCategoryKind::History,
                label: "history".to_owned(),
                tokens: 1_300,
                value: "~1.3k (1.0%)".to_owned(),
            },
        ],
        free_input: ContextCapacity {
            tokens: 121_904,
            value: "~121.9k (98.3%)".to_owned(),
        },
        reserve: ContextCapacity {
            tokens: 4_096,
            value: "4k (3.2%)".to_owned(),
        },
        model_window: "128k total · 123.9k input budget".to_owned(),
        counting: "estimated · 3 segments".to_owned(),
        compaction: ContextCompaction::Enabled {
            recovery_target: "74.3k".to_owned(),
        },
        tool_context: "offload above 8192 serialized bytes · artifact pages up to 2048 bytes"
            .to_owned(),
        provider_input: "?".to_owned(),
        cache_read: "?".to_owned(),
        cache: "state unknown · CH ? · misses 0 · re-billed 0 · guarantee ? · maintenance calls 0"
            .to_owned(),
        reasoning: "provider default · effort provider default · provider/model default".to_owned(),
        reasoning_controls: "unsupported · switch unavailable · efforts none · resolved model catalog (presence only)"
            .to_owned(),
    })));

    assert!(app.overlay.is_none(), "context output must stay inline");
    for width in [44, 74, 120] {
        let lines = transcript_lines(&app, Theme::new().without_color(), width);
        let screen = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("/context"), "{width} columns:\n{screen}");
        assert!(
            screen.contains("Estimated usage by category"),
            "{width} columns:\n{screen}"
        );
        assert!(
            screen.lines().any(|line| line.contains("■ ◆ ● ● ·")),
            "{width} columns:\n{screen}"
        );
        assert!(
            lines.iter().all(|line| line.width() <= usize::from(width)),
            "{width} columns overflowed:\n{screen}"
        );
    }
}

#[test]
fn empty_error_and_oversized_local_results_name_their_state() {
    use smith_client::message_report::MessageReport;

    let mut empty = App::new("gpt-5.3", "~/work/api");
    empty.show_local_report(LocalResult::Message(Box::new(MessageReport::Empty {
        title: "agents".to_owned(),
        message: String::new(),
    })));
    let empty_screen = render(&empty, 74, 12, Theme::new().without_color());
    assert!(empty_screen.contains("/agents"), "{empty_screen}");
    assert!(empty_screen.contains("● No output."), "{empty_screen}");
    assert!(empty_screen.contains("No output."), "{empty_screen}");

    let mut error = App::new("gpt-5.3", "~/work/api");
    error.show_local_report(LocalResult::Message(Box::new(MessageReport::Error {
        title: "diff".to_owned(),
        message: "Git inspection is unavailable.".to_owned(),
    })));
    let error_screen = render(&error, 74, 12, Theme::new().without_color());
    assert!(error_screen.contains("/diff"), "{error_screen}");
    assert!(
        error_screen.contains("■ Git inspection is unavailable."),
        "{error_screen}"
    );
    assert!(
        error_screen.contains("Git inspection is unavailable."),
        "{error_screen}"
    );

    let mut oversized = App::new("gpt-5.3", "~/work/api");
    oversized.show_local_report(LocalResult::Message(Box::new(MessageReport::Notice {
        title: "diff".to_owned(),
        message: "x".repeat(MAX_LOCAL_RESULT_BYTES + 1),
    })));
    let oversized_screen = render(&oversized, 74, 12, Theme::new().without_color());
    assert!(
        oversized_screen.contains("[local result truncated at the display limit]"),
        "{oversized_screen}"
    );
}

#[tokio::test]
async fn a_diff_marks_its_lines_with_signs_not_only_color() {
    let app = edit_approval("once();\n", "twice();\n").await;
    // Monochrome rendering must still distinguish removal from addition.
    let screen = render(&app, 74, 24, Theme::new().without_color());
    insta_like(&screen, &["- once();", "+ twice();"]);
}

#[tokio::test]
async fn malformed_edit_arguments_fall_back_rather_than_show_an_empty_diff() {
    let mut app = conversation();
    // `new_string` is missing: the call cannot be reviewed truthfully.
    app.present_approval(
        prompt(
            "edit",
            serde_json::json!({"path": "src/retry.rs", "old_string": "once();"}),
        )
        .await,
    );
    let screen = render(&app, 74, 24, Theme::new());

    insta_like(&screen, &["old_string: once();", "y  Yes"]);
    assert!(
        !screen.contains("change  "),
        "an unreviewable edit must not claim a diff:\n{screen}"
    );
}
