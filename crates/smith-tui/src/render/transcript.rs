//! Transcript, Markdown, tool, status, and local-result rendering.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, ProviderPhase};
use crate::status::{Activity, render_elapsed};
use crate::theme::{Theme, Tone, glyph};
use crate::transcript::{Block, LocalResult, LocalResultState, ToolStatus};
use smith_client::agent_report::{AgentReport, AgentResumeReport, AgentSnapshot};
use smith_client::context_report::{ContextCategoryKind, ContextCompaction, ContextReport};
use smith_client::diff_report::{DiffLineKind, DiffOutcome, DiffReport};
use smith_client::goal_report::GoalReport;
use smith_client::help_report::{HelpCommand, HelpReport};
use smith_client::mcp_report::McpReport;
use smith_client::review_report::{ReviewPreview, ReviewReport, ReviewStartReport};
use smith_client::skills_report::SkillsReport;
use smith_client::status_report::{StatusGoal, StatusReport};
use smith_client::timeline_report::{TimelineEntry, TimelinePlan, TimelineReport};
use smith_tools::ToolCallDisplay;

use super::helpers::*;
use super::wrap::{wrap_lines, wrapped_row_count};

pub(super) fn draw_transcript(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    lines: Option<Vec<Line<'static>>>,
) {
    let lines = lines.unwrap_or_else(|| transcript_lines(app, theme, area.width));
    let max_scroll = visual_scroll_limit(&lines, area);
    let pending_offset = app
        .scroll_to_block
        .filter(|_| app.inspected_child.is_none())
        .and_then(|block| block_start_row(app, block, theme, area.width));
    let offset = if let Some(offset) = pending_offset {
        offset.min(max_scroll)
    } else if app.following {
        max_scroll
    } else {
        max_scroll.saturating_sub(app.scroll_back)
    };

    let paragraph = Paragraph::new(wrap_lines(&lines, area.width)).scroll((offset, 0));
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
    wrapped_row_count(lines, width)
}

/// Find a local result's start using the same block suppression and wrapping
/// as the transcript, including the separator after preceding visible blocks.
pub(super) fn block_start_row(app: &App, block: usize, theme: Theme, width: u16) -> Option<u16> {
    let blocks = app.transcript.blocks();
    if block >= blocks.len() {
        return None;
    }
    let preceding = block_lines(&blocks[..block], theme, width);
    let rows = rendered_rows(&preceding, width) + usize::from(!preceding.is_empty());
    Some(u16::try_from(rows).unwrap_or(u16::MAX))
}

pub(super) fn transcript_lines(app: &App, theme: Theme, width: u16) -> Vec<Line<'static>> {
    // The inspector borrows the transcript region rather than floating over
    // it: a child's history is read, scrolled, and selected exactly like the
    // root timeline, and one Esc gives the region back unchanged.
    if let Some(child) = &app.inspected_child {
        return child_lines(app, child, theme, width);
    }
    if app.transcript.is_empty()
        && app.status.activity == Activity::Idle
        && !app.has_live_work()
        && app.live_child_count() == 0
        && app.speculative_text().is_none()
        && app.turn_summary.is_none()
    {
        return getting_started_lines(theme);
    }
    let mut lines = block_lines(app.transcript.blocks(), theme, width);

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
        let working_label = match app.status.activity {
            Activity::Working => "Working",
            Activity::Interrupting => "Interrupting",
            Activity::Idle | Activity::ParkedAwaitingChild | Activity::Ended => {
                unreachable!("activity was filtered above")
            }
        };
        let details = app.work_detail_lines();
        let retry = app.provider_retry();
        let label = retry
            .map(|retry| format!("Retrying {}/{}", retry.next_attempt, retry.max_attempts))
            .unwrap_or_else(|| working_label.to_owned());
        // The provider round-trip stage answers "is anything happening?"
        // during an otherwise silent wait: `↑ 45s` is a stall the user can
        // see, where a bare `Working…` looks identical to progress. The
        // transfer phases read as direction; only thinking keeps its word.
        let phase = match retry.and_then(|retry| retry.backoff_remaining) {
            Some(remaining) => format!(" · backoff {}", render_retry_backoff(remaining),),
            None => app
                .provider_phase()
                .map(|(phase, elapsed)| {
                    let marker = match phase {
                        ProviderPhase::Sending => glyph::SENDING,
                        ProviderPhase::Thinking => "thinking",
                        ProviderPhase::Responding => glyph::RECEIVING,
                    };
                    format!(" · {marker} {}", render_elapsed(elapsed))
                })
                .unwrap_or_default(),
        };
        // Delegated work is part of "is anything happening?": a silent
        // parent waiting on children would otherwise look stalled.
        let agents = match app.live_child_count() {
            0 => String::new(),
            1 => " · 1 agent".to_owned(),
            count => format!(" · {count} agents"),
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", theme.spinner(app.tick)),
                theme.style(Tone::Accent),
            ),
            Span::styled(
                format!(
                    "{label}{} · {}{phase}{agents}{}",
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

fn render_retry_backoff(remaining: Duration) -> String {
    let millis = remaining.as_millis();
    if millis < 1_000 {
        "<1s".to_owned()
    } else {
        format!("{}s", millis.saturating_add(999) / 1_000)
    }
}

fn getting_started_lines(theme: Theme) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled("  Get started", theme.style(Tone::Heading))),
        Line::from(Span::styled(
            "  Type a task below and press Enter.",
            theme.style(Tone::Default),
        )),
        Line::default(),
    ];
    for (command, description) in [
        ("/model", "Choose a model"),
        ("/connect", "Add a connection"),
        ("/help", "Explore commands"),
    ] {
        lines.push(Line::from(vec![
            Span::styled(format!("  {command:<10}"), theme.style(Tone::Code)),
            Span::styled(description, theme.style(Tone::Dim)),
        ]));
    }
    lines
}

