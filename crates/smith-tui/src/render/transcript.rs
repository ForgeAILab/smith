//! Transcript, Markdown, tool, status, and local-result rendering.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::status::{Activity, render_elapsed};
use crate::theme::{Theme, Tone, glyph};
use crate::transcript::{Block, LocalResultState, ToolStatus};

use super::helpers::*;

pub(super) fn draw_transcript(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    lines: Option<Vec<Line<'static>>>,
) {
    let lines = lines.unwrap_or_else(|| transcript_lines(app, theme, area.width));
    let max_scroll = visual_scroll_limit(&lines, area);
    let offset = if app.following {
        max_scroll
    } else {
        max_scroll.saturating_sub(app.scroll_back)
    };

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((offset, 0));
    frame.render_widget(paragraph, area);
}

pub(super) fn visual_scroll_limit(lines: &[Line<'static>], area: Rect) -> u16 {
    let rows = rendered_rows(lines, area.width);
    u16::try_from(rows.saturating_sub(usize::from(area.height))).unwrap_or(u16::MAX)
}

/// Rows `lines` occupy under the exact word-wrap arithmetic the paragraphs
/// render with. Every height/scroll estimate must go through here: a local
/// character-wrap guess undercounts word-wrapped prose, which clips the newest
/// transcript rows and truncates modal action bars.
pub(super) fn rendered_rows(lines: &[Line<'static>], width: u16) -> usize {
    if lines.is_empty() {
        return 0;
    }
    Paragraph::new(lines.to_vec())
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
}

pub(super) fn transcript_lines(app: &App, theme: Theme, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for block in app.transcript.blocks() {
        // Reasoning is canonical model state, not a second assistant answer.
        // The turn-level working row below represents progress without
        // exposing raw provider reasoning as transcript prose.
        if matches!(block, Block::Reasoning { .. }) {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        match block {
            Block::User { text } => {
                for (index, raw) in text.lines().enumerate() {
                    let marker = if index == 0 { glyph::USER } else { " " };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{marker} "),
                            theme.style(Tone::Dim).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(raw.to_owned(), theme.style(Tone::Default)),
                    ]));
                }
            }
            Block::Assistant { text, .. } => {
                lines.extend(render_assistant_lines(text, theme));
            }
            Block::Reasoning { .. } => {}
            Block::Tool {
                name,
                display,
                protected_summary,
                status,
                result_preview,
                ..
            } => {
                let tone = match status {
                    ToolStatus::Running => Tone::Dim,
                    ToolStatus::Ok => Tone::Success,
                    ToolStatus::Failed | ToolStatus::Denied => Tone::Danger,
                };
                let invocation = display.as_ref().map_or_else(
                    || format!("{}({protected_summary})", safe_tool_name(name)),
                    smith_tools::ToolCallDisplay::invocation,
                );
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{} ", glyph::TOOL),
                        theme.style(tone).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(invocation, theme.style(Tone::Heading)),
                    Span::styled(" · ", theme.style(Tone::Dim)),
                    Span::styled(status.label(), theme.style(tone)),
                ]));
                if !matches!(status, ToolStatus::Running)
                    && let Some(preview) = result_preview
                {
                    for raw in preview.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("    {raw}"),
                            theme.style(Tone::Dim),
                        )));
                    }
                }
            }
            Block::Error { message } => {
                for (index, raw) in message.lines().enumerate() {
                    let marker = if index == 0 { glyph::ERROR } else { " " };
                    lines.push(Line::from(Span::styled(
                        format!("{marker} {raw}"),
                        theme.style(Tone::Danger),
                    )));
                }
            }
            // Turn boundaries read as quiet punctuation — "Worked for 5s" —
            // not as a sourced notice row.
            Block::Notice { source, text } if source == "turn" => {
                for raw in text.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("  {raw}"),
                        theme.style(Tone::Dim),
                    )));
                }
            }
            Block::Notice { source, text } => {
                for (index, raw) in text.lines().enumerate() {
                    if index == 0 {
                        lines.push(Line::from(vec![
                            Span::styled(format!("{} ", glyph::NOTICE), theme.style(Tone::Dim)),
                            Span::styled(source.clone(), theme.style(Tone::Heading)),
                            Span::styled(" · ", theme.style(Tone::Dim)),
                            Span::styled(raw.to_owned(), theme.style(Tone::Default)),
                        ]));
                    } else {
                        lines.push(Line::from(Span::styled(
                            format!("  {raw}"),
                            theme.style(Tone::Dim),
                        )));
                    }
                }
            }
            Block::LocalResult {
                title,
                content,
                state,
            } => {
                lines.push(Line::from(Span::styled(
                    format!("/{title}"),
                    theme.style(Tone::Command),
                )));
                match state {
                    LocalResultState::Info if title == "status" => {
                        lines.extend(render_status_card(content, width, theme));
                    }
                    LocalResultState::Info if title == "context" => {
                        lines.extend(render_context_content(content, width, theme));
                    }
                    LocalResultState::Info => {
                        lines.extend(render_local_content(title, content, width, theme));
                    }
                    LocalResultState::Empty => {
                        lines.extend(render_prefixed_local_state(
                            glyph::BULLET,
                            content,
                            width,
                            theme.style(Tone::Dim),
                        ));
                    }
                    LocalResultState::Error => {
                        lines.extend(render_prefixed_local_state(
                            glyph::ERROR,
                            content,
                            width,
                            theme.style(Tone::Danger),
                        ));
                    }
                }
            }
        }
    }

    if let Some(text) = app.speculative_text() {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.extend(render_speculative_lines(text, theme));
    }

    if let Some(summary) = &app.turn_summary
        && !matches!(
            app.status.activity,
            Activity::Working | Activity::Interrupting
        )
    {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::from(Span::styled(
            format!("  {summary}"),
            theme.style(Tone::Dim),
        )));
    }

    if matches!(
        app.status.activity,
        Activity::Working | Activity::Interrupting
    ) {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        let label = match app.status.activity {
            Activity::Working => "Working",
            Activity::Interrupting => "Interrupting",
            Activity::Idle | Activity::Ended => unreachable!("activity was filtered above"),
        };
        let details = app.work_detail_lines();
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", theme.spinner(app.tick)),
                theme.style(Tone::Accent),
            ),
            Span::styled(
                format!(
                    "{label}{} · {}{}",
                    glyph::ELIDED,
                    app.turn_elapsed()
                        .map(render_elapsed)
                        .unwrap_or_else(|| "?".to_owned()),
                    details
                        .first()
                        .map(|line| format!(" · {line}"))
                        .unwrap_or_default(),
                ),
                theme.style(Tone::Reasoning),
            ),
        ]));
        for detail in details.iter().skip(1) {
            lines.push(Line::from(vec![
                Span::styled("  ", theme.style(Tone::Dim)),
                Span::styled(detail.clone(), theme.style(Tone::Reasoning)),
            ]));
        }
    }

    lines
}

