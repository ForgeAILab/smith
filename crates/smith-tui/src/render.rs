//! Rendering, per the layout in `DESIGN.md` §2.
//!
//! Drawing is a pure function of [`App`] plus the frame area. Nothing here
//! mutates state or caches across frames, which is what makes resize correct by
//! construction: every wrap is recomputed at the new width.

use std::time::{SystemTime, UNIX_EPOCH};

use agent_runtime_core::clock::Deadline;
use agent_runtime_core::event::{PlanItemStatus, PlanSensitivity};
use agent_runtime_core::security::SecurityResource;
use agent_runtime_core::tool::PreparedToolCall;
use agent_runtime_registry::Permission;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WidgetBlock, Borders, Clear, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Overlay};
use crate::commands;
use crate::diff::{Change, EditReview};
use crate::picker::{compact_resource_picker_rows, draw_compact_resource_picker};
use crate::questionnaire::{QuestionnaireFocus, QuestionnaireState};
use crate::status::{Activity, render_elapsed};
use crate::theme::{Theme, Tone, glyph};
#[cfg(test)]
use crate::transcript::MAX_LOCAL_RESULT_BYTES;
use crate::transcript::{Block, LocalResultState, ToolStatus};

/// The smallest terminal Smith will draw a working surface in. Below this a
/// half-rendered coding surface is worse than an honest refusal.
pub const MIN_WIDTH: u16 = 40;
/// See [`MIN_WIDTH`].
pub const MIN_HEIGHT: u16 = 10;

/// The most body lines a modal builds before the height budget takes over.
///
/// A pathological argument blob would otherwise cost a `Line` per JSON line on
/// every frame, and no terminal is tall enough to show them.
const MAX_BODY_LINES: usize = 128;

/// Maximum public todo items kept immediately above the composer.
const MAX_VISIBLE_TODOS: usize = 5;

/// Draws the whole client without changing application state.
///
/// Interactive hosts should use [`draw_synced`] so scroll input is bounded by
/// the current terminal geometry. This pure entry point remains useful for
/// snapshots and other read-only renderers.
pub fn draw(frame: &mut Frame<'_>, app: &App, theme: Theme) {
    draw_surface(frame, app, theme, None);
}

/// Draws the client and synchronizes transcript scroll bounds with the frame.
pub fn draw_synced(frame: &mut Frame<'_>, app: &mut App, theme: Theme) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        app.sync_scroll_limit(0);
        draw_too_small(frame, area, theme);
        return;
    }

    let transcript = transcript_area(area, app);
    let lines = transcript_lines(app, theme, transcript.width);
    let limit = visual_scroll_limit(&lines, transcript);
    app.sync_scroll_limit(limit);
    draw_surface(frame, app, theme, Some(lines));
}

fn draw_surface(
    frame: &mut Frame<'_>,
    app: &App,
    theme: Theme,
    mut transcript_lines: Option<Vec<Line<'static>>>,
) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        draw_too_small(frame, area, theme);
        return;
    }

    let composer_rows = composer_rows(app, area.width).saturating_add(2);
    let anchored = anchored_rows(app, area, composer_rows);
    let [transcript, compact, todos, composer, hint] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(anchored.compact),
        Constraint::Length(anchored.todos),
        Constraint::Length(composer_rows),
        Constraint::Length(hint_rows(app)),
    ])
    .areas(area);
    draw_transcript(frame, transcript, app, theme, transcript_lines.take());
    if anchored.compact > 0 {
        match &app.overlay {
            Some(Overlay::Palette {
                selected, error, ..
            }) => draw_palette(frame, compact, app, *selected, error.as_deref(), theme),
            Some(Overlay::ResourcePicker { picker, .. }) => {
                draw_compact_resource_picker(frame, compact, picker, theme);
            }
            _ => unreachable!("compact rows require a compact interaction"),
        }
    }
    if anchored.todos > 0 {
        draw_todos(frame, todos, app, theme);
    }
    draw_composer(frame, composer, app, theme);
    draw_hint(frame, hint, app, theme);

    match &app.overlay {
        Some(Overlay::Approval { prompt, review }) => {
            draw_approval(frame, area, prompt, review.as_ref(), theme);
        }
        Some(Overlay::Questionnaire { state }) => {
            draw_questionnaire(frame, area, state, theme);
        }
        Some(Overlay::Palette { .. } | Overlay::ResourcePicker { .. }) => {}
        Some(Overlay::UndoConfirm { content }) => {
            draw_recovery_confirm(frame, area, "undo last Smith turn", content, theme);
        }
        Some(Overlay::RedoConfirm { content }) => {
            draw_recovery_confirm(frame, area, "redo last exact Smith turn", content, theme);
        }
        Some(Overlay::RevertConfirm { content, .. }) => {
            draw_recovery_confirm(frame, area, "revert selected change", content, theme);
        }
        Some(Overlay::ReviewConfirm { content, .. }) => {
            draw_review_confirm(frame, area, content, theme);
        }
        Some(Overlay::AgentConfirm { content, .. }) => {
            draw_agent_confirm(frame, area, content, theme);
        }
        Some(Overlay::AgentFollowUpConfirm { content, .. }) => {
            draw_child_continuation_confirm(
                frame,
                area,
                "existing child follow-up",
                " start follow-up and spend provider tokens   ",
                content,
                theme,
            );
        }
        Some(Overlay::AgentResumeConfirm { content, .. }) => {
            draw_child_continuation_confirm(
                frame,
                area,
                "resume interrupted child",
                " resume exact checkpoint   ",
                content,
                theme,
            );
        }
        Some(Overlay::ExitConfirm {
            approval,
            questionnaire,
        }) => {
            draw_exit_confirm(
                frame,
                area,
                app,
                approval.is_some(),
                questionnaire.is_some(),
                theme,
            );
        }
        None => {}
    }
}

fn transcript_area(area: Rect, app: &App) -> Rect {
    let composer_rows = composer_rows(app, area.width).saturating_add(2);
    let anchored = anchored_rows(app, area, composer_rows);
    let [transcript, _, _, _, _] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(anchored.compact),
        Constraint::Length(anchored.todos),
        Constraint::Length(composer_rows),
        Constraint::Length(hint_rows(app)),
    ])
    .areas(area);
    transcript
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AnchoredRows {
    compact: u16,
    todos: u16,
}

fn anchored_rows(app: &App, area: Rect, composer_rows: u16) -> AnchoredRows {
    let compact_desired = match &app.overlay {
        Some(Overlay::Palette { error, .. }) => desired_palette_rows(app, error.as_deref()),
        Some(Overlay::ResourcePicker { picker, .. }) => compact_resource_picker_rows(picker),
        _ => 0,
    };
    let available = area
        .height
        .saturating_sub(composer_rows)
        .saturating_sub(hint_rows(app))
        .saturating_sub(3);
    let todo_desired = desired_todo_rows(app);
    let (compact, todos) = if compact_desired > 0 {
        // Compact interactions temporarily own the anchored pane. The todo
        // projection remains in App state and returns unchanged when the
        // interaction closes.
        (compact_desired.min(available), 0)
    } else {
        (0, todo_desired.min(available))
    };
    AnchoredRows { compact, todos }
}

fn desired_todo_rows(app: &App) -> u16 {
    let Some(plan) = &app.plan else {
        return 0;
    };
    if plan.sensitivity != PlanSensitivity::Public {
        return 0;
    }
    let Some(items) = plan.items.as_ref().filter(|items| !items.is_empty()) else {
        return 0;
    };
    u16::try_from(items.len().min(MAX_VISIBLE_TODOS).saturating_add(1)).unwrap_or(u16::MAX)
}

fn hint_rows(app: &App) -> u16 {
    if has_stacked_control_hint(app) { 2 } else { 1 }
}

fn has_stacked_control_hint(app: &App) -> bool {
    matches!(app.overlay, Some(Overlay::ResourcePicker { .. }))
}

fn draw_too_small(frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let text = format!("terminal too small (need {MIN_WIDTH}×{MIN_HEIGHT})");
    frame.render_widget(Paragraph::new(text).style(theme.style(Tone::Warning)), area);
}

fn composer_rows(app: &App, width: u16) -> u16 {
    let usable = usize::from(width.saturating_sub(2)).max(1);
    let wrapped: usize = app
        .composer
        .lines()
        .iter()
        .map(|line| line.width().div_ceil(usable).max(1))
        .sum();
    u16::try_from(wrapped.clamp(1, 8)).unwrap_or(1)
}

fn push_segment(spans: &mut Vec<Span<'static>>, theme: Theme, segment: Span<'static>) {
    spans.push(Span::styled(
        format!(" {} ", glyph::SEPARATOR),
        theme.style(Tone::Dim),
    ));
    spans.push(segment);
}