/// Every transcript block as rendered rows.
///
/// The root timeline and a delegated child's history both come through
/// here. A child is an agent that reports back, not a different kind of
/// thing, so it must not get a second, thinner renderer that drifts from
/// this one — whatever the runtime chooses to report about it lands in the
/// same blocks and draws the same way.
fn block_lines(blocks: &[Block], theme: Theme, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for block in blocks {
        // Reasoning is canonical model state, not a second assistant answer.
        // The turn-level working row below represents progress without
        // exposing raw provider reasoning as transcript prose.
        if matches!(block, Block::Reasoning { .. }) {
            continue;
        }
        // A row whose effect a better surface already reports is dropped
        // whole, before the blank-line separator below ever considers it —
        // that is what keeps a suppression from leaving a doubled or
        // leading blank line, and it drops the result preview for free,
        // since the preview lives inside this same block.
        if let Block::Tool {
            name,
            display,
            status,
            ..
        } = block
            && is_redundant_tool_row(name, *status, display.as_ref())
        {
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
                started_at,
                enrichment,
                ..
            } => {
                let tone = match status {
                    ToolStatus::Running | ToolStatus::Unreported => Tone::Dim,
                    ToolStatus::Ok => Tone::Success,
                    ToolStatus::Failed | ToolStatus::Denied => Tone::Danger,
                };
                let invocation =
                    tool_invocation(name, display.as_ref(), enrichment, protected_summary);
                let status_text = match status {
                    ToolStatus::Running => {
                        if let Some(started) = started_at {
                            format!("running {}", render_elapsed(started.elapsed()))
                        } else {
                            status.label().to_owned()
                        }
                    }
                    _ => status.label().to_owned(),
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{} ", glyph::TOOL),
                        theme.style(tone).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(invocation, theme.style(Tone::Heading)),
                    Span::styled(" · ", theme.style(Tone::Dim)),
                    Span::styled(status_text, theme.style(tone)),
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
            Block::Local(LocalResult::Status(report)) => {
                lines.push(Line::from(Span::styled(
                    "/status",
                    theme.style(Tone::Command),
                )));
                lines.extend(render_status_card(report, width, theme));
            }
            Block::Local(LocalResult::Context(report)) => {
                lines.push(Line::from(Span::styled(
                    "/context",
                    theme.style(Tone::Command),
                )));
                lines.extend(render_context_report(report, width, theme));
            }
            Block::Local(LocalResult::Help(report)) => {
                lines.push(Line::from(Span::styled(
                    "/help",
                    theme.style(Tone::Command),
                )));
                lines.extend(render_help_report(report, width, theme));
            }
            Block::Local(LocalResult::Timeline(report)) => {
                lines.push(Line::from(Span::styled(
                    "/timeline",
                    theme.style(Tone::Command),
                )));
                lines.extend(render_timeline_report(report, width, theme));
            }
            Block::Local(LocalResult::Goal(report)) => {
                lines.push(Line::from(Span::styled(
                    "/goal",
                    theme.style(Tone::Command),
                )));
                lines.extend(render_goal_report(report, width, theme));
            }
            Block::Local(LocalResult::Agent(report)) => {
                lines.extend(render_agent_report(report, width, theme));
            }
            Block::Local(LocalResult::Mcp(report)) => {
                lines.push(Line::from(Span::styled("/mcp", theme.style(Tone::Command))));
                lines.extend(render_mcp_report(report, width, theme));
            }
            Block::Local(LocalResult::Skills(report)) => {
                lines.push(Line::from(Span::styled(
                    "/skills",
                    theme.style(Tone::Command),
                )));
                lines.extend(render_skills_report(report, width, theme));
            }
            Block::Local(LocalResult::Diff(report)) => {
                lines.push(Line::from(Span::styled(
                    format!("/{}", report.title),
                    theme.style(Tone::Command),
                )));
                lines.extend(render_diff_report(report, width, theme));
            }
            Block::Local(LocalResult::Review(report)) => {
                lines.extend(render_review_report(report, theme));
            }
            Block::Local(LocalResult::Text {
                title,
                body: content,
                state,
            }) => {
                lines.push(Line::from(Span::styled(
                    format!("/{title}"),
                    theme.style(Tone::Command),
                )));
                match state {
                    LocalResultState::Info => {
                        lines.extend(render_local_content(content, width, theme));
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
    lines
}

/// The inspected child's read-only view: one identity heading, then the
/// history the client kept for it.
///
/// Nothing here is invented. The child owns its own canonical transcript in
/// the runtime; what the parent process receives about it is bounded, so
/// what this draws is bounded too — but it is drawn by the same renderer as
/// the root timeline, because a delegated child is an agent that reports
/// back, not a different kind of thing.
fn child_lines(app: &App, child: &str, theme: Theme, width: u16) -> Vec<Line<'static>> {
    let summary = app.children.get(child);
    let state = summary.map_or("unknown", |summary| summary.state.as_str());
    let elapsed = app
        .child_elapsed(child)
        .map(|elapsed| format!(" \u{b7} {}", render_elapsed(elapsed)))
        .unwrap_or_default();
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{} ", glyph::AGENT_CURRENT),
                theme.style(Tone::Accent),
            ),
            Span::styled(
                child.to_owned(),
                theme.style(Tone::Heading).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" \u{b7} {state}{elapsed}"),
                theme.style(child_state_tone(state)),
            ),
        ]),
        Line::default(),
    ];
    if let Some(detail) = app.inspected_detail() {
        lines.extend(render_agent_inspector(detail, theme));
        lines.push(Line::default());
    }

    let blocks = app.child_blocks(child);
    let speculative = app.child_speculative_text(child);
    if blocks.is_empty() && speculative.is_none() {
        // A child restored from a durable record has a state but no live
        // history in this process. Saying so beats an empty pane.
        lines.push(Line::from(Span::styled(
            format!(
                "  no activity recorded in this session{}",
                summary
                    .and_then(|summary| summary.detail.as_deref())
                    .map(|detail| format!(" \u{b7} {detail}"))
                    .unwrap_or_default()
            ),
            theme.style(Tone::Dim),
        )));
        return lines;
    }
    lines.extend(block_lines(blocks, theme, width));
    // A child answers by streaming, like any agent. Its uncommitted text draws
    // exactly where the root timeline draws its own — held out of the
    // transcript until the attempt commits, so a retry cannot leave prose
    // behind.
    if let Some(text) = speculative {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.extend(render_speculative_lines(text, theme));
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

/// Whether a successful tool call's row is already reported in full by a
/// named non-transcript surface, and can therefore be dropped.
///
/// The set is enumerated explicitly here, never inferred from the call's
/// name shape, argument count, or result size — see `tool-call-display`'s
/// "Reviewed redundant-row suppression". Only a successful call ever
/// qualifies: a failure, a denial, or a call whose outcome never arrived is
/// not redundant with anything, so any status other than
/// [`ToolStatus::Ok`] always renders. `agent`'s action is read from the
/// projector's own `target()` (`"spawn"`, `"wait"`, …) — a call with no
/// reviewed display cannot be matched against that vocabulary at all, so it
/// renders rather than being guessed at.
fn is_redundant_tool_row(
    name: &str,
    status: ToolStatus,
    display: Option<&ToolCallDisplay>,
) -> bool {
    if status != ToolStatus::Ok {
        return false;
    }
    match name {
        "write_todos" => true,
        "registry.search" => true,
        // Delegation's own lifecycle line reports `wait`, `result`,
        // `resume`, and `stop`. `spawn` is the one row that announces the
        // spawn, so it is never suppressed, and nothing else reports
        // `follow_up` or `list`, so they render too.
        "agent" => matches!(
            display.map(ToolCallDisplay::target),
            Some("wait" | "result" | "resume" | "stop")
        ),
        _ => false,
    }
}

/// The compact invocation portion of a tool row, with any host-confirmed
/// enrichment appended after the projector's own qualifiers.
///
/// Enrichment is kept in a field of its own on `Block::Tool` rather than
/// folded into `display`'s qualifiers, specifically so a later
/// re-projection at tool completion cannot silently drop it; this is where
/// the two are joined back together, freshly on every draw.
fn tool_invocation(
    name: &str,
    display: Option<&ToolCallDisplay>,
    enrichment: &[String],
    protected_summary: &str,
) -> String {
    let Some(display) = display else {
        return format!("{}({protected_summary})", safe_tool_name(name));
    };
    if enrichment.is_empty() {
        return display.invocation();
    }
    let mut details = Vec::with_capacity(display.qualifiers().len() + enrichment.len() + 1);
    details.push(display.target());
    details.extend(display.qualifiers().iter().map(String::as_str));
    details.extend(enrichment.iter().map(String::as_str));
    format!("{}({})", display.label(), details.join(" · "))
}

pub(super) use crate::transcript::safe_tool_name;

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

pub(super) fn render_status_card(
    report: &StatusReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let available = usize::from(width.saturating_sub(4)).max(1);
    let mut fields = vec![
        ("session", report.session.clone()),
        ("profile", report.profile.clone()),
        (
            "provider",
            format!("{} · model: {}", report.provider, report.model),
        ),
        ("permission", report.permission.clone()),
        ("reasoning", report.reasoning.clone()),
        ("reasoning controls", report.reasoning_controls.clone()),
        ("prompt cache", report.prompt_cache.clone()),
        ("cache maintenance", report.cache_maintenance.clone()),
        ("resume checkpoint", report.resume_checkpoint.clone()),
        ("project", report.project.clone()),
        ("Git", report.git.clone()),
    ];
    match &report.goal {
        StatusGoal::None => fields.push(("goal", "none".to_owned())),
        StatusGoal::Unavailable(error) => {
            fields.push(("goal", format!("unavailable ({error})")));
        }
        StatusGoal::Active(goal) => fields.extend([
            ("goal", goal.objective.clone()),
            ("status", goal.status.clone()),
            ("tokens", goal.tokens.clone()),
            ("budget", goal.budget.clone()),
            ("active elapsed", goal.active_elapsed.clone()),
            ("reason", goal.reason.clone()),
            ("id", goal.id.clone()),
        ]),
    }
    fields.extend([
        ("children", report.children.to_string()),
        ("usage", report.usage.clone()),
        ("cost", report.cost.clone()),
    ]);
    let label_width = fields
        .iter()
        .map(|(label, _)| label.width())
        .max()
        .unwrap_or(0);

    let mut body = vec![
        Line::from(vec![
            Span::styled(" >_ ", theme.style(Tone::Dim)),
            Span::styled("Smith", theme.style(Tone::Heading)),
        ]),
        Line::default(),
    ];

    for (label, value) in fields {
        let mut value_lines = value.lines();
        body.extend(render_status_field(
            label,
            value_lines.next().unwrap_or_default().trim_start(),
            label_width,
            available,
            theme,
        ));
        for raw in value_lines {
            body.extend(
                wrap_text(raw, available)
                    .into_iter()
                    .map(|line| Line::from(Span::styled(line, theme.style(Tone::Default)))),
            );
        }
    }
    body.extend(
        wrap_text(StatusReport::DIAGNOSTICS_HINT, available)
            .into_iter()
            .map(|line| Line::from(Span::styled(line, theme.style(Tone::Default)))),
    );

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

pub(super) fn render_local_content(content: &str, width: u16, theme: Theme) -> Vec<Line<'static>> {
    let available = usize::from(width).max(1);
    let mut lines = Vec::new();
    for raw in content.lines() {
        for wrapped in wrap_text(raw, available) {
            lines.push(styled_local_line(&wrapped, theme));
        }
    }
    lines
}

fn render_diff_report(report: &DiffReport, width: u16, theme: Theme) -> Vec<Line<'static>> {
    let patch = match &report.outcome {
        DiffOutcome::Empty => {
            return render_prefixed_local_state(
                glyph::BULLET,
                DiffReport::EMPTY_MESSAGE,
                width,
                theme.style(Tone::Dim),
            );
        }
        DiffOutcome::Error(message) => {
            return render_prefixed_local_state(
                glyph::ERROR,
                message,
                width,
                theme.style(Tone::Danger),
            );
        }
        DiffOutcome::Patch(lines) => lines,
    };
    let available = usize::from(width).max(1);
    patch
        .iter()
        .flat_map(|line| {
            let tone = match line.kind {
                DiffLineKind::Hunk => Tone::Code,
                DiffLineKind::Addition => Tone::Success,
                DiffLineKind::Removal => Tone::Danger,
                DiffLineKind::Metadata => Tone::Dim,
                DiffLineKind::Context => Tone::Default,
            };
            line.text
                .lines()
                .flat_map(move |raw| wrap_text(raw, available))
                .map(move |wrapped| {
                    if wrapped.is_empty() {
                        Line::default()
                    } else {
                        Line::from(Span::styled(wrapped, theme.style(tone)))
                    }
                })
        })
        .collect()
}

/// Keeps the confirmation's existing unstyled patch presentation. The modal
/// owns wrapping and its row cap; no heading or source prefix selects a style.
pub(super) fn render_review_preview(report: &ReviewPreview) -> Vec<Line<'static>> {
    let mut lines = format!("scope: {}\n", report.title)
        .lines()
        .map(|raw| Line::from(raw.to_owned()))
        .collect::<Vec<_>>();
    lines.extend([
        Line::from("provider-backed: yes"),
        Line::from("workspace authority: read-only"),
        Line::from(ReviewReport::AUTHORITY_MESSAGE),
        Line::default(),
    ]);
    lines.extend(
        report
            .patch
            .iter()
            .flat_map(|line| line.text.lines())
            .map(|raw| Line::from(raw.to_owned())),
    );
    lines
}