// Streaming answer text renders exactly like committed prose — no "draft"
// label. Only reasoning stays behind the dim working row; a later discard
// simply removes these lines.
pub(super) fn render_speculative_lines(text: &str, theme: Theme) -> Vec<Line<'static>> {
    text.lines()
        .enumerate()
        .map(|(index, raw)| {
            let spans = vec![
                Span::styled(
                    if index == 0 {
                        format!("{} ", glyph::BULLET)
                    } else {
                        "  ".to_owned()
                    },
                    theme.style(Tone::Dim),
                ),
                Span::styled(raw.to_owned(), theme.style(Tone::Default)),
            ];
            Line::from(spans)
        })
        .collect()
}

pub(super) fn safe_tool_name(name: &str) -> String {
    let name = name
        .chars()
        .take(64)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if name.is_empty() {
        "tool".to_owned()
    } else {
        name
    }
}

pub(super) fn render_assistant_lines(text: &str, theme: Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut first = true;
    let mut in_code_block = false;

    for raw in text.lines() {
        if raw.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        let mut spans = vec![Span::styled(
            if first {
                format!("{} ", glyph::BULLET)
            } else {
                "  ".to_owned()
            },
            theme.style(Tone::Dim),
        )];
        if in_code_block {
            spans.push(Span::styled(raw.to_owned(), theme.style(Tone::Code)));
        } else {
            spans.extend(render_markdown_spans(raw, theme));
        }
        lines.push(Line::from(spans));
        first = false;
    }

    if lines.is_empty() && !text.is_empty() {
        lines.push(Line::from(Span::styled(
            glyph::BULLET,
            theme.style(Tone::Dim),
        )));
    }
    lines
}