/// Drops whole spans from the right until the line fits, so the footer never
/// wraps into the transcript.
fn truncate(spans: Vec<Span<'static>>, width: u16) -> Vec<Span<'static>> {
    let limit = usize::from(width);
    let mut used = 0;
    let mut kept = Vec::new();
    for span in spans {
        let span_width = span.content.width();
        if used + span_width > limit {
            break;
        }
        used += span_width;
        kept.push(span);
    }
    if kept
        .last()
        .is_some_and(|span| span.content.as_ref() == " · ")
    {
        kept.pop();
    }
    kept
}

// ---------------------------------------------------------------------------
// Transcript
// ---------------------------------------------------------------------------

fn draw_transcript(
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

fn visual_scroll_limit(lines: &[Line<'_>], area: Rect) -> u16 {
    let height = usize::from(area.height);
    let width = usize::from(area.width).max(1);

    // A blank line separates blocks so screen-reader paragraph navigation
    // matches Smith's block structure (`DESIGN.md` §9).
    let visual_rows = lines
        .iter()
        .map(|line| line.width().div_ceil(width).max(1))
        .sum::<usize>();
    u16::try_from(visual_rows.saturating_sub(height)).unwrap_or(u16::MAX)
}

fn transcript_lines(app: &App, theme: Theme, width: u16) -> Vec<Line<'static>> {
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

fn render_speculative_lines(text: &str, theme: Theme) -> Vec<Line<'static>> {
    text.lines()
        .enumerate()
        .map(|(index, raw)| {
            let mut spans = vec![Span::styled(
                if index == 0 {
                    format!("{} ", glyph::BULLET)
                } else {
                    "  ".to_owned()
                },
                theme.style(Tone::Dim),
            )];
            if index == 0 {
                spans.push(Span::styled("draft · ", theme.style(Tone::Warning)));
            }
            spans.push(Span::styled(raw.to_owned(), theme.style(Tone::Reasoning)));
            Line::from(spans)
        })
        .collect()
}

fn safe_tool_name(name: &str) -> String {
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

fn render_assistant_lines(text: &str, theme: Theme) -> Vec<Line<'static>> {
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

fn render_markdown_spans(raw: &str, theme: Theme) -> Vec<Span<'static>> {
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

fn render_inline_markdown(raw: &str, base: Style, theme: Theme) -> Vec<Span<'static>> {
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

fn render_status_card(content: &str, width: u16, theme: Theme) -> Vec<Line<'static>> {
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

fn render_status_field(
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

fn render_local_content(
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

fn render_context_content(content: &str, width: u16, theme: Theme) -> Vec<Line<'static>> {
    let available = usize::from(width).max(1);
    let mut lines = Vec::new();
    for raw in content.lines() {
        for wrapped in wrap_text(raw, available) {
            lines.push(styled_context_line(&wrapped, theme));
        }
    }
    lines
}

fn styled_context_line(raw: &str, theme: Theme) -> Line<'static> {
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

fn styled_local_line(title: &str, raw: &str, theme: Theme) -> Line<'static> {
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

fn render_prefixed_local_state(
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

fn wrap_text(raw: &str, available: usize) -> Vec<String> {
    if raw.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for character in raw.chars() {
        let character_width = character.width().unwrap_or(0);
        if used > 0 && used + character_width > available {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        line.push(character);
        used += character_width;
    }
    lines.push(line);
    lines
}

// ---------------------------------------------------------------------------
// Composer and hint
// ---------------------------------------------------------------------------

fn draw_composer(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let input_area = Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(2),
    );
    let empty = app.composer.text().is_empty();
    let lines: Vec<Line<'static>> = app
        .composer
        .lines()
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let marker = if index == 0 { glyph::USER } else { " " };
            let mut spans = vec![Span::styled(
                format!("{marker} "),
                theme.style(Tone::Default).add_modifier(Modifier::BOLD),
            )];
            if empty && index == 0 {
                spans.push(Span::styled(
                    "Ask Smith to do anything",
                    theme.style(Tone::Dim),
                ));
            } else {
                spans.push(Span::raw((*line).to_owned()));
            }
            Line::from(spans)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), input_area);

    if app.overlay.is_none() || matches!(app.overlay, Some(Overlay::Palette { .. })) {
        let (line, column) = app.composer.cursor_position();
        let x = input_area.x + 2 + u16::try_from(column).unwrap_or(0);
        let y = input_area.y + u16::try_from(line).unwrap_or(0);
        if x < input_area.right() && y < input_area.bottom() {
            frame.set_cursor_position((x, y));
        }
    }
}

fn draw_todos(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let Some(items) = app.plan.as_ref().and_then(|plan| plan.items.as_ref()) else {
        return;
    };
    let capacity = usize::from(area.height).saturating_sub(1);
    let mut lines = vec![Line::from(Span::styled(
        "  Todo",
        theme.style(Tone::Heading),
    ))];
    lines.extend(items.iter().take(capacity).map(|item| {
        let (marker, tone) = match item.status {
            PlanItemStatus::Pending => ("[ ]", Tone::Default),
            PlanItemStatus::InProgress => ("[>]", Tone::Accent),
            PlanItemStatus::Completed => ("[x]", Tone::Success),
            PlanItemStatus::Cancelled => ("[-]", Tone::Dim),
        };
        let text = item.text.split_whitespace().collect::<Vec<_>>().join(" ");
        Line::from(vec![
            Span::styled(format!("  {marker} "), theme.style(tone)),
            Span::styled(
                if text.is_empty() {
                    item.id.clone()
                } else {
                    text
                },
                theme.style(tone),
            ),
        ])
    }));
    // Todo rows are fixed-height composer chrome; long text clips instead of
    // wrapping and moving the input.
    frame.render_widget(Paragraph::new(lines), area);
}

fn overlay_hint(app: &App) -> Option<String> {
    match &app.overlay {
        Some(Overlay::Approval { .. }) => {
            let waiting = app.pending_approval_count().saturating_sub(1);
            Some(if waiting == 0 {
                "y allow once · a allow for session · n deny".to_owned()
            } else {
                format!("y allow once · a allow for session · n deny · {waiting} queued")
            })
        }
        Some(Overlay::Questionnaire { state }) => {
            let queued = app.pending_questionnaire_count().saturating_sub(1);
            let answer = if state.question().choices.is_empty() {
                "type answer".to_owned()
            } else if state.question().allows_free_form {
                "↑↓ choose · space stage · type other".to_owned()
            } else {
                "↑↓ choose · space stage".to_owned()
            };
            Some(if queued == 0 {
                format!("{answer} · tab actions · esc cancel")
            } else {
                format!("{answer} · tab actions · esc cancel · {queued} queued")
            })
        }
        Some(Overlay::Palette { .. }) => None,
        Some(Overlay::ResourcePicker { .. }) => {
            Some("type to filter · ↑↓ choose · enter confirm · esc cancel".to_owned())
        }
        Some(Overlay::UndoConfirm { .. }) => Some("y apply undo · n/esc cancel".to_owned()),
        Some(Overlay::RedoConfirm { .. }) => Some("y apply redo · n/esc cancel".to_owned()),
        Some(Overlay::RevertConfirm { .. }) => Some("y apply revert · n/esc cancel".to_owned()),
        Some(Overlay::ReviewConfirm { .. }) => Some("y start review · n/esc cancel".to_owned()),
        Some(Overlay::AgentConfirm { .. }) => {
            Some("y start read-only child · n/esc cancel".to_owned())
        }
        Some(Overlay::AgentFollowUpConfirm { .. }) => {
            Some("y start follow-up turn · n/esc cancel".to_owned())
        }
        Some(Overlay::AgentResumeConfirm { .. }) => {
            Some("y resume exact checkpoint · n/esc cancel".to_owned())
        }
        Some(Overlay::ExitConfirm { .. }) => Some("y quit · n keep working".to_owned()),
        None => None,
    }
}

fn draw_hint(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    if let Some(hint) = overlay_hint(app) {
        if has_stacked_control_hint(app) && area.height > 1 {
            let [identity, controls] =
                Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
            draw_identity_footer(frame, identity, app, theme);
            draw_control_hint(frame, controls, &hint, theme);
        } else {
            draw_control_hint(frame, area, &hint, theme);
        }
        return;
    }

    draw_identity_footer(frame, area, app, theme);
}

fn draw_control_hint(frame: &mut Frame<'_>, area: Rect, hint: &str, theme: Theme) {
    frame.render_widget(
        Paragraph::new(Line::from(truncate(
            vec![
                Span::styled("  ", theme.style(Tone::Dim)),
                Span::styled(hint.to_owned(), theme.style(Tone::Dim)),
            ],
            area.width,
        ))),
        area,
    );
}

fn draw_identity_footer(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    // Identity lives at the point of action. During work, activity replaces
    // the idle mode label; no permanent header or focusable status region is
    // introduced.
    let mut identity = vec![Span::styled("  ", theme.style(Tone::Dim))];
    if !app.following {
        identity.push(Span::styled(
            "▼ following paused · End/Ctrl+L newest".to_owned(),
            theme.style(Tone::Warning),
        ));
        push_segment(
            &mut identity,
            theme,
            Span::styled(
                if app.is_busy() {
                    app.status.activity.label().to_owned()
                } else {
                    app.status.agent.clone()
                },
                theme.style(Tone::Accent),
            ),
        );
    } else {
        identity.push(Span::styled(
            if app.is_busy() {
                app.status.activity.label().to_owned()
            } else {
                app.status.agent.clone()
            },
            theme.style(Tone::Accent),
        ));
    }
    let model = match &app.status.provider {
        Some(provider) => format!("{provider}/{}", app.status.model),
        None => app.status.model.clone(),
    };
    push_segment(
        &mut identity,
        theme,
        Span::styled(model, theme.style(Tone::StatusModel)),
    );
    push_segment(
        &mut identity,
        theme,
        Span::styled(app.status.project.clone(), theme.style(Tone::StatusPath)),
    );
    let context = app.status.render_context_footer();
    push_segment(
        &mut identity,
        theme,
        Span::styled(
            context,
            theme.style(
                if app.status.context_plan.as_ref().is_some_and(|plan| {
                    plan.confidence == agent_runtime_core::event::EstimationConfidence::Exact
                }) {
                    Tone::Default
                } else {
                    Tone::Warning
                },
            ),
        ),
    );

    frame.render_widget(
        Paragraph::new(Line::from(truncate(identity, area.width))),
        area,
    );
}

// ---------------------------------------------------------------------------
// Consequential overlays
// ---------------------------------------------------------------------------

fn draw_approval(
    frame: &mut Frame<'_>,
    area: Rect,
    prompt: &smith_host::approval::ApprovalPrompt,
    review: Option<&EditReview>,
    theme: Theme,
) {
    let prepared = prompt.prepared();

    // The exact prepared target is the first rendered text so a screen reader
    // and the tightest supported terminal retain the facts a user cannot
    // answer without (`DESIGN.md` §9).
    let identity = vec![
        Span::styled(
            format!("{} approval required  ", glyph::APPROVAL),
            theme.style(Tone::Heading),
        ),
        Span::styled(
            security_resource_text(prepared.resource()),
            theme.style(Tone::Danger),
        ),
    ];
    let mut head = vec![Line::from(identity)];
    head.push(Line::from(vec![
        Span::styled(prepared.tool().to_owned(), theme.style(Tone::Dim)),
        Span::styled(format!("  {}  ", glyph::SEPARATOR), theme.style(Tone::Dim)),
        Span::raw(prepared.display().title.clone()),
    ]));

    let mut detail = Vec::new();
    if let Some(display_detail) = &prepared.display().detail {
        detail.push(Line::from(vec![
            Span::styled("action  ", theme.style(Tone::Dim)),
            Span::raw(display_detail.clone()),
        ]));
    }
    if let Some(review) = review {
        detail.push(Line::from(vec![
            Span::styled("change  ", theme.style(Tone::Dim)),
            Span::raw(review.summary()),
        ]));
    }

    let permissions = prepared
        .required_permissions()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if !permissions.is_empty() {
        detail.push(Line::from(vec![
            Span::styled("permissions  ", theme.style(Tone::Dim)),
            Span::styled(permissions.join(", "), theme.style(Tone::Danger)),
        ]));
    }
    if let Some(warning) = authority_warning(prepared) {
        detail.push(Line::from(Span::styled(
            format!("{} {warning}", glyph::WARNING),
            theme.style(Tone::Warning),
        )));
    }
    detail.push(Line::from(vec![
        Span::styled("deadline  ", theme.style(Tone::Dim)),
        Span::raw(deadline_text(prompt.deadline())),
    ]));
    detail.push(Line::from(vec![
        Span::styled("fingerprint  ", theme.style(Tone::Dim)),
        Span::raw(prepared.fingerprint().as_str().to_owned()),
    ]));
    detail.push(Line::default());

    let (body, elided) = match review {
        Some(review) => review_lines(review, theme),
        None => argument_lines(prepared.arguments(), theme),
    };

    let content = ModalContent {
        head,
        detail,
        body,
        elided,
        // No default action, and no `Enter`: the bar names all three keys.
        foot: vec![Line::from(vec![
            Span::styled("y", theme.style(Tone::Success)),
            Span::styled(" allow once   ", theme.style(Tone::Dim)),
            Span::styled("a", theme.style(Tone::Warning)),
            Span::styled(" allow for session   ", theme.style(Tone::Dim)),
            Span::styled("n", theme.style(Tone::Danger)),
            Span::styled(" deny", theme.style(Tone::Dim)),
        ])],
    };

    draw_modal(
        frame,
        area,
        "approval",
        content.fit(area, theme),
        theme,
        Tone::Danger,
    );
}

fn draw_questionnaire(frame: &mut Frame<'_>, area: Rect, state: &QuestionnaireState, theme: Theme) {
    let form = state.form();
    let question = state.question();
    let mut head = vec![Line::from(vec![
        Span::styled(
            format!("{} answer required  ", glyph::APPROVAL),
            theme.style(Tone::Heading),
        ),
        Span::styled(question.header.clone(), theme.style(Tone::Heading)),
    ])];
    head.extend(
        wrap_text(&question.prompt, usize::from(MIN_WIDTH.saturating_sub(4)))
            .into_iter()
            .map(Line::from),
    );

    let mut detail = vec![Line::from(Span::styled(
        format!(
            "question {} of {}",
            state.current_index() + 1,
            form.questions.len()
        ),
        theme.style(Tone::Accent),
    ))];
    if form.restored {
        detail.push(Line::from(Span::styled(
            "restored pending question",
            theme.style(Tone::Warning),
        )));
    }
    if let Some(error) = state.error() {
        detail.push(Line::from(Span::styled(
            format!("{} {error}", glyph::ERROR),
            theme.style(Tone::Danger),
        )));
    }
    detail.extend([
        Line::from(vec![
            Span::styled("deadline  ", theme.style(Tone::Dim)),
            Span::raw(deadline_text(form.deadline)),
        ]),
        Line::default(),
    ]);

    let mut body = Vec::new();
    for (index, choice) in question.choices.iter().enumerate() {
        let staged = state.staged_choice() == Some(choice.id.as_str());
        let cursor = state.focus() == QuestionnaireFocus::Answer && state.choice_cursor() == index;
        body.push(Line::from(vec![
            Span::styled(
                if cursor { "› " } else { "  " },
                theme.style(if cursor { Tone::Accent } else { Tone::Dim }),
            ),
            Span::styled(
                if staged { "[x] " } else { "[ ] " },
                theme.style(if staged { Tone::Success } else { Tone::Dim }),
            ),
            Span::styled(
                format!("{} {}", index + 1, choice.label),
                theme.style(if cursor { Tone::Accent } else { Tone::Default }),
            ),
        ]));
        if let Some(description) = &choice.description {
            body.push(Line::from(Span::styled(
                format!("      {description}"),
                theme.style(Tone::Dim),
            )));
        }
    }
    if question.allows_free_form {
        let draft = state.displayed_draft();
        let value = if draft.is_empty() {
            "type another answer".to_owned()
        } else if question.sensitive {
            format!("{draft} (masked)")
        } else {
            draft
        };
        body.push(Line::from(vec![
            Span::styled("  other  ", theme.style(Tone::Dim)),
            Span::styled(
                value,
                theme.style(if state.focus() == QuestionnaireFocus::Answer {
                    Tone::Accent
                } else {
                    Tone::Default
                }),
            ),
        ]));
    }

    let mut controls = Vec::new();
    if state.current_index() > 0 {
        controls.push((QuestionnaireFocus::Back, "Back"));
    }
    if state.current_index() + 1 < form.questions.len() {
        controls.push((QuestionnaireFocus::Next, "Next"));
    }
    controls.extend([
        (QuestionnaireFocus::Submit, "Submit"),
        (QuestionnaireFocus::Decline, "Decline"),
    ]);
    let mut action_spans = vec![Span::styled("tab actions  ", theme.style(Tone::Dim))];
    for (index, (focus, label)) in controls.into_iter().enumerate() {
        if index > 0 {
            action_spans.push(Span::styled("  ", theme.style(Tone::Dim)));
        }
        action_spans.push(Span::styled(
            format!("[{label}]"),
            if state.focus() == focus {
                theme
                    .style(Tone::Accent)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                theme.style(Tone::Dim)
            },
        ));
    }
    let foot = vec![
        Line::from(action_spans),
        Line::from(vec![
            Span::styled("enter", theme.style(Tone::Success)),
            Span::styled(" activate   ", theme.style(Tone::Dim)),
            Span::styled("esc", theme.style(Tone::Danger)),
            Span::styled(" cancel", theme.style(Tone::Dim)),
        ]),
    ];

    let content = ModalContent {
        head,
        detail,
        body,
        elided: 0,
        foot,
    };
    draw_modal(
        frame,
        area,
        "questionnaire",
        content.fit(area, theme),
        theme,
        Tone::Accent,
    );
}

fn security_resource_text(resource: &SecurityResource) -> String {
    match resource {
        SecurityResource::Filesystem { mount, segments } => {
            if segments.is_empty() {
                mount.clone()
            } else {
                format!("{}/{}", mount.trim_end_matches('/'), segments.join("/"))
            }
        }
        SecurityResource::Network {
            origin,
            method,
            segments,
        } => {
            let target = if segments.is_empty() {
                origin.clone()
            } else {
                format!("{}/{}", origin.trim_end_matches('/'), segments.join("/"))
            };
            match (method.is_empty(), target.is_empty()) {
                (true, true) => "unrestricted network endpoint".to_owned(),
                (true, false) => target,
                (false, true) => format!("{method} unrestricted network endpoint"),
                (false, false) => format!("{method} {target}"),
            }
        }
        SecurityResource::Credential { reference } => {
            format!("credential:{reference}")
        }
        SecurityResource::Other { kind, id } => format!("{kind}:{id}"),
    }
}

fn authority_warning(prepared: &PreparedToolCall) -> Option<String> {
    let permissions = prepared.required_permissions();
    let mut capabilities = Vec::new();
    if permissions.contains(&Permission::ProcessSpawn) {
        capabilities.push("process execution");
    }
    if permissions.contains(&Permission::FsDelete) {
        capabilities.push("file deletion");
    }
    if matches!(
        prepared.resource(),
        SecurityResource::Filesystem { segments, .. } if segments.is_empty()
    ) && (permissions.contains(&Permission::FsWrite)
        || permissions.contains(&Permission::FsCreate)
        || permissions.contains(&Permission::FsDelete))
    {
        capabilities.push("workspace-root mutation");
    }
    if permissions.contains(&Permission::CredentialUse) {
        capabilities.push("credential use");
    }
    if permissions.contains(&Permission::DataEgress) {
        capabilities.push("data egress");
    }
    if permissions.contains(&Permission::NetHttp) {
        capabilities.push("outbound network access");
    }
    if permissions
        .iter()
        .any(|permission| matches!(permission, Permission::Other(_)))
    {
        capabilities.push("host-defined authority");
    }
    (!capabilities.is_empty()).then(|| format!("authority warning: {}", capabilities.join(", ")))
}

fn deadline_text(deadline: Deadline) -> String {
    let Some(expires) = deadline.instant() else {
        return "no deadline".to_owned();
    };
    let millis = expires.as_millis();
    let local = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(millis) * 1_000_000)
        .map(|instant| {
            instant
                .to_offset(time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC))
        })
        .ok();
    let absolute = local.map_or_else(
        || format!("{millis}ms since epoch"),
        |instant| {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {}",
                instant.year(),
                u8::from(instant.month()),
                instant.day(),
                instant.hour(),
                instant.minute(),
                instant.second(),
                instant.offset()
            )
        },
    );
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    let remaining = millis.saturating_sub(now);
    let status = if remaining == 0 {
        "expired".to_owned()
    } else {
        let seconds = remaining.saturating_add(999) / 1_000;
        if seconds < 120 {
            format!("{seconds}s remaining")
        } else {
            format!("{}m remaining", seconds.saturating_add(59) / 60)
        }
    };
    format!("{absolute} · {status}")
}