fn render_review_report(report: &ReviewReport, theme: Theme) -> Vec<Line<'static>> {
    match report {
        ReviewReport::Confirmation(preview) => {
            let mut lines = vec![Line::from(Span::styled(
                "/review",
                theme.style(Tone::Command),
            ))];
            lines.extend(render_review_preview(preview));
            lines
        }
        ReviewReport::Empty => render_review_notice(ReviewReport::EMPTY_MESSAGE, theme),
        ReviewReport::Error(message) => render_review_error(message, theme),
        ReviewReport::Start(start) => {
            let content = start.render_value();
            match start {
                ReviewStartReport::Started { .. } | ReviewStartReport::Queued { .. } => {
                    render_review_notice(&content, theme)
                }
                ReviewStartReport::Unavailable
                | ReviewStartReport::AtCapacity { .. }
                | ReviewStartReport::Failed(_) => render_review_error(&content, theme),
            }
        }
    }
}

fn render_review_notice(content: &str, theme: Theme) -> Vec<Line<'static>> {
    content
        .lines()
        .enumerate()
        .map(|(index, raw)| {
            if index == 0 {
                Line::from(vec![
                    Span::styled(format!("{} ", glyph::NOTICE), theme.style(Tone::Dim)),
                    Span::styled("review", theme.style(Tone::Heading)),
                    Span::styled(" · ", theme.style(Tone::Dim)),
                    Span::styled(raw.to_owned(), theme.style(Tone::Default)),
                ])
            } else {
                Line::from(Span::styled(format!("  {raw}"), theme.style(Tone::Dim)))
            }
        })
        .collect()
}