pub(super) fn render_markdown_spans(raw: &str, theme: Theme) -> Vec<Span<'static>> {
    let trimmed = raw.trim_start();
    let leading = &raw[..raw.len().saturating_sub(trimmed.len())];
    let heading_marks = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    let is_heading = (1..=6).contains(&heading_marks)
        && trimmed
            .as_bytes()
            .get(heading_marks)
            .is_some_and(u8::is_ascii_whitespace);
    let (body, base) = if is_heading {
        let body = trimmed[heading_marks..].trim_start();
        let style = match heading_marks {
            1 => theme
                .style(Tone::Heading)
                .add_modifier(Modifier::UNDERLINED),
            2 => theme.style(Tone::Heading),
            3 => theme.style(Tone::Heading).add_modifier(Modifier::ITALIC),
            _ => theme.style(Tone::Default).add_modifier(Modifier::ITALIC),
        };
        (body, style)
    } else {
        (raw, theme.style(Tone::Default))
    };

    let mut spans = Vec::new();
    if is_heading && !leading.is_empty() {
        spans.push(Span::styled(leading.to_owned(), base));
    }
    spans.extend(render_inline_markdown(body, base, theme));
    spans
}

pub(super) fn render_inline_markdown(raw: &str, base: Style, theme: Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = raw;

    while !rest.is_empty() {
        if let Some(link) = rest.strip_prefix('[')
            && let Some(label_end) = link.find("](")
            && let Some(target_end) = link[label_end + 2..].find(')')
        {
            spans.push(Span::styled(
                link[..label_end].to_owned(),
                base.patch(theme.style(Tone::Link)),
            ));
            rest = &link[label_end + 2 + target_end + 1..];
            continue;
        }
        if let Some(strong) = rest.strip_prefix("**")
            && let Some(end) = strong.find("**")
        {
            spans.push(Span::styled(
                strong[..end].to_owned(),
                base.add_modifier(Modifier::BOLD),
            ));
            rest = &strong[end + 2..];
            continue;
        }
        if let Some(code) = rest.strip_prefix('`')
            && let Some(end) = code.find('`')
        {
            spans.push(Span::styled(
                code[..end].to_owned(),
                base.patch(theme.style(Tone::Code)),
            ));
            rest = &code[end + 1..];
            continue;
        }
        if let Some(emphasis) = rest.strip_prefix('*')
            && let Some(end) = emphasis.find('*')
        {
            spans.push(Span::styled(
                emphasis[..end].to_owned(),
                base.add_modifier(Modifier::ITALIC),
            ));
            rest = &emphasis[end + 1..];
            continue;
        }

        let next = ["[", "**", "`", "*"]
            .into_iter()
            .filter_map(|delimiter| rest.find(delimiter))
            .filter(|index| *index > 0)
            .min()
            .unwrap_or(rest.len());
        if next == 0 {
            let first = rest.chars().next().expect("rest was checked as non-empty");
            spans.push(Span::styled(first.to_string(), base));
            rest = &rest[first.len_utf8()..];
        } else {
            spans.push(Span::styled(rest[..next].to_owned(), base));
            rest = &rest[next..];
        }
    }

    spans
}