/// The diff body: one row per change, each marked with a sign as well as a
/// color, because color is a second channel and never the only one
/// (`DESIGN.md` §4).
fn review_lines(review: &EditReview, theme: Theme) -> (Vec<Line<'static>>, usize) {
    let lines = review
        .changes
        .iter()
        .take(MAX_BODY_LINES)
        .map(|change| match change {
            Change::Context(text) => {
                Line::from(Span::styled(format!("  {text}"), theme.style(Tone::Dim)))
            }
            Change::Removed(text) => Line::from(Span::styled(
                format!("{} {text}", glyph::REMOVED),
                theme.style(Tone::Danger),
            )),
            Change::Added(text) => Line::from(Span::styled(
                format!("{} {text}", glyph::ADDED),
                theme.style(Tone::Success),
            )),
            Change::Skipped(count) => Line::from(Span::styled(
                format!("{} {count} unchanged lines", glyph::ELIDED),
                theme.style(Tone::Dim),
            )),
        })
        .collect();
    (lines, review.changes.len().saturating_sub(MAX_BODY_LINES))
}

/// The fallback body: the raw arguments, exactly as before this modal learned
/// to read edits. Anything that is not a reviewable edit lands here, because a
/// diff that lies about what will change is worse than no diff.
fn argument_lines(arguments: &serde_json::Value, theme: Theme) -> (Vec<Line<'static>>, usize) {
    let arguments =
        serde_json::to_string_pretty(arguments).unwrap_or_else(|_| arguments.to_string());
    let lines: Vec<Line<'static>> = arguments
        .lines()
        .take(MAX_BODY_LINES)
        .map(|raw| Line::from(Span::styled(raw.to_owned(), theme.style(Tone::Dim))))
        .collect();
    (
        lines,
        arguments.lines().count().saturating_sub(MAX_BODY_LINES),
    )
}

/// A modal's content, grouped by what it is willing to lose first.
///
/// A modal is capped at 60% of the terminal's height (`DESIGN.md` §2), so on a
/// short terminal something has to go. The order is not cosmetic: an approval
/// whose keys or whose subject scrolled out of view cannot be answered, and
/// there is no default action to fall back on.
struct ModalContent {
    /// Title and subject. Dropped last, and only if even the keys do not fit.
    head: Vec<Line<'static>>,
    /// Secondary facts: change shape, write scope, the separating blank.
    detail: Vec<Line<'static>>,
    /// The diff, or the raw arguments.
    body: Vec<Line<'static>>,
    /// Body lines that were never built, counted into the elision notice.
    elided: usize,
    /// The action bar, which is never dropped.
    foot: Vec<Line<'static>>,
}

impl ModalContent {
    /// Assembles the content to fit the box it will be drawn in, announcing
    /// whatever it had to leave out — a silently truncated diff is
    /// indistinguishable from a complete one.
    fn fit(self, area: Rect, theme: Theme) -> Vec<Line<'static>> {
        let inner = usize::from(modal_width(area).saturating_sub(2)).max(1);
        let rows = usize::from(modal_max_height(area).saturating_sub(2));