fn render_review_error(message: &str, theme: Theme) -> Vec<Line<'static>> {
    message
        .lines()
        .enumerate()
        .map(|(index, raw)| {
            let marker = if index == 0 { glyph::ERROR } else { " " };
            Line::from(Span::styled(
                format!("{marker} {raw}"),
                theme.style(Tone::Danger),
            ))
        })
        .collect()
}

fn render_agent_report(report: &AgentReport, width: u16, theme: Theme) -> Vec<Line<'static>> {
    if let AgentReport::Resume(resume) = report {
        let content = resume.render_value();
        return content
            .lines()
            .enumerate()
            .map(|(index, raw)| match resume {
                AgentResumeReport::RequiresIdle | AgentResumeReport::Started { .. } => {
                    if index == 0 {
                        Line::from(vec![
                            Span::styled(format!("{} ", glyph::NOTICE), theme.style(Tone::Dim)),
                            Span::styled(report.title().to_owned(), theme.style(Tone::Heading)),
                            Span::styled(" · ", theme.style(Tone::Dim)),
                            Span::styled(raw.to_owned(), theme.style(Tone::Default)),
                        ])
                    } else {
                        Line::from(Span::styled(format!("  {raw}"), theme.style(Tone::Dim)))
                    }
                }
                AgentResumeReport::Missing { .. }
                | AgentResumeReport::Incompatible { .. }
                | AgentResumeReport::Unavailable
                | AgentResumeReport::Failed { .. } => Line::from(Span::styled(
                    format!("{} {raw}", if index == 0 { glyph::ERROR } else { " " }),
                    theme.style(Tone::Danger),
                )),
            })
            .collect();
    }

    let mut lines = vec![Line::from(Span::styled(
        format!("/{}", report.title()),
        theme.style(Tone::Command),
    ))];
    match report {
        AgentReport::Empty => lines.extend(render_prefixed_local_state(
            glyph::BULLET,
            AgentReport::EMPTY_MESSAGE,
            width,
            theme.style(Tone::Dim),
        )),
        AgentReport::Unavailable => lines.extend(render_prefixed_local_state(
            glyph::ERROR,
            AgentReport::UNAVAILABLE_MESSAGE,
            width,
            theme.style(Tone::Danger),
        )),
        AgentReport::Missing(child) => lines.extend(render_prefixed_local_state(
            glyph::ERROR,
            &format!("No child named `{child}`."),
            width,
            theme.style(Tone::Danger),
        )),
        AgentReport::Parent => lines.extend(wrap_context_line(
            Line::from(Span::styled(
                AgentReport::PARENT_MESSAGE,
                theme.style(Tone::Default),
            )),
            usize::from(width).max(1),
        )),
        AgentReport::List(children) => {
            for child in children {
                let content = format!(
                    "{} · {} · {} · resumable {} · {} turns · {} tokens",
                    child.child,
                    child.durability,
                    child.state,
                    child.resumable,
                    child.turns,
                    child.tokens_used,
                );
                lines.extend(wrap_context_line(
                    Line::from(Span::styled(content, theme.style(Tone::Default))),
                    usize::from(width).max(1),
                ));
            }
        }
        AgentReport::Inspector(child) => lines.extend(render_agent_inspector(child, theme)),
        AgentReport::Resume(_) => {}
    }
    lines
}