pub(super) fn render_status_card(content: &str, width: u16, theme: Theme) -> Vec<Line<'static>> {
    let available = usize::from(width.saturating_sub(4)).max(1);
    let parsed = content
        .lines()
        .map(|raw| raw.split_once(':'))
        .collect::<Vec<_>>();
    let label_width = parsed
        .iter()
        .flatten()
        .map(|(label, _)| label.trim().width())
        .max()
        .unwrap_or(0);

    let mut body = vec![
        Line::from(vec![
            Span::styled(" >_ ", theme.style(Tone::Dim)),
            Span::styled("Smith", theme.style(Tone::Heading)),
        ]),
        Line::default(),
    ];

    for (raw, field) in content.lines().zip(parsed) {
        if let Some((label, value)) = field {
            body.extend(render_status_field(
                label.trim(),
                value.trim_start(),
                label_width,
                available,
                theme,
            ));
        } else {
            body.extend(
                wrap_text(raw, available)
                    .into_iter()
                    .map(|line| Line::from(Span::styled(line, theme.style(Tone::Default)))),
            );
        }
    }

    let inner_width = body
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(1)
        .min(available)
        .max(1);
    let mut bordered = Vec::with_capacity(body.len() + 2);
    bordered.push(Line::from(Span::styled(
        format!("╭{}╮", "─".repeat(inner_width + 2)),
        theme.style(Tone::Dim),
    )));
    for line in body {
        let used = line.width().min(inner_width);
        let mut spans = vec![Span::styled("│ ", theme.style(Tone::Dim))];
        spans.extend(line.spans);
        spans.push(Span::styled(
            format!("{} │", " ".repeat(inner_width.saturating_sub(used))),
            theme.style(Tone::Dim),
        ));
        bordered.push(Line::from(spans));
    }
    bordered.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(inner_width + 2)),
        theme.style(Tone::Dim),
    )));
    bordered
}

pub(super) fn render_status_field(
    label: &str,
    value: &str,
    label_width: usize,
    available: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    let padding = 3 + label_width.saturating_sub(label.width());
    let prefix = format!(" {label}:{}", " ".repeat(padding));
    let prefix_width = prefix.width();
    if prefix_width >= available {
        return wrap_text(&format!("{label}: {value}"), available)
            .into_iter()
            .map(|line| Line::from(Span::styled(line, theme.style(Tone::Default))))
            .collect();
    }

    let chunks = wrap_text(value, available - prefix_width);
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            if index == 0 {
                Line::from(vec![
                    Span::styled(prefix.clone(), theme.style(Tone::Dim)),
                    Span::styled(chunk, theme.style(Tone::Default)),
                ])
            } else {
                Line::from(vec![
                    Span::styled(" ".repeat(prefix_width), theme.style(Tone::Dim)),
                    Span::styled(chunk, theme.style(Tone::Default)),
                ])
            }
        })
        .collect()
}

pub(super) fn render_local_content(
    title: &str,
    content: &str,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let available = usize::from(width).max(1);
    let mut lines = Vec::new();
    for raw in content.lines() {
        for wrapped in wrap_text(raw, available) {
            lines.push(styled_local_line(title, &wrapped, theme));
        }
    }
    lines
}

pub(super) fn render_context_content(
    content: &str,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let available = usize::from(width).max(1);
    let mut lines = Vec::new();
    for raw in content.lines() {
        for wrapped in wrap_text(raw, available) {
            lines.push(styled_context_line(&wrapped, theme));
        }
    }
    lines
}