        let budget = rows.saturating_sub(wrapped_rows(&self.foot, inner));
        let (head, _) = fit_rows(self.head, inner, budget);
        let budget = budget.saturating_sub(wrapped_rows(&head, inner));

        // A row is held back for the body, so a modal never spends its last
        // row on a blank separator while the detail it exists to show is gone.
        let reserved = usize::from(!self.body.is_empty());
        let (detail, _) = fit_rows(self.detail, inner, budget.saturating_sub(reserved));
        let budget = budget.saturating_sub(wrapped_rows(&detail, inner));

        let mut hidden = self.elided;
        let body = if hidden == 0 && wrapped_rows(&self.body, inner) <= budget {
            self.body
        } else {
            // One more row is held back for the notice, which is worth more
            // than the line it replaces.
            let (kept, dropped) = fit_rows(self.body, inner, budget.saturating_sub(1));
            hidden += dropped;
            kept
        };

        let mut lines = head;
        lines.extend(detail);
        lines.extend(body);
        if hidden > 0 && budget > 0 {
            lines.push(Line::from(Span::styled(
                format!("{} {hidden} more lines not shown", glyph::ELIDED),
                theme.style(Tone::Warning),
            )));
        }
        lines.extend(self.foot);
        lines
    }
}

/// Keeps whole lines from the front while they fit, reporting how many it
/// dropped.
fn fit_rows(lines: Vec<Line<'static>>, width: usize, budget: usize) -> (Vec<Line<'static>>, usize) {
    let mut used = 0;
    let mut kept = Vec::new();
    let mut dropped = 0;
    for line in lines {
        let rows = line.width().div_ceil(width).max(1);
        // Once one line is dropped the rest go too, so the survivors stay a
        // prefix and the reader is never shown a gap they cannot see.
        if dropped > 0 || used + rows > budget {
            dropped += 1;
            continue;
        }
        used += rows;
        kept.push(line);
    }
    (kept, dropped)
}

/// Rows `lines` occupy once wrapped to `width` columns.
fn wrapped_rows(lines: &[Line<'static>], width: usize) -> usize {
    lines
        .iter()
        .map(|line| line.width().div_ceil(width).max(1))
        .sum()
}

fn draw_palette(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    selected: usize,
    error: Option<&str>,
    theme: Theme,
) {
    let matches = commands::matches(app.composer.text());
    let mut lines = Vec::new();
    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no matching commands",
            theme.style(Tone::Warning),
        )));
    } else {
        let capacity = usize::from(area.height).saturating_sub(usize::from(error.is_some()));
        let visible = capacity.min(matches.len());
        let start = selected
            .saturating_sub(visible / 2)
            .min(matches.len().saturating_sub(visible));
        for (index, command) in matches.into_iter().enumerate().skip(start).take(visible) {
            let marker = if index == selected { "› " } else { "  " };
            let hint = if command.argument_hint.is_empty() {
                String::new()
            } else {
                format!(" {}", command.argument_hint)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{marker}/{}{hint}", command.name),
                    theme.style(if index == selected {
                        Tone::Accent
                    } else {
                        Tone::Default
                    }),
                ),
                Span::styled(format!("  {}", command.description), theme.style(Tone::Dim)),
            ]));
        }
    }
    if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            format!("  {} {error}", glyph::ERROR),
            theme.style(Tone::Danger),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn desired_palette_rows(app: &App, error: Option<&str>) -> u16 {
    let matches = commands::matches(app.composer.text()).len().max(1);
    u16::try_from(matches.saturating_add(usize::from(error.is_some()))).unwrap_or(u16::MAX)
}

fn draw_recovery_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    content: &str,
    theme: Theme,
) {
    let action = if title.starts_with("revert") {
        "apply revert"
    } else {
        "apply undo"
    };
    let mut lines = vec![Line::from(Span::styled(
        "No action is selected by default. Review the complete reverse patch.",
        theme.style(Tone::Warning),
    ))];
    lines.extend(
        content
            .lines()
            .take(MAX_BODY_LINES.saturating_sub(2))
            .map(|line| Line::from(line.to_owned())),
    );
    lines.push(Line::from(vec![
        Span::styled("y", theme.style(Tone::Danger)),
        Span::styled(format!(" {action}   "), theme.style(Tone::Dim)),
        Span::styled("n/esc", theme.style(Tone::Success)),
        Span::styled(" cancel", theme.style(Tone::Dim)),
    ]));
    draw_modal(frame, area, title, lines, theme, Tone::Warning);
}

fn draw_review_confirm(frame: &mut Frame<'_>, area: Rect, content: &str, theme: Theme) {
    let mut lines = content
        .lines()
        .take(MAX_BODY_LINES.saturating_sub(2))
        .map(|line| Line::from(line.to_owned()))
        .collect::<Vec<_>>();
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("y", theme.style(Tone::Accent)),
        Span::styled(" start provider-backed review   ", theme.style(Tone::Dim)),
        Span::styled("n/esc", theme.style(Tone::Success)),
        Span::styled(" cancel", theme.style(Tone::Dim)),
    ]));
    draw_modal(frame, area, "read-only review", lines, theme, Tone::Accent);
}

fn draw_agent_confirm(frame: &mut Frame<'_>, area: Rect, content: &str, theme: Theme) {
    draw_child_continuation_confirm(
        frame,
        area,
        "read-only child agent",
        " start child and spend provider tokens   ",
        content,
        theme,
    );
}

fn draw_child_continuation_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    action: &str,
    content: &str,
    theme: Theme,
) {
    let mut lines = content
        .lines()
        .take(MAX_BODY_LINES.saturating_sub(2))
        .map(|line| Line::from(line.to_owned()))
        .collect::<Vec<_>>();
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("y", theme.style(Tone::Accent)),
        Span::styled(action.to_owned(), theme.style(Tone::Dim)),
        Span::styled("n/esc", theme.style(Tone::Success)),
        Span::styled(" cancel", theme.style(Tone::Dim)),
    ]));
    draw_modal(frame, area, title, lines, theme, Tone::Accent);
}

fn draw_exit_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    pending_approval: bool,
    pending_questionnaire: bool,
    theme: Theme,
) {
    let mut lines = vec![
        Line::from(Span::styled(
            "quit with work in progress?",
            theme.style(Tone::Heading),
        )),
        Line::default(),
    ];
    if app.is_busy() {
        lines.push(Line::from(Span::raw("· a turn is still running")));
    }
    if pending_approval {
        lines.push(Line::from(Span::raw("· an approval is pending")));
    }
    if pending_questionnaire {
        lines.push(Line::from(Span::raw("· a questionnaire is pending")));
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("y", theme.style(Tone::Danger)),
        Span::styled(" quit   ", theme.style(Tone::Dim)),
        Span::styled("n", theme.style(Tone::Success)),
        Span::styled(" keep working", theme.style(Tone::Dim)),
    ]));

    draw_modal(frame, area, "exit", lines, theme, Tone::Warning);
}

/// A modal's width: centered, at most 72 columns (`DESIGN.md` §2).
fn modal_width(area: Rect) -> u16 {
    area.width
        .saturating_sub(4)
        .min(72)
        .max(MIN_WIDTH.min(area.width))
}

/// The tallest a modal may be: 60% of the height (`DESIGN.md` §2), and never
/// more than the viewport, so an overlay cannot spill off screen.
fn modal_max_height(area: Rect) -> u16 {
    (area.height.saturating_mul(3) / 5).max(3).min(area.height)
}