/// Inspector fields keep the existing indentation and paragraph word wrapping.
fn render_agent_inspector(child: &AgentSnapshot, theme: Theme) -> Vec<Line<'static>> {
    [
        format!(
            "session {} · {} · {} · {} · {} tokens · {}",
            child.session,
            child.summary.durability,
            child.summary.state,
            child.summary.turns,
            child.summary.tokens_used,
            child.workspace,
        ),
        format!(
            "resumable {}{}",
            child.summary.resumable,
            child
                .incompatibility
                .as_deref()
                .map(|reason| format!(" · incompatible: {reason}"))
                .unwrap_or_default(),
        ),
        format!(
            "continue: type a follow-up below · exact recovery: /agent resume {}",
            child.summary.child,
        ),
        format!(
            "result: {}",
            child.last_result.as_deref().unwrap_or("not available"),
        ),
    ]
    .into_iter()
    .flat_map(|content| {
        content
            .lines()
            .map(|raw| Line::from(Span::styled(format!("  {raw}"), theme.style(Tone::Dim))))
            .collect::<Vec<_>>()
    })
    .collect()
}

fn render_mcp_report(report: &McpReport, width: u16, theme: Theme) -> Vec<Line<'static>> {
    let servers = match report {
        McpReport::Empty { guidance } => {
            return render_mcp_text(
                if *guidance {
                    McpReport::EMPTY_GUIDANCE
                } else {
                    McpReport::EMPTY_MESSAGE
                },
                width,
                theme,
            );
        }
        McpReport::Unavailable => {
            return render_prefixed_local_state(
                glyph::ERROR,
                McpReport::EMPTY_MESSAGE,
                width,
                theme.style(Tone::Danger),
            );
        }
        McpReport::Error(error) => {
            return render_prefixed_local_state(
                glyph::ERROR,
                error,
                width,
                theme.style(Tone::Danger),
            );
        }
        McpReport::Trusted { server, digest } => {
            return render_mcp_text(&McpReport::trusted_value(server, digest), width, theme);
        }
        McpReport::Servers(servers) => servers,
    };
    let available = usize::from(width).max(1);
    let mut lines = Vec::new();
    for server in servers {
        lines.extend(render_mcp_text(
            &format!(
                "{} · {} · {} · {}",
                server.name,
                server.transport,
                server.state.render_value(&server.name),
                server.source,
            ),
            width,
            theme,
        ));
        for rejected in &server.rejected {
            lines.extend(wrap_context_line(
                context_field("  refused a tool", rejected.clone(), Tone::Default, theme),
                available,
            ));
        }
        for value in &server.values {
            lines.extend(wrap_context_line(
                Line::from(vec![
                    Span::styled(
                        format!("  {} {} ← ", value.kind.label(), value.name),
                        theme.style(Tone::Default),
                    ),
                    Span::styled(
                        value
                            .credential
                            .as_deref()
                            .unwrap_or("value withheld")
                            .to_owned(),
                        theme.style(Tone::Default),
                    ),
                ]),
                available,
            ));
        }
    }
    lines
}