pub(super) fn styled_context_line(raw: &str, theme: Theme) -> Line<'static> {
    if raw.is_empty() {
        return Line::default();
    }
    if raw == "Context usage" {
        return Line::from(Span::styled(raw.to_owned(), theme.style(Tone::Heading)));
    }
    if matches!(
        raw,
        "Exact usage by category" | "Estimated usage by category" | "Available capacity"
    ) {
        return Line::from(Span::styled(
            raw.to_owned(),
            theme.style(Tone::Dim).add_modifier(Modifier::ITALIC),
        ));
    }

    let glyph_tone = |character| match character {
        '■' => Some(Tone::Accent),
        '◆' => Some(Tone::Warning),
        '●' => Some(Tone::Command),
        '▲' => Some(Tone::Success),
        '✦' => Some(Tone::Accent),
        '+' => Some(Tone::Default),
        '·' | '□' => Some(Tone::Dim),
        _ => None,
    };
    if raw.chars().any(|character| glyph_tone(character).is_some())
        && raw
            .chars()
            .all(|character| character.is_whitespace() || glyph_tone(character).is_some())
    {
        return Line::from(
            raw.chars()
                .map(|character| {
                    Span::styled(
                        character.to_string(),
                        glyph_tone(character)
                            .map_or_else(|| theme.style(Tone::Default), |tone| theme.style(tone)),
                    )
                })
                .collect::<Vec<_>>(),
        );
    }

    if let Some(first) = raw.chars().next()
        && let Some(tone) = glyph_tone(first)
        && let Some(rest) = raw[first.len_utf8()..].strip_prefix(' ')
    {
        if let Some((label, value)) = rest.split_once(':') {
            return Line::from(vec![
                Span::styled(first.to_string(), theme.style(tone)),
                Span::raw(" "),
                Span::styled(format!("{label}:"), theme.style(Tone::Default)),
                Span::styled(value.to_owned(), theme.style(Tone::Dim)),
            ]);
        }
        return Line::from(vec![
            Span::styled(first.to_string(), theme.style(tone)),
            Span::styled(format!(" {rest}"), theme.style(Tone::Default)),
        ]);
    }

    if let Some((label, value)) = raw.split_once(':') {
        let value_tone = if label == "counting" && value.trim_start() != "exact tokenizer" {
            Tone::Warning
        } else if label == "compaction" && value.trim_start().starts_with("applied") {
            Tone::Success
        } else {
            Tone::Default
        };
        return Line::from(vec![
            Span::styled(format!("{label}:"), theme.style(Tone::Dim)),
            Span::styled(value.to_owned(), theme.style(value_tone)),
        ]);
    }

    Line::from(Span::styled(raw.to_owned(), theme.style(Tone::Dim)))
}

pub(super) fn styled_local_line(title: &str, raw: &str, theme: Theme) -> Line<'static> {
    if raw.is_empty() {
        return Line::default();
    }
    if title == "help" {
        if matches!(raw, "Primary" | "Advanced") {
            return Line::from(Span::styled(raw.to_owned(), theme.style(Tone::Heading)));
        }
        if let Some((command, description)) = raw.split_once(" — ") {
            return Line::from(vec![
                Span::styled(command.to_owned(), theme.style(Tone::Code)),
                Span::styled("  ", theme.style(Tone::Dim)),
                Span::styled(description.to_owned(), theme.style(Tone::Dim)),
            ]);
        }
        return Line::from(Span::styled(raw.to_owned(), theme.style(Tone::Dim)));
    }

    if title.starts_with("diff") {
        let tone = if raw.starts_with("@@") {
            Tone::Code
        } else if raw.starts_with('+') && !raw.starts_with("+++") {
            Tone::Success
        } else if raw.starts_with('-') && !raw.starts_with("---") {
            Tone::Danger
        } else if raw.starts_with("diff --git")
            || raw.starts_with("index ")
            || raw.starts_with("---")
            || raw.starts_with("+++")
        {
            Tone::Dim
        } else {
            Tone::Default
        };
        return Line::from(Span::styled(raw.to_owned(), theme.style(tone)));
    }

    if let Some((label, value)) = raw.split_once(':') {
        return Line::from(vec![
            Span::styled(format!("{label}:"), theme.style(Tone::Dim)),
            Span::styled(value.to_owned(), theme.style(Tone::Default)),
        ]);
    }
    Line::from(render_inline_markdown(
        raw,
        theme.style(Tone::Default),
        theme,
    ))
}

pub(super) fn render_prefixed_local_state(
    marker: &str,
    content: &str,
    width: u16,
    style: Style,
) -> Vec<Line<'static>> {
    let available = usize::from(width.saturating_sub(2)).max(1);
    let mut lines = Vec::new();
    for raw in content.lines() {
        for (index, wrapped) in wrap_text(raw, available).into_iter().enumerate() {
            lines.push(Line::from(vec![
                Span::styled(
                    if index == 0 {
                        format!("{marker} ")
                    } else {
                        "  ".to_owned()
                    },
                    style,
                ),
                Span::styled(wrapped, style),
            ]));
        }
    }
    lines
}