fn draw_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    lines: Vec<Line<'static>>,
    theme: Theme,
    accent: Tone,
) {
    let width = modal_width(area);
    let inner = usize::from(width.saturating_sub(2)).max(1);
    let wanted = u16::try_from(wrapped_rows(&lines, inner) + 2).unwrap_or(u16::MAX);
    let height = wanted.min(modal_max_height(area)).max(3);
    let modal = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    frame.render_widget(Clear, modal);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            WidgetBlock::default()
                .borders(Borders::ALL)
                .border_style(theme.style(accent))
                .title(Span::styled(
                    format!(" {title} "),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
        ),
        modal,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::approval::{ApprovalOrigin, ApprovalPolicy, ApprovalRequest};
    use agent_runtime_core::cancel::CancelReason;
    use agent_runtime_core::clock::{Deadline, SystemClock, Timestamp};
    use agent_runtime_core::content::{ContentPart, Message, ToolCall, ToolResultBlock};
    use agent_runtime_core::event::{
        EstimationConfidence, EventEnvelope, PlanItemProjection, PlanItemStatus, PlanSensitivity,
        RuntimeEvent, TurnFinish,
    };
    use agent_runtime_core::ids::{AttemptId, EventId, RequestId, SessionId, ToolCallId};
    use agent_runtime_core::manifest::SegmentKind;
    use agent_runtime_core::security::{PermissionSet, SecurityResource};
    use agent_runtime_core::tool::{PreparedToolCall, ToolCallDisplay, ToolEffects};
    use agent_runtime_core::usage::{
        CounterKind, Provenance, UsageDelta, UsageRecord, UsageSource,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    use crate::app::Notification;
    use crate::questionnaire::{QuestionnaireChoice, QuestionnaireForm, QuestionnaireQuestion};

    fn render(app: &App, width: u16, height: u16, theme: Theme) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal
            .draw(|frame| draw(frame, app, theme))
            .expect("a frame");
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render_synced(app: &mut App, width: u16, height: u16, theme: Theme) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
        terminal
            .draw(|frame| draw_synced(frame, app, theme))
            .expect("a frame");
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn event(payload: RuntimeEvent) -> EventEnvelope {
        event_at(Timestamp::ZERO, payload)
    }

    fn event_at(timestamp: Timestamp, payload: RuntimeEvent) -> EventEnvelope {
        EventEnvelope::new(
            0,
            EventId::new("e"),
            SessionId::new("s"),
            None,
            timestamp,
            payload,
        )
    }

    fn conversation() -> App {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.transcript.push_user("explain the retry policy");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::TextDelta {
            request: RequestId::new("request-1"),
            attempt: AttemptId::new("attempt-1"),
            text: "The retry policy classifies failures.".into(),
        }));
        app.apply(&event(RuntimeEvent::ProviderAttemptOutputCommitted {
            request: RequestId::new("request-1"),
            attempt: AttemptId::new("attempt-1"),
        }));
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("c1"),
            name: "read".into(),
            argument_keys: vec!["path".into()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        app.set_tool_display(
            "c1",
            smith_tools::project_tool_call_display(
                "read",
                &serde_json::json!({"path": "src/retry.rs"}),
            )
            .expect("reviewed read projection"),
        );
        app.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("c1"),
            name: "read".into(),
            is_error: false,
        }));
        app.apply(&event(RuntimeEvent::ContextPlanned {
            context: agent_runtime_registry::Fingerprint::of("context"),
            cache_plan: agent_runtime_registry::Fingerprint::of("cache"),
            segment_count: 2,
            totals: std::collections::BTreeMap::from([
                (SegmentKind::new("history"), 10_000),
                (SegmentKind::new("tool_schema"), 2_400),
            ]),
            input_tokens: 12_400,
            input_budget_tokens: 100_000,
            reserved_tokens: 28_000,
            confidence: EstimationConfidence::Exact,
        }));
        app.apply(&event(RuntimeEvent::Usage {
            record: UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance::default(),
                delta: UsageDelta::new().with(CounterKind::InputUncached, 12_400),
            },
        }));
        app
    }

    #[test]
    fn a_conversation_renders_its_transcript_composer_and_footer() {
        let app = conversation();
        let screen = render(&app, 74, 16, Theme::new());
        insta_like(
            &screen,
            &[
                "gpt-5.3",
                "87% ctx",
                "› explain the retry policy",
                "• The retry policy classifies failures.",
                "• Read(src/retry.rs) · ok",
                "Ask Smith to do anything",
            ],
        );
    }

    #[test]
    fn command_completion_renders_above_the_composer_without_a_control_strip() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        for character in "bogus".chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let screen = render(&app, 74, 20, Theme::new().without_color());
        insta_like(
            &screen,
            &[
                "no matching commands",
                "› /bogus",
                "unknown command",
                "gpt-5.3",
            ],
        );
        assert!(!screen.contains("tab complete"), "{screen}");
        assert!(!screen.contains("↑↓ select"), "{screen}");
        assert!(!screen.contains("enter run"), "{screen}");
        assert!(!screen.contains("esc close"), "{screen}");
        assert!(
            !screen.contains("command completion"),
            "the Codex-style completion list must not grow a modal title:\n{screen}"
        );
    }

    #[test]
    fn command_completion_keeps_one_identity_footer_row_at_all_widths() {
        for (width, height) in [(44, 14), (74, 24), (120, 32)] {
            let mut app = App::new("gpt-5.3", "~/work/api");
            app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));

            let screen = render(
                &app,
                width,
                height,
                Theme::new().without_color().without_motion(),
            );
            assert!(screen.contains("/help"), "{width}x{height}: {screen}");
            assert!(
                screen
                    .lines()
                    .last()
                    .is_some_and(|line| line.contains("gpt-5.3")),
                "{width}x{height}: {screen}"
            );
            assert!(
                !screen.contains("tab complete"),
                "{width}x{height}: {screen}"
            );
            assert!(!screen.contains("enter run"), "{width}x{height}: {screen}");
            assert!(!screen.contains("esc close"), "{width}x{height}: {screen}");
        }
    }

    #[test]
    fn codex_formatting_uses_quiet_markers_and_semantic_text_styles() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.transcript.push_user("hello");
        app.transcript.push_text_delta(
            "# Heading\nUse `cargo test`, **care**, and [the docs](https://example.com).",
        );
        app.transcript
            .push_reasoning_delta("Checking the workspace.", false);
        app.transcript
            .push_tool_call("c1", "read", None, &["path".to_owned()]);
        app.transcript.complete_tool_call("c1", ToolStatus::Ok);
        app.show_local_result("status", "session: s1\nmodel: gpt-5.3");

        let lines = transcript_lines(&app, Theme::new(), 80);
        let find_line = |needle: &str| {
            lines
                .iter()
                .find(|line| line.spans.iter().any(|span| span.content.contains(needle)))
                .unwrap_or_else(|| panic!("missing `{needle}` in {lines:#?}"))
        };

        let user = find_line("hello");
        assert_eq!(user.spans[0].content, "› ");
        assert!(
            user.spans[0]
                .style
                .add_modifier
                .contains(Modifier::DIM | Modifier::BOLD)
        );
        assert_eq!(user.spans[1].style.fg, None);

        let heading = find_line("Heading");
        assert_eq!(heading.spans[0].content, "• ");
        assert!(heading.spans[0].style.add_modifier.contains(Modifier::DIM));
        assert!(
            heading.spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );

        let prose = find_line("cargo test");
        let code = prose
            .spans
            .iter()
            .find(|span| span.content == "cargo test")
            .expect("inline code span");
        assert_eq!(code.style.fg, Some(Color::Cyan));
        let link = prose
            .spans
            .iter()
            .find(|span| span.content == "the docs")
            .expect("link span");
        assert_eq!(link.style.fg, Some(Color::Cyan));
        assert!(link.style.add_modifier.contains(Modifier::UNDERLINED));

        assert!(
            lines.iter().all(|line| line
                .spans
                .iter()
                .all(|span| !span.content.contains("Checking the workspace."))),
            "closed reasoning must not render as assistant prose: {lines:#?}"
        );

        let tool = find_line("read(path · details unavailable)");
        assert_eq!(tool.spans[0].style.fg, Some(Color::Green));
        assert!(tool.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert!(tool.spans[1].style.add_modifier.contains(Modifier::BOLD));

        let command = find_line("/status");
        assert_eq!(command.spans[0].style.fg, Some(Color::Magenta));
        let border = find_line("╭");
        assert!(border.spans[0].style.add_modifier.contains(Modifier::DIM));
        let status = find_line("session:");
        assert!(status.spans[1].style.add_modifier.contains(Modifier::DIM));
        assert_eq!(status.spans[2].style.fg, None);
    }

    #[test]
    fn working_indicator_replaces_raw_reasoning_until_the_turn_finishes() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::TurnStarted));

        let waiting = render(&app, 74, 12, Theme::new());
        assert!(waiting.contains("Working… · 0s"), "{waiting}");
        assert!(!waiting.contains("plan 0 active"), "{waiting}");
        assert!(!waiting.contains("tools 0 active"), "{waiting}");

        app.transcript
            .push_reasoning_delta("private draft that resembles the answer", false);

        let working = render(&app, 74, 12, Theme::new());
        assert!(working.contains("Working…"), "{working}");
        assert!(
            !working.contains("private draft that resembles the answer"),
            "{working}"
        );

        app.transcript
            .push_notice("monitor", "a background event arrived");
        app.transcript.push_text_delta("The actual visible answer.");
        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        }));
        let answered = render(&app, 74, 12, Theme::new());
        assert!(
            answered.contains("The actual visible answer."),
            "{answered}"
        );
        assert!(!answered.contains("Working…"), "{answered}");
        assert!(answered.contains("turn · completed"), "{answered}");
        assert!(
            !answered.contains("private draft that resembles the answer"),
            "{answered}"
        );
    }

    #[test]
    fn successful_and_non_success_terminals_stay_visible_at_all_widths() {
        for (width, height) in [(44, 14), (74, 24), (120, 32)] {
            let theme = Theme::new().without_color().without_motion();

            let mut completed = App::new("gpt-5.3", "~/work/api");
            completed.apply(&event_at(Timestamp(1_000), RuntimeEvent::TurnStarted));
            completed.transcript.push_text_delta("Committed answer.");
            completed.apply(&event_at(
                Timestamp(1_842),
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ));
            let completed_screen = render(&completed, width, height, theme);
            assert!(
                completed_screen.contains("Committed answer."),
                "{width}x{height}: {completed_screen}"
            );
            assert!(
                completed_screen.contains("turn · completed in 842ms"),
                "{width}x{height}: {completed_screen}"
            );
            assert!(
                !completed_screen.contains("reasoning only")
                    && !completed_screen.contains("Working…"),
                "{width}x{height}: {completed_screen}"
            );

            let mut interrupted = App::new("gpt-5.3", "~/work/api");
            interrupted.apply(&event(RuntimeEvent::TurnStarted));
            interrupted.apply(&event(RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Cancelled {
                    reason: CancelReason::UserRequested,
                },
                visible_output: false,
            }));
            let interrupted_screen = render(&interrupted, width, height, theme);
            assert!(
                interrupted_screen.contains("interrupted"),
                "{width}x{height}: {interrupted_screen}"
            );
        }
    }

    #[test]
    fn tool_only_reasoning_only_and_fallback_states_render_at_all_widths() {
        for (width, height) in [(44, 18), (74, 24), (120, 32)] {
            let theme = Theme::new().without_color().without_motion();

            let mut tool_only = App::new("gpt-5.3", "~/work/api");
            tool_only.apply(&event_at(Timestamp(2_000), RuntimeEvent::TurnStarted));
            tool_only.apply(&event(RuntimeEvent::ToolCallRequested {
                call: ToolCallId::new("search-redacted"),
                name: "search".to_owned(),
                argument_keys: vec!["path".to_owned(), "pattern".to_owned()],
                argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
                arguments: None,
            }));
            tool_only.set_tool_display(
                "search-redacted",
                smith_tools::project_tool_call_display(
                    "search",
                    &serde_json::json!({"pattern": "[redacted]", "path": "src"}),
                )
                .expect("reviewed search projection"),
            );
            tool_only.apply(&event(RuntimeEvent::ToolCallCompleted {
                call: ToolCallId::new("search-redacted"),
                name: "search".to_owned(),
                is_error: false,
            }));
            tool_only.apply(&event_at(
                Timestamp(2_842),
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: false,
                },
            ));
            let tool_screen = render(&tool_only, width, height, theme);
            assert!(
                tool_screen.contains("Search("),
                "{width}x{height}: {tool_screen}"
            );
            assert!(
                tool_screen.contains("[redacted]"),
                "{width}x{height}: {tool_screen}"
            );
            assert!(
                tool_screen.contains(" · ok"),
                "{width}x{height}: {tool_screen}"
            );
            assert!(
                tool_screen.contains("turn · completed in 842ms"),
                "{width}x{height}: {tool_screen}"
            );

            let mut reasoning_only = App::new("gpt-5.3", "~/work/api");
            reasoning_only.apply(&event_at(Timestamp(3_000), RuntimeEvent::TurnStarted));
            reasoning_only
                .transcript
                .push_reasoning_delta("private chain of thought", false);
            reasoning_only.apply(&event_at(
                Timestamp(3_842),
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: false,
                },
            ));
            let reasoning_screen = render(&reasoning_only, width, height, theme);
            assert!(
                reasoning_screen.contains("turn · completed in 842ms"),
                "{width}x{height}: {reasoning_screen}"
            );
            assert!(
                !reasoning_screen.contains("reasoning only"),
                "{reasoning_screen}"
            );
            assert!(
                !reasoning_screen.contains("private chain of thought"),
                "{reasoning_screen}"
            );

            let mut unavailable_duration = App::new("gpt-5.3", "~/work/api");
            unavailable_duration.apply(&event(RuntimeEvent::TurnStarted));
            unavailable_duration.apply(&event(RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: false,
            }));
            let unavailable_screen = render(&unavailable_duration, width, height, theme);
            assert!(
                unavailable_screen.contains("turn · completed"),
                "{unavailable_screen}"
            );
            assert!(
                !unavailable_screen.contains("completed in"),
                "{unavailable_screen}"
            );

            let mut fallback = App::new("gpt-5.3", "~/work/api");
            fallback.apply(&event(RuntimeEvent::ToolCallRequested {
                call: ToolCallId::new("third-party"),
                name: "third_party".to_owned(),
                argument_keys: vec!["path".to_owned()],
                argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
                arguments: None,
            }));
            fallback.apply(&event(RuntimeEvent::ToolCallCompleted {
                call: ToolCallId::new("third-party"),
                name: "third_party".to_owned(),
                is_error: false,
            }));
            let fallback_screen = render(&fallback, width, height, theme);
            assert!(
                fallback_screen.contains("unknown schema"),
                "{fallback_screen}"
            );
            assert!(
                !fallback_screen.contains("values protected"),
                "{fallback_screen}"
            );
        }
    }

    #[test]
    fn speculative_text_is_labelled_draft_until_the_attempt_commits() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::TextDelta {
            request: RequestId::new("request-draft"),
            attempt: AttemptId::new("attempt-draft"),
            text: "tentative answer".into(),
        }));

        let speculative = render(&app, 74, 12, Theme::new());
        assert!(
            speculative.contains("draft · tentative answer"),
            "{speculative}"
        );
        assert!(app.transcript.is_empty());

        app.apply(&event(RuntimeEvent::ProviderAttemptOutputCommitted {
            request: RequestId::new("request-draft"),
            attempt: AttemptId::new("attempt-draft"),
        }));
        let committed = render(&app, 74, 12, Theme::new());
        assert!(committed.contains("tentative answer"), "{committed}");
        assert!(!committed.contains("draft ·"), "{committed}");
    }

    #[test]
    fn command_completion_and_footer_keep_the_codex_color_roles() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));

        {
            let mut terminal = Terminal::new(TestBackend::new(74, 20)).expect("a test terminal");
            terminal
                .draw(|frame| draw(frame, &app, Theme::new()))
                .expect("a frame");
            let buffer = terminal.backend().buffer();
            let row = |y: u16| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            };

            let completion_y = (0..buffer.area.height)
                .find(|y| row(*y).contains("/help"))
                .expect("selected completion row");
            let composer_y = (0..buffer.area.height)
                .find(|y| row(*y).trim_end() == "› /")
                .expect("composer row");
            assert!(
                completion_y < composer_y,
                "completion should sit above the fixed composer"
            );
            let completion = row(completion_y);
            let command_x = u16::try_from(completion.find("/help").expect("command position"))
                .expect("command position fits");
            let description_x = u16::try_from(
                completion
                    .find("list available commands")
                    .expect("description"),
            )
            .expect("description position fits");
            assert_eq!(buffer[(command_x, completion_y)].fg, Color::Cyan);
            assert!(
                buffer[(command_x, completion_y)]
                    .modifier
                    .contains(Modifier::BOLD)
            );
            assert!(
                buffer[(description_x, completion_y)]
                    .modifier
                    .contains(Modifier::DIM)
            );
            assert!(!completion.contains("command completion"));
            assert!(!completion.contains('╭'));
        }

        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let mut terminal = Terminal::new(TestBackend::new(74, 20)).expect("a test terminal");
        terminal
            .draw(|frame| draw(frame, &app, Theme::new()))
            .expect("a frame");
        let buffer = terminal.backend().buffer();
        let footer_y = buffer.area.height - 1;
        let footer = (0..buffer.area.width)
            .map(|x| buffer[(x, footer_y)].symbol())
            .collect::<String>();
        let model_x = u16::try_from(footer.find("gpt-5.3").expect("model in footer"))
            .expect("model position fits");
        let path_x = u16::try_from(footer.find("~/work/api").expect("path in footer"))
            .expect("path position fits");
        assert_eq!(buffer[(model_x, footer_y)].fg, Color::Cyan);
        assert_eq!(buffer[(path_x, footer_y)].fg, Color::Green);
    }

    #[test]
    fn a_multiline_notice_renders_every_line() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.transcript.push_notice(
            "help",
            "/help — list available commands\n/quit — exit Smith",
        );
        let screen = render(&app, 74, 16, Theme::new());
        assert!(
            screen.contains("/help — list available commands"),
            "{screen}"
        );
        assert!(screen.contains("/quit — exit Smith"), "{screen}");
    }

    #[test]
    fn a_tool_row_states_its_outcome_in_words_not_only_color() {
        let app = conversation();
        // Monochrome rendering must still say `ok`.
        let screen = render(&app, 74, 16, Theme::new().without_color());
        assert!(screen.contains("ok"), "{screen}");
        assert!(screen.contains("• Read(src/retry.rs)"), "{screen}");
    }

    #[test]
    fn compact_tool_rows_show_redacted_details_without_results_or_unknown_values() {
        let call_id = ToolCallId::new("search-1");
        let history = vec![
            Message::assistant(vec![ContentPart::ToolCall(ToolCall {
                id: call_id.clone(),
                name: "search".to_owned(),
                arguments: serde_json::json!({
                    "pattern": "TOP_SECRET_PATTERN",
                    "path": "src/\n\u{1b}[31m\u{202e}tests",
                    "unknown": "TOP_SECRET_UNKNOWN"
                }),
            })]),
            Message::tool_result(ToolResultBlock {
                call_id,
                name: "search".to_owned(),
                content: vec![ContentPart::text("TOP_SECRET_RESULT")],
                is_error: false,
            }),
        ];
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.transcript.replace_from_history(&history);
        app.set_tool_display(
            "search-1",
            smith_tools::project_tool_call_display(
                "search",
                &serde_json::json!({
                    "pattern": "[redacted]",
                    "path": "src/\n\u{1b}[31m\u{202e}tests"
                }),
            )
            .expect("reviewed search projection"),
        );
        app.transcript
            .push_tool_call("unknown-1", "third_party", None, &["path".to_owned()]);
        app.transcript
            .complete_tool_call("unknown-1", ToolStatus::Failed);

        let screen = render(&app, 74, 16, Theme::new().without_color());
        assert!(
            screen.contains("• Search(\"[redacted]\" · src/ [31m tests) · ok"),
            "{screen}"
        );
        assert!(
            screen.contains("• third_party(path · unknown schema) · failed"),
            "{screen}"
        );
        assert!(!screen.contains("TOP_SECRET_PATTERN"), "{screen}");
        assert!(!screen.contains("TOP_SECRET_UNKNOWN"), "{screen}");
        assert!(!screen.contains("TOP_SECRET_RESULT"), "{screen}");
        assert!(!screen.contains('\u{1b}'), "{screen:?}");
        assert!(!screen.contains('\u{202e}'), "{screen:?}");
        assert!(!screen.contains(glyph::BRANCH), "{screen}");
    }

    #[test]
    fn an_unknown_context_renders_as_a_question_mark_not_zero() {
        let app = App::new("gpt-5.3", "~/work/api");
        let screen = render(&app, 74, 16, Theme::new());
        assert!(screen.contains("? ctx"), "{screen}");
        assert!(!screen.contains("0 ctx"), "{screen}");
    }

    #[test]
    fn a_tiny_terminal_refuses_rather_than_half_renders() {
        let app = conversation();
        let screen = render(&app, 30, 8, Theme::new());
        assert!(screen.contains("terminal too small"), "{screen}");
        assert!(!screen.contains("retry policy"), "{screen}");
    }

    #[test]
    fn runtime_resource_picker_is_a_bounded_pane_above_the_composer() {
        let mut app = conversation();
        let history_len = app.transcript.blocks().len();
        let entries = (0..12)
            .map(|index| {
                crate::picker::ResourceEntry::new(
                    format!("provider/model-{index:02}"),
                    format!("provider/model-{index:02}"),
                    "trusted limits",
                )
                .active(index == 0)
            })
            .collect();
        app.overlay = Some(Overlay::ResourcePicker {
            picker: crate::picker::ResourcePicker::new("Choose model", entries, "run setup"),
            target: crate::app::ResourceTarget::Model,
            restore_on_escape: "/model".into(),
        });
        let rendered = render(&app, 64, 18, Theme::from_env().without_color());
        assert!(rendered.contains("Choose model"), "{rendered}");
        assert!(rendered.contains("provider/model-00"), "{rendered}");
        assert!(rendered.contains("current"), "{rendered}");
        assert!(rendered.contains("1/12"), "{rendered}");
        assert!(
            rendered.contains("The retry policy classifies failures."),
            "{rendered}"
        );
        assert!(
            !rendered.contains("provider/model-05"),
            "the pane expanded past five results:\n{rendered}"
        );
        assert!(
            !rendered.contains('╭'),
            "runtime choices should not draw a modal border:\n{rendered}"
        );
        let lines = rendered.lines().collect::<Vec<_>>();
        let picker_y = lines
            .iter()
            .position(|line| line.contains("Choose model"))
            .expect("picker row");
        let composer_y = lines
            .iter()
            .position(|line| line.contains("Ask Smith to do anything"))
            .expect("composer row");
        assert!(picker_y < composer_y, "{rendered}");
        assert!(
            composer_y - picker_y <= 7,
            "pane grew too tall:\n{rendered}"
        );
        assert_eq!(
            app.transcript.blocks().len(),
            history_len,
            "picker metadata entered canonical history"
        );
    }

    #[test]
    fn unified_reference_picker_labels_files_and_agents_without_color() {
        let mut app = App::new("glm-5.2", "/Volumes/Data/codes/ai/agent-runtime:main");
        app.status.switch_model(Some("zai".to_owned()), "glm-5.2");
        app.status.set_agent("build");
        app.set_resources(crate::app::RuntimeResources {
            files: vec![crate::picker::ResourceEntry::new(
                "file:src/lib.rs",
                "@src/lib.rs",
                "file · 42 bytes",
            )],
            child_agents: vec![crate::picker::ResourceEntry::new(
                "agent:review",
                "@review",
                "agent · read-only child preset",
            )],
            ..crate::app::RuntimeResources::default()
        });
        let before = render(&app, 120, 24, Theme::new().without_color().without_motion());
        let before_lines = before.lines().collect::<Vec<_>>();
        let before_identity = before_lines
            .iter()
            .position(|line| line.contains("build · zai/glm-5.2"))
            .expect("idle identity row");
        let before_composer = before_lines
            .iter()
            .position(|line| line.contains("Ask Smith to do anything"))
            .expect("idle composer row");

        app.on_key(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE));

        let screen = render(&app, 120, 24, Theme::new().without_color().without_motion());
        insta_like(
            &screen,
            &[
                "Attach file or invoke agent",
                "@review",
                "agent · read-only",
                "@src/lib.rs",
                "file · 42 bytes",
                "build · zai/glm-5.2 · /Volumes/Data/codes/ai/agent-runtime:main · ? ctx",
                "type to filter · ↑↓ choose · enter confirm · esc cancel",
            ],
        );
        let open_lines = screen.lines().collect::<Vec<_>>();
        let open_identity = open_lines
            .iter()
            .position(|line| line.contains("build · zai/glm-5.2"))
            .expect("picker identity row");
        let open_composer = open_lines
            .iter()
            .position(|line| line.contains("Ask Smith to do anything"))
            .expect("picker composer row");
        assert_eq!(
            open_identity.saturating_add(1),
            before_identity,
            "picker controls should reserve exactly one temporary footer row:\n{screen}"
        );
        assert_eq!(
            open_composer.saturating_add(1),
            before_composer,
            "picker controls should move the composer by exactly one row:\n{screen}"
        );
    }

    #[test]
    fn compact_picker_replaces_todo_pane_with_one_temporary_control_row() {
        let mut app = App::new("glm-5.2", "api:main");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::PlanUpdated {
            revision: 1,
            sensitivity: PlanSensitivity::Public,
            counts: std::collections::BTreeMap::from([
                ("cancelled".to_owned(), 0),
                ("completed".to_owned(), 1),
                ("in_progress".to_owned(), 1),
                ("pending".to_owned(), 1),
            ]),
            items: Some(vec![
                PlanItemProjection {
                    id: "inspect".to_owned(),
                    text: "Inspect relevant code".to_owned(),
                    status: PlanItemStatus::Completed,
                    reason: None,
                },
                PlanItemProjection {
                    id: "change".to_owned(),
                    text: "Implement the change".to_owned(),
                    status: PlanItemStatus::InProgress,
                    reason: None,
                },
                PlanItemProjection {
                    id: "verify".to_owned(),
                    text: "Run focused tests".to_owned(),
                    status: PlanItemStatus::Pending,
                    reason: None,
                },
            ]),
        }));
        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        }));
        app.set_resources(crate::app::RuntimeResources {
            files: vec![crate::picker::ResourceEntry::new(
                "file:src/lib.rs",
                "@src/lib.rs",
                "file · 42 bytes",
            )],
            ..crate::app::RuntimeResources::default()
        });

        let theme = Theme::new().without_color().without_motion();
        let before = render(&app, 80, 14, theme);
        insta_like(
            &before,
            &[
                "Todo",
                "[x] Inspect relevant code",
                "[>] Implement the change",
                "[ ] Run focused tests",
            ],
        );
        assert!(!before.contains("work ·"), "{before}");
        assert!(!before.contains("plan 0 active"), "{before}");
        let before_lines = before.lines().collect::<Vec<_>>();
        let before_composer = before_lines
            .iter()
            .position(|line| line.contains("Ask Smith to do anything"))
            .expect("composer row");

        app.on_key(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE));
        let open = render(&app, 80, 14, theme);
        let open_lines = open.lines().collect::<Vec<_>>();
        let picker = open_lines
            .iter()
            .position(|line| line.contains("Attach file or invoke agent"))
            .expect("picker row");
        let open_composer = open_lines
            .iter()
            .position(|line| line.contains("Ask Smith to do anything"))
            .expect("open composer row");
        assert!(picker < open_composer, "{open}");
        assert!(!open.contains("Todo"), "{open}");
        assert!(!open.contains("Inspect relevant code"), "{open}");
        assert!(!open.contains("Implement the change"), "{open}");
        assert!(!open.contains("Run focused tests"), "{open}");
        assert_eq!(
            open_composer.saturating_add(1),
            before_composer,
            "picker controls should move the composer by exactly one row:\n{open}"
        );

        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let closed = render(&app, 80, 14, theme);
        insta_like(
            &closed,
            &[
                "Todo",
                "[x] Inspect relevant code",
                "[>] Implement the change",
                "[ ] Run focused tests",
            ],
        );
    }

    #[test]
    fn a_narrow_footer_keeps_the_model_and_drops_detail() {
        let app = conversation();
        let screen = render(&app, 44, 14, Theme::new());
        let footer = screen.lines().last().unwrap_or_default();
        assert!(footer.contains("gpt-5.3"), "{footer}");
        assert!(
            footer.width() <= 44,
            "the footer must not overflow: {footer}"
        );
    }

    #[test]
    fn agent_first_idle_snapshots_are_accessible_at_supported_sizes() {
        let mut app = App::new("glm-5.2", "api:main");
        app.status.switch_model(Some("zai".to_owned()), "glm-5.2");
        app.status.set_agent("build");
        let theme = Theme::new().without_color().without_motion();

        for (width, height) in [(44, 14), (74, 24), (120, 32)] {
            let screen = render(&app, width, height, theme);
            assert!(screen.contains("build"), "{width}×{height}:\n{screen}");
            assert!(screen.contains("glm-5.2"), "{width}×{height}:\n{screen}");
            assert!(screen.contains("? ctx"), "{width}×{height}:\n{screen}");
            assert!(
                !screen.contains("Tab agents"),
                "{width}×{height}:\n{screen}"
            );
            assert!(
                !screen.contains("Ctrl+P commands"),
                "{width}×{height}:\n{screen}"
            );
            assert!(
                screen
                    .lines()
                    .all(|line| line.width() <= usize::from(width)),
                "{width}×{height} overflowed:\n{screen}"
            );
            assert!(!screen.contains('\u{1b}'), "ANSI leaked:\n{screen:?}");
            assert_eq!(
                screen.matches("zai/glm-5.2").count(),
                1,
                "identity became permanent chrome:\n{screen}"
            );
        }

        let normal = render(&app, 74, 24, theme);
        insta_like(&normal, &["build · zai/glm-5.2 · api:main · ? ctx"]);

        app.apply(&event(RuntimeEvent::TurnStarted));
        let reduced_motion = render(&app, 74, 24, theme);
        assert!(reduced_motion.contains("Working"), "{reduced_motion}");
        assert!(
            reduced_motion.contains(glyph::STILL),
            "reduced motion did not use the static activity marker:\n{reduced_motion}"
        );
    }

    #[test]
    fn resizing_recomputes_the_layout_without_corrupting_history() {
        let app = conversation();
        for (width, height) in [(74, 16), (44, 12), (120, 40), (74, 16)] {
            let screen = render(&app, width, height, Theme::new());
            assert!(
                screen.contains("gpt-5.3"),
                "{width}×{height} lost the footer:\n{screen}"
            );
        }
    }

    #[test]
    fn the_paused_follow_indicator_appears_only_when_scrolled_back() {
        let mut app = conversation();
        assert!(!render_synced(&mut app, 74, 10, Theme::new()).contains("following paused"));
        app.scroll_up(3);
        let paused = render_synced(&mut app, 74, 10, Theme::new());
        assert!(paused.contains("following paused"), "{paused}");
        assert!(paused.contains("End/Ctrl+L newest"), "{paused}");
    }

    #[test]
    fn an_empty_transcript_cannot_enter_a_phantom_scroll_state() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        let _ = render_synced(&mut app, 74, 16, Theme::new());

        app.on_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        let screen = render_synced(&mut app, 74, 16, Theme::new());

        assert!(app.following);
        assert_eq!(app.scroll_back, 0);
        assert!(!screen.contains("following paused"), "{screen}");
    }

    #[test]
    fn a_notification_appears_inline_in_the_transcript() {
        let mut app = conversation();
        assert!(!render(&app, 74, 24, Theme::new()).contains("monitor:build"));

        app.notify(Notification {
            source: "monitor:build".into(),
            text: "error[E0433]".into(),
            terminal: false,
        });
        assert!(render(&app, 74, 24, Theme::new()).contains("monitor:build"));
    }

    #[test]
    fn local_results_render_inline_across_supported_sizes() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.show_local_result(
            "diff · all uncommitted",
            "No changes in this scope.\nBinary file exists; content omitted.",
        );
        assert!(app.overlay.is_none());
        for (width, height) in [(44, 12), (74, 20), (120, 30)] {
            let screen = render(&app, width, height, Theme::new().without_color());
            assert!(screen.contains("/diff · all uncommitted"), "{screen}");
            assert!(screen.contains("No changes"), "{screen}");
            assert!(screen.contains("Binary file"), "{screen}");
            assert!(screen.contains("›"), "{screen}");
        }
    }

    #[test]
    fn wrapped_local_result_continuations_keep_the_content_indent() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.show_local_result("status", format!("session: {}", "x".repeat(80)));
        let screen = render(&app, 44, 14, Theme::new().without_color());
        assert!(
            screen.lines().all(|line| !line.starts_with('x')),
            "a wrapped continuation escaped the local-result indent:\n{screen}"
        );
        assert!(
            screen.lines().filter(|line| line.starts_with('│')).count() >= 2,
            "{screen}"
        );
    }

    #[test]
    fn context_status_stays_bounded_across_supported_widths() {
        let mut app = App::new("glm-4.7", "~/work/api");
        app.show_local_result(
            "status",
            "context window: ~98% input left (~1.1k used / 68.9k budget)\n\
             model window: 200k total · 131k reserved\n\
             context plan: estimated · 8 segments\n\
               system instruction: ~21\n\
               tool schema: ~1.1k\n\
               user input: ~12\n\
             provider input (session): 1.3k",
        );

        for (width, height) in [(44, 30), (74, 24), (120, 24)] {
            let screen = render(&app, width, height, Theme::new().without_color());
            assert!(screen.contains("/status"), "{width}×{height}:\n{screen}");
            assert!(screen.contains("~98% input"), "{width}×{height}:\n{screen}");
            assert!(
                screen
                    .lines()
                    .all(|line| line.width() <= usize::from(width)),
                "{width}×{height} overflowed:\n{screen}"
            );
        }
    }

    #[test]
    fn focused_context_view_keeps_the_grid_and_legend_inline() {
        let mut app = App::new("glm-4.7", "~/work/api");
        app.show_local_result(
            "context",
            "Context usage\n\
             glm-4.7 · ~2k / 123.9k input tokens · ~98% left\n\n\
             ■ ■ ◆ ● · · · · □ □\n\
             · · · · · · · · □ □\n\
             · · · · · · · · □ □\n\
             · · · · · · · · □ □\n\
             · · · · · · · · □ □\n\n\
             Estimated usage by category\n\
             ■ system instructions: ~20 (0.1%)\n\
             ◆ tool schemas: ~500 (0.4%)\n\
             ● history: ~1.5k (1.2%)\n\
             · free input: ~121.9k (98.4%)\n\
             □ output/reasoning reserve: 4k (3.1%)\n\
             counting: estimated · 4 segments\n\
             compaction: enabled on overflow · 74.3k recovery target",
        );

        for (width, height) in [(44, 28), (74, 24), (120, 24)] {
            let screen = render(&app, width, height, Theme::new().without_color());
            assert!(screen.contains("/context"), "{width}×{height}:\n{screen}");
            assert!(
                screen.contains("Estimated usage by category"),
                "{width}×{height}:\n{screen}"
            );
            assert!(
                screen.lines().any(|line| line.contains("■ ■ ◆ ● ·")),
                "{width}×{height}:\n{screen}"
            );
            assert!(
                screen
                    .lines()
                    .all(|line| line.width() <= usize::from(width)),
                "{width}×{height} overflowed:\n{screen}"
            );
        }
    }

    #[test]
    fn empty_error_and_oversized_local_results_name_their_state() {
        let mut empty = App::new("gpt-5.3", "~/work/api");
        empty.show_local_empty("agents", "");
        let empty_screen = render(&empty, 74, 12, Theme::new().without_color());
        assert!(empty_screen.contains("/agents"), "{empty_screen}");
        assert!(empty_screen.contains("• No output."), "{empty_screen}");
        assert!(empty_screen.contains("No output."), "{empty_screen}");

        let mut error = App::new("gpt-5.3", "~/work/api");
        error.show_local_error("diff", "Git inspection is unavailable.");
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
        oversized.show_local_result("diff", "x".repeat(MAX_LOCAL_RESULT_BYTES + 1));
        let oversized_screen = render(&oversized, 74, 12, Theme::new().without_color());
        assert!(
            oversized_screen.contains("[local result truncated at the display limit]"),
            "{oversized_screen}"
        );
    }

    #[test]
    fn recovery_and_review_modals_name_the_action_without_a_default() {
        let mut undo = App::new("gpt-5.3", "~/work/api");
        undo.confirm_undo("--- current\n+++ restore\n-old\n+new");
        let undo_screen = render(&undo, 74, 20, Theme::new().without_color());
        assert!(undo_screen.contains("No action is selected by default"));
        assert!(undo_screen.contains("apply undo"));

        let mut review = App::new("gpt-5.3", "~/work/api");
        review.confirm_review(
            "all",
            "provider-backed: yes\nworkspace authority: read-only",
        );
        let review_screen = render(&review, 74, 20, Theme::new().without_color());
        assert!(review_screen.contains("read-only review"));
        assert!(review_screen.contains("provider-backed: yes"));
    }

    #[test]
    fn child_follow_up_and_resume_confirmations_are_clear_without_color_at_supported_sizes() {
        for (width, height) in [(44, 16), (74, 20), (120, 28)] {
            let mut follow_up = App::new("glm-5.2", "~/work/api");
            follow_up.overlay = Some(Overlay::AgentFollowUpConfirm {
                child_id: "child-1".to_owned(),
                task: "check the parser".to_owned(),
                content: "child: child-1\noperation: new follow-up turn\ncontinuity: reuse prior child history\nprovider spend: yes".to_owned(),
            });
            let follow_up_screen = render(&follow_up, width, height, Theme::new().without_color());
            assert!(
                follow_up_screen.contains("existing child follow-up"),
                "{width}×{height}:\n{follow_up_screen}"
            );
            assert!(
                follow_up_screen.contains("new follow-up"),
                "{width}×{height}:\n{follow_up_screen}"
            );

            let mut resume = App::new("glm-5.2", "~/work/api");
            resume.overlay = Some(Overlay::AgentResumeConfirm {
                child_id: "child-1".to_owned(),
                content: "child: child-1\noperation: continue exact interrupted checkpoint\nturn slot consumed: no\nside effects: committed work is not replayed".to_owned(),
            });
            let resume_screen = render(&resume, width, height, Theme::new().without_color());
            assert!(
                resume_screen.contains("resume interrupted child"),
                "{width}×{height}:\n{resume_screen}"
            );
            assert!(
                resume_screen.contains("exact interrupted"),
                "{width}×{height}:\n{resume_screen}"
            );
        }
    }

    /// Drives the real approval policy to obtain a real prompt.
    async fn prompt(
        tool: &str,
        arguments: serde_json::Value,
    ) -> smith_host::approval::ApprovalPrompt {
        use smith_host::approval::InteractiveApproval;

        let (policy, mut requests) = InteractiveApproval::new(1);
        let tool = tool.to_owned();
        tokio::spawn(async move {
            let effects = if tool == "shell" {
                ToolEffects::read_only()
                    .with_write("/repo")
                    .with_spawn()
                    .with_network()
            } else {
                ToolEffects::read_only().with_write("/repo")
            };
            let (permissions, _) = effects.authorization_request(&tool, "/repo");
            let segments = arguments
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map(|path| {
                    path.split('/')
                        .filter(|segment| !segment.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let request = ApprovalRequest::new(
                PreparedToolCall::new(
                    ToolCallId::new("c1"),
                    &tool,
                    arguments,
                    permissions,
                    SecurityResource::filesystem("/repo", segments),
                    effects,
                    ToolCallDisplay::new(format!("Run {tool}")),
                ),
                Deadline::after(&SystemClock, 60_000),
                ApprovalOrigin::new(
                    agent_runtime_core::ids::SessionId::new("session-1"),
                    agent_runtime_core::ids::RequestId::new("request-1"),
                ),
            );
            let _ = policy.decide(&request).await;
        });
        requests.recv().await.expect("a prompt")
    }

    /// An approval waiting on an `edit` of `src/retry.rs`.
    async fn edit_approval(old: &str, new: &str) -> App {
        let mut app = conversation();
        app.present_approval(
            prompt(
                "edit",
                serde_json::json!({
                    "path": "src/retry.rs",
                    "old_string": old,
                    "new_string": new,
                }),
            )
            .await,
        );
        app
    }

    #[tokio::test]
    async fn the_approval_modal_names_the_tool_and_its_keys() {
        let mut app = conversation();
        app.present_approval(prompt("shell", serde_json::json!({"command": "rm -rf build"})).await);
        let screen = render(&app, 74, 24, Theme::new());

        insta_like(
            &screen,
            &[
                "approval required",
                "shell",
                "rm -rf build",
                "process execution",
                "deadline",
                "fingerprint",
                "y allow once",
            ],
        );
    }

    #[test]
    fn approval_warnings_follow_typed_authority_not_only_scheduler_effects() {
        let prepared = PreparedToolCall::new(
            ToolCallId::new("sensitive-call"),
            "broker",
            serde_json::json!({"reference": "provider"}),
            [
                Permission::CredentialUse,
                Permission::DataEgress,
                Permission::FsDelete,
            ]
            .into_iter()
            .collect::<PermissionSet>(),
            SecurityResource::credential("provider"),
            ToolEffects::new(Vec::new()),
            ToolCallDisplay::new("Use a protected broker"),
        );

        let warning = authority_warning(&prepared).expect("sensitive authority warning");
        assert!(warning.contains("credential use"), "{warning}");
        assert!(warning.contains("data egress"), "{warning}");
        assert!(warning.contains("file deletion"), "{warning}");
    }

    #[tokio::test]
    async fn an_edit_approval_shows_a_diff_instead_of_raw_json() {
        let app = edit_approval(
            "fn retry() {\n    once();\n}\n",
            "fn retry(limit: u32) {\n    once();\n}\n",
        )
        .await;
        let screen = render(&app, 74, 24, Theme::new());

        insta_like(
            &screen,
            &[
                "approval required",
                "src/retry.rs",
                "1 removed · 1 added",
                "- fn retry() {",
                "+ fn retry(limit: u32) {",
                "    once();",
                "y allow once",
            ],
        );
        assert!(
            !screen.contains("old_string"),
            "the raw arguments must give way to the diff:\n{screen}"
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
    async fn a_non_edit_approval_falls_back_to_its_arguments() {
        let mut app = conversation();
        app.present_approval(
            prompt(
                "shell",
                serde_json::json!({"command": "rm -rf build", "cwd": "/repo"}),
            )
            .await,
        );
        let screen = render(&app, 74, 24, Theme::new());

        insta_like(&screen, &["\"command\"", "rm -rf build"]);
        assert!(
            !screen.contains("change  "),
            "a shell call has no diff to summarize:\n{screen}"
        );
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

        insta_like(&screen, &["\"old_string\"", "y allow once"]);
        assert!(
            !screen.contains("change  "),
            "an unreviewable edit must not claim a diff:\n{screen}"
        );
    }

    #[tokio::test]
    async fn a_diff_too_tall_for_the_modal_says_how_much_it_hid() {
        let old: String = (0..60).map(|n| format!("let x{n} = {n};\n")).collect();
        let new = old.replace("let x", "let y");
        let app = edit_approval(&old, &new).await;
        let screen = render(&app, 74, 24, Theme::new());

        insta_like(&screen, &["more lines not shown", "y allow once"]);
    }

    #[tokio::test]
    async fn a_change_buried_in_context_still_reaches_the_top_of_the_modal() {
        let old: String = (0..20).map(|n| format!("let x{n} = {n};\n")).collect();
        let new = old.replace("let x10 = 10;", "let x10 = 11;");
        let app = edit_approval(&old, &new).await;
        let screen = render(&app, 74, 40, Theme::new());

        // The collapsed context is counted, not silently dropped.
        insta_like(
            &screen,
            &[
                "unchanged lines",
                "- let x10 = 10;",
                "+ let x10 = 11;",
                "y allow once",
            ],
        );
    }

    #[tokio::test]
    async fn a_short_terminal_still_renders_an_answerable_modal() {
        let app = edit_approval("once();\n", "twice();\n").await;
        for (width, height) in [(MIN_WIDTH, MIN_HEIGHT), (44, 12), (52, 14)] {
            let screen = render(&app, width, height, Theme::new());
            insta_like(&screen, &["approval required", "src/retry.rs"]);
            assert!(
                screen.contains("allow") && screen.contains("deny"),
                "{width}×{height} left the approval unanswerable:\n{screen}"
            );
            for line in screen.lines() {
                assert!(
                    line.width() <= usize::from(width),
                    "{width}×{height} overflowed the viewport:\n{screen}"
                );
            }
            assert!(
                screen.lines().count() <= usize::from(height),
                "{width}×{height} overflowed the viewport:\n{screen}"
            );
        }
    }

    #[test]
    fn questionnaire_is_answerable_when_narrow_and_masks_sensitive_drafts() {
        let mut app = conversation();
        let form = QuestionnaireForm::new(
            "interaction-1",
            vec![
                QuestionnaireQuestion::new(
                    "token",
                    "Credential",
                    "Which secret token should be used?",
                    vec![QuestionnaireChoice::new("configured", "Configured token")],
                )
                .with_free_form(true),
            ],
            Deadline::never(),
        )
        .expect("valid questionnaire")
        .restored(true);
        app.present_questionnaire(form);
        for character in "supersecret".chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        let normal = render(&app, 74, 24, Theme::new().without_color());
        insta_like(
            &normal,
            &[
                "answer required",
                "Which secret token should be used?",
                "restored pending question",
                "(masked)",
                "[Submit]",
                "esc cancel",
            ],
        );
        assert!(!normal.contains("supersecret"), "{normal}");

        let narrow = render(&app, MIN_WIDTH, MIN_HEIGHT, Theme::new().without_color());
        insta_like(
            &narrow,
            &[
                "answer required",
                "Which secret token should be used?",
                "Submit",
                "cancel",
            ],
        );
        assert!(!narrow.contains("supersecret"), "{narrow}");
        assert!(
            narrow
                .lines()
                .all(|line| line.width() <= usize::from(MIN_WIDTH)),
            "{narrow}"
        );
    }

    /// Asserts every expected fragment appears, reporting the whole screen on
    /// failure so a broken layout is readable rather than a boolean.
    fn insta_like(screen: &str, expected: &[&str]) {
        for fragment in expected {
            assert!(
                screen.contains(fragment),
                "expected to find `{fragment}` in:\n{screen}"
            );
        }
    }
}