/// Retains character wrapping before inline Markdown for the typed MCP fields.
fn render_mcp_text(content: &str, width: u16, theme: Theme) -> Vec<Line<'static>> {
    content
        .lines()
        .flat_map(|raw| {
            wrap_text(raw, usize::from(width).max(1))
                .into_iter()
                .map(|wrapped| {
                    Line::from(render_inline_markdown(
                        &wrapped,
                        theme.style(Tone::Default),
                        theme,
                    ))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn render_skills_report(report: &SkillsReport, width: u16, theme: Theme) -> Vec<Line<'static>> {
    let (groups, problems) = match report {
        SkillsReport::Empty => {
            return render_skills_text(SkillsReport::EMPTY_MESSAGE, width, theme);
        }
        SkillsReport::Error(error) => {
            return render_prefixed_local_state(
                glyph::ERROR,
                error,
                width,
                theme.style(Tone::Danger),
            );
        }
        SkillsReport::Trusted { skill, digest } => {
            return render_skills_text(&SkillsReport::trusted_value(skill, digest), width, theme);
        }
        SkillsReport::Indexed { groups, problems } => (groups, problems),
    };
    let mut lines = Vec::new();
    for group in groups {
        lines.extend(render_skills_text(group.layer.as_str(), width, theme));
        for entry in &group.entries {
            lines.extend(render_skills_text(
                &format!(
                    "  {} · {} · {}",
                    entry.name,
                    entry.state.render_value(&entry.name),
                    entry.description,
                ),
                width,
                theme,
            ));
        }
    }
    if groups.is_empty() {
        lines.extend(render_skills_text(
            SkillsReport::EMPTY_MESSAGE,
            width,
            theme,
        ));
    }
    if !problems.is_empty() {
        lines.extend(render_skills_text("not loaded", width, theme));
        for problem in problems {
            lines.extend(render_skills_text(
                &format!("  {} · {} · {}", problem.name, problem.reason, problem.path,),
                width,
                theme,
            ));
        }
    }
    lines
}

/// Retains wrapping before inline Markdown for the typed skill fields.
fn render_skills_text(content: &str, width: u16, theme: Theme) -> Vec<Line<'static>> {
    content
        .lines()
        .flat_map(|raw| {
            wrap_text(raw, usize::from(width).max(1))
                .into_iter()
                .map(|wrapped| {
                    // The previous generic text path left rows containing a
                    // colon literal. Keep those bytes without interpreting
                    // the value as a label or recovering report structure.
                    if wrapped.contains(':') {
                        Line::from(Span::styled(wrapped, theme.style(Tone::Default)))
                    } else {
                        Line::from(render_inline_markdown(
                            &wrapped,
                            theme.style(Tone::Default),
                            theme,
                        ))
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn render_goal_report(report: &GoalReport, width: u16, theme: Theme) -> Vec<Line<'static>> {
    let goal = match report {
        GoalReport::Empty => {
            return render_prefixed_local_state(
                glyph::BULLET,
                GoalReport::EMPTY_MESSAGE,
                width,
                theme.style(Tone::Dim),
            );
        }
        GoalReport::Unavailable(error) => {
            return render_prefixed_local_state(
                glyph::ERROR,
                error,
                width,
                theme.style(Tone::Danger),
            );
        }
        GoalReport::Cleared => {
            return wrap_context_line(
                Line::from(Span::styled(
                    GoalReport::CLEARED_MESSAGE,
                    theme.style(Tone::Default),
                )),
                usize::from(width).max(1),
            );
        }
        GoalReport::Snapshot(goal) => goal,
    };
    let available = usize::from(width).max(1);
    let mut lines = Vec::new();
    for raw in goal.objective.split('\n') {
        for wrapped in wrap_text(raw.strip_suffix('\r').unwrap_or(raw), available) {
            lines.push(Line::from(render_inline_markdown(
                &wrapped,
                theme.style(Tone::Default),
                theme,
            )));
        }
    }
    for (label, value) in [
        ("status", goal.status.clone()),
        (
            "tokens",
            format!(
                "{} · {}",
                goal.charged_tokens_value(),
                goal.usage_provenance
            ),
        ),
        ("budget", goal.budget_value()),
        ("active elapsed", goal.active_elapsed.clone()),
        ("reason", goal.reason_value()),
        (
            "id",
            format!("{} · generation {}", goal.id, goal.generation),
        ),
    ] {
        lines.extend(wrap_context_line(
            context_field(label, value, Tone::Default, theme),
            available,
        ));
    }
    lines
}

fn render_timeline_report(report: &TimelineReport, width: u16, theme: Theme) -> Vec<Line<'static>> {
    let entries = match report {
        TimelineReport::Empty => {
            return render_prefixed_local_state(
                glyph::BULLET,
                TimelineReport::EMPTY_MESSAGE,
                width,
                theme.style(Tone::Dim),
            );
        }
        TimelineReport::Unavailable(error) => {
            return render_prefixed_local_state(
                glyph::ERROR,
                &format!("timeline unavailable: {error}"),
                width,
                theme.style(Tone::Danger),
            );
        }
        TimelineReport::Entries(entries) => entries,
    };
    let available = usize::from(width).max(1);
    let mut lines = Vec::new();
    for entry in entries {
        let content = match entry {
            TimelineEntry::RootTurn {
                turn,
                finish,
                plan,
                passed_gates,
                failed_gates,
            } => format!(
                "root {turn} · {finish} · {} · gates {passed_gates} passed/{failed_gates} failed",
                plan.as_ref()
                    .map_or_else(|| "plan none".to_owned(), TimelinePlan::render_value),
            ),
            TimelineEntry::RootManifest {
                turn,
                provider,
                model,
                activated_capabilities,
            } => format!(
                "root {turn} · committed · {provider}/{model} · {activated_capabilities} activated capability/capabilities",
            ),
            TimelineEntry::ChildEvent { child, event } => {
                format!("child {child} · {}", event.render_value())
            }
            TimelineEntry::ChildSnapshot {
                child,
                session,
                durability,
                state,
                resumable,
                turns,
            } => format!(
                "child {child} · session {session} · {durability} · {state} · resumable {resumable} · {turns} turns",
            ),
            TimelineEntry::Recovery { number, detail } => {
                format!("recovery recovery-{number} · {detail}")
            }
        };
        for raw in content.lines() {
            for wrapped in wrap_text(raw, available) {
                lines.push(Line::from(render_inline_markdown(
                    &wrapped,
                    theme.style(Tone::Default),
                    theme,
                )));
            }
        }
    }
    lines
}

pub(super) fn render_help_report(
    report: &HelpReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let available = usize::from(width).max(1);
    let mut lines = wrap_context_line(
        Line::from(Span::styled(
            HelpReport::GETTING_STARTED_HEADING,
            theme.style(Tone::Heading),
        )),
        available,
    );
    lines.extend(wrap_context_line(
        Line::from(Span::styled(
            report.introduction.clone(),
            theme.style(Tone::Dim),
        )),
        available,
    ));
    lines.extend(
        report
            .getting_started
            .iter()
            .flat_map(|command| render_help_command(command, available, theme)),
    );
    for (heading, commands) in [
        (HelpReport::PRIMARY_HEADING, &report.primary),
        (HelpReport::ADVANCED_HEADING, &report.advanced),
    ] {
        lines.push(Line::default());
        lines.extend(wrap_context_line(
            Line::from(Span::styled(heading, theme.style(Tone::Heading))),
            available,
        ));
        lines.extend(
            commands
                .iter()
                .flat_map(|command| render_help_command(command, available, theme)),
        );
    }
    lines.push(Line::default());
    lines.extend(wrap_context_line(
        Line::from(Span::styled(
            HelpReport::COMPOSER_HEADING,
            theme.style(Tone::Dim),
        )),
        available,
    ));
    lines.extend(report.composer.iter().flat_map(|guidance| {
        wrap_context_line(
            Line::from(Span::styled(guidance.clone(), theme.style(Tone::Dim))),
            available,
        )
    }));
    lines
}

fn render_help_command(
    command: &HelpCommand,
    available: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    let invocation = command.invocation();
    let command_end = invocation.len();
    let description_start = command_end + " — ".len();
    let raw = format!("{invocation} — {}", command.description);
    let mut start = 0;
    wrap_text(&raw, available)
        .into_iter()
        .map(|wrapped| {
            let end = start + wrapped.len();
            // Preserve wrapping before replacing the plain separator with two
            // spaces. Only a row containing the entire separator has a code
            // span; its boundaries come from the fields, never parsed text.
            let line = if start <= command_end && end >= description_start {
                Line::from(vec![
                    Span::styled(
                        wrapped[..command_end - start].to_owned(),
                        theme.style(Tone::Code),
                    ),
                    Span::styled("  ", theme.style(Tone::Dim)),
                    Span::styled(
                        wrapped[description_start - start..].to_owned(),
                        theme.style(Tone::Dim),
                    ),
                ])
            } else {
                Line::from(Span::styled(wrapped, theme.style(Tone::Dim)))
            };
            start = end;
            line
        })
        .collect()
}

pub(super) fn render_context_report(
    report: &ContextReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let available = usize::from(width).max(1);
    let mut body = vec![Line::from(Span::styled(
        ContextReport::HEADING,
        theme.style(Tone::Heading),
    ))];
    if !report.available_windows.is_empty() {
        body.push(context_field(
            "available context windows",
            report.available_windows_value(),
            Tone::Default,
            theme,
        ));
    }
    body.push(Line::from(Span::styled(
        report.summary.clone(),
        theme.style(Tone::Dim),
    )));
    body.push(Line::default());
    for row in report.grid() {
        let mut spans = Vec::new();
        for (index, kind) in row.into_iter().enumerate() {
            if index > 0 {
                spans.push(Span::styled(" ", theme.style(Tone::Default)));
            }
            let (glyph, tone) = context_category_style(kind);
            spans.push(Span::styled(glyph, theme.style(tone)));
        }
        if report.grid_is_empty() {
            spans.push(Span::styled(" ", theme.style(Tone::Default)));
        }
        body.push(Line::from(spans));
    }
    body.push(Line::default());
    body.push(Line::from(Span::styled(
        report.usage.category_heading(),
        theme.style(Tone::Dim).add_modifier(Modifier::ITALIC),
    )));
    for category in &report.categories {
        body.push(context_category_line(
            category.kind,
            &category.label,
            &category.value,
            theme,
        ));
    }
    body.push(context_category_line(
        ContextCategoryKind::Free,
        "free input",
        &report.free_input.value,
        theme,
    ));
    body.push(context_category_line(
        ContextCategoryKind::Reserve,
        "output/reasoning reserve",
        &report.reserve.value,
        theme,
    ));
    body.extend([
        context_field(
            "model window",
            report.model_window.clone(),
            Tone::Default,
            theme,
        ),
        context_field("counting", report.counting.clone(), Tone::Warning, theme),
        context_field(
            "compaction",
            report.compaction.render_value(),
            match report.compaction {
                ContextCompaction::Applied { .. } => Tone::Success,
                ContextCompaction::Enabled { .. } => Tone::Default,
            },
            theme,
        ),
        context_field(
            "tool context",
            report.tool_context.clone(),
            Tone::Default,
            theme,
        ),
        Line::from(Span::styled(
            ContextReport::OCCUPANCY_HINT,
            theme.style(Tone::Dim),
        )),
        context_field(
            "provider input (session)",
            report.provider_input.clone(),
            Tone::Default,
            theme,
        ),
        context_field(
            "cache read (session)",
            report.cache_read.clone(),
            Tone::Default,
            theme,
        ),
        context_field("cache", report.cache.clone(), Tone::Default, theme),
        context_field("reasoning", report.reasoning.clone(), Tone::Default, theme),
        context_field(
            "reasoning controls",
            report.reasoning_controls.clone(),
            Tone::Default,
            theme,
        ),
    ]);
    body.into_iter()
        .flat_map(|line| wrap_context_line(line, available))
        .collect()
}

fn context_category_style(kind: ContextCategoryKind) -> (&'static str, Tone) {
    match kind {
        ContextCategoryKind::System => (glyph::CONTEXT_SYSTEM, Tone::Accent),
        ContextCategoryKind::Tool => (glyph::CONTEXT_TOOL, Tone::Warning),
        ContextCategoryKind::History => (glyph::CONTEXT_HISTORY, Tone::Command),
        ContextCategoryKind::Summary => (glyph::CONTEXT_SUMMARY, Tone::Success),
        ContextCategoryKind::Input => (glyph::CONTEXT_INPUT, Tone::Accent),
        ContextCategoryKind::Other => (glyph::CONTEXT_OTHER, Tone::Default),
        ContextCategoryKind::Free => (glyph::CONTEXT_FREE, Tone::Dim),
        ContextCategoryKind::Reserve => (glyph::CONTEXT_RESERVE, Tone::Dim),
    }
}

fn context_category_line(
    kind: ContextCategoryKind,
    label: &str,
    value: &str,
    theme: Theme,
) -> Line<'static> {
    let (glyph, tone) = context_category_style(kind);
    Line::from(vec![
        Span::styled(glyph, theme.style(tone)),
        Span::raw(" "),
        Span::styled(format!("{label}:"), theme.style(Tone::Default)),
        Span::styled(format!(" {value}"), theme.style(Tone::Dim)),
    ])
}

fn context_field(label: &str, value: String, tone: Tone, theme: Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}:"), theme.style(Tone::Dim)),
        Span::styled(format!(" {value}"), theme.style(tone)),
    ])
}

/// Keeps the existing character wrapping, carrying styles from typed fields.
/// Byte ranges only split spans; no text selects a presentation branch.
fn wrap_context_line(line: Line<'static>, available: usize) -> Vec<Line<'static>> {
    let raw = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    if raw.is_empty() {
        return vec![Line::default()];
    }
    let mut rows = Vec::new();
    let mut line_start = 0;
    for raw_line in raw.split_inclusive('\n') {
        let content = raw_line
            .strip_suffix('\n')
            .map(|text| text.strip_suffix('\r').unwrap_or(text))
            .unwrap_or(raw_line);
        let mut start = line_start;
        for wrapped in wrap_text(content, available) {
            let end = start + wrapped.len();
            let mut offset = 0;
            let spans = line
                .spans
                .iter()
                .filter_map(|span| {
                    let span_start = offset;
                    offset += span.content.len();
                    let from = start.max(span_start);
                    let to = end.min(offset);
                    if from < to {
                        Some(Span::styled(
                            span.content[from - span_start..to - span_start].to_owned(),
                            span.style,
                        ))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            rows.push(Line::from(spans));
            start = end;
        }
        line_start += raw_line.len();
    }
    rows
}

pub(super) fn styled_local_line(raw: &str, theme: Theme) -> Line<'static> {
    if raw.is_empty() {
        return Line::default();
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
