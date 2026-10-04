//! Transcript, Markdown, tool, status, and local-result rendering.

use std::borrow::Cow;
use std::cell::Ref;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::status::{Activity, render_elapsed};
use crate::theme::{Theme, Tone, glyph};
use crate::transcript::{Block, LocalResult, ToolStatus};
use smith_client::agent_report::{AgentReport, AgentResumeReport, AgentSnapshot};
use smith_client::commands::getting_started_commands;
use smith_client::context_report::{ContextCategoryKind, ContextCompaction, ContextReport};
use smith_client::diagnostics_report::{DiagnosticsReport, DiagnosticsRow};
use smith_client::diff_report::{DiffLine, DiffLineKind, DiffOutcome, DiffReport};
use smith_client::goal_report::GoalReport;
use smith_client::help_report::{HelpCommand, HelpReport};
use smith_client::mcp_report::McpReport;
use smith_client::message_report::MessageReport;
use smith_client::recovery_report::{RecoveryAction, RecoveryReport, RevertPreview};
use smith_client::review_report::{ReviewPreview, ReviewReport, ReviewStartReport};
use smith_client::shell_report::{ShellOutput, ShellReport};
use smith_client::skills_report::SkillsReport;
use smith_client::status_report::{StatusGoal, StatusReport};
use smith_client::timeline_report::{TimelineEntry, TimelinePlan, TimelineReport};
use smith_tools::{ToolCallDisplay, tool_display_label};

use super::helpers::*;
use super::lists::clip_words;
use super::markdown::render_assistant_lines;
use super::reports;
use super::wrap::{wrap_lines, wrapped_row_count};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlockKey {
    revision: u64,
    width: u16,
    expanded: bool,
    theme: Theme,
}

#[derive(Debug)]
struct CachedBlock {
    key: BlockKey,
    rows: Vec<Line<'static>>,
    suppressed: bool,
    elapsed: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct TranscriptCache {
    blocks: Vec<Option<CachedBlock>>,
    prefix: Vec<usize>,
    #[cfg(test)]
    renders: usize,
}

impl TranscriptCache {
    fn sync(&mut self, app: &App, theme: Theme, width: u16) {
        let blocks = app.transcript.blocks();
        self.blocks.resize_with(blocks.len(), || None);
        self.prefix.clear();
        self.prefix.push(0);
        for (index, block) in blocks.iter().enumerate() {
            let key = BlockKey {
                revision: app.transcript.block_revision(index),
                width,
                expanded: app.work_details,
                theme,
            };
            // A running call's clock is presentation-only. Refresh its rows
            // when the displayed elapsed time changes, even without an event.
            let elapsed = match block {
                Block::Tool {
                    status: ToolStatus::Running,
                    started_at: Some(started),
                    ..
                } => Some(render_elapsed(started.elapsed())),
                _ => None,
            };
            if self.blocks[index]
                .as_ref()
                .is_none_or(|cached| cached.key != key || cached.elapsed != elapsed)
            {
                let suppressed = suppressed_block(block);
                self.blocks[index] = Some(CachedBlock {
                    key,
                    rows: wrap_lines(
                        &block_lines(std::slice::from_ref(block), theme, width, app.work_details),
                        width,
                    ),
                    suppressed,
                    elapsed,
                });
                #[cfg(test)]
                {
                    self.renders += 1;
                }
            }
            let cached = self.blocks[index].as_ref().expect("cached block");
            let preceding = self.prefix[index];
            let separator = usize::from(!cached.suppressed && preceding > 0);
            self.prefix.push(preceding + separator + cached.rows.len());
        }
    }
}

pub(super) struct TranscriptRows<'a> {
    cache: Option<Ref<'a, TranscriptCache>>,
    block_count: usize,
    tail: Vec<Line<'static>>,
}

impl TranscriptRows<'_> {
    /// A result starts after the preceding visible blocks and their separator.
    pub(super) fn block_start_row(&self, block: usize) -> Option<usize> {
        let cache = self.cache.as_ref()?;
        if block >= cache.blocks.len() {
            return None;
        }
        let preceding = cache.prefix[block];
        Some(preceding + usize::from(preceding > 0))
    }

    fn block_rows(&self) -> usize {
        self.cache
            .as_ref()
            .map_or(0, |cache| cache.prefix[self.block_count])
    }

    pub(super) fn scroll_limit(&self, area: Rect) -> usize {
        (self.block_rows() + self.tail.len()).saturating_sub(usize::from(area.height))
    }

    fn window(&self, start: usize, height: u16) -> Vec<Line<'static>> {
        let end = start.saturating_add(usize::from(height));
        let mut rows = Vec::with_capacity(usize::from(height));
        if let Some(cache) = &self.cache {
            let first = cache.prefix[..=self.block_count]
                .partition_point(|offset| *offset <= start)
                .saturating_sub(1);
            for index in first..self.block_count {
                let mut offset = cache.prefix[index];
                if offset >= end {
                    break;
                }
                let block = cache.blocks[index].as_ref().expect("cached block");
                if !block.suppressed && offset > 0 {
                    if offset >= start {
                        rows.push(Line::default());
                    }
                    offset += 1;
                }
                append_window(&mut rows, &block.rows, offset, start, end);
            }
        }
        append_window(&mut rows, &self.tail, self.block_rows(), start, end);
        rows
    }
}

fn append_window(
    visible: &mut Vec<Line<'static>>,
    rows: &[Line<'static>],
    offset: usize,
    start: usize,
    end: usize,
) {
    let from = start.saturating_sub(offset).min(rows.len());
    let to = end.saturating_sub(offset).min(rows.len());
    visible.extend(rows[from..to].iter().cloned());
}

pub(super) fn transcript_rows(app: &App, theme: Theme, width: u16) -> TranscriptRows<'_> {
    if let Some(child) = &app.inspected_child {
        return TranscriptRows {
            cache: None,
            block_count: 0,
            tail: wrap_lines(&child_lines(app, child, theme, width), width),
        };
    }
    if show_getting_started(app) {
        return TranscriptRows {
            cache: None,
            block_count: 0,
            tail: wrap_lines(&getting_started_lines(theme, width), width),
        };
    }
    app.transcript_cache.borrow_mut().sync(app, theme, width);
    let cache = app.transcript_cache.borrow();
    let blocks = app.transcript.blocks();
    let mut block_count = blocks.len();
    let mut tail = Vec::new();
    if let Some(text) = app.speculative_text() {
        // Preview a commit into the open assistant block at the same boundary.
        let text = if let Some(Block::Assistant {
            text: body,
            open: true,
        }) = blocks.last()
        {
            block_count -= 1;
            Cow::Owned(format!("{body}{text}"))
        } else {
            Cow::Borrowed(text)
        };
        if cache.prefix[block_count] > 0 {
            tail.push(Line::default());
        }
        tail.extend(render_assistant_lines(&text, theme, width));
    }
    append_turn_summary(&mut tail, cache.prefix[block_count] > 0, app, theme);
    TranscriptRows {
        cache: Some(cache),
        block_count,
        tail: wrap_lines(&tail, width),
    }
}

pub(super) fn draw_transcript(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let rows = transcript_rows(app, theme, area.width);
    let max_scroll = rows.scroll_limit(area);
    let pending_offset = app
        .scroll_to_block
        .filter(|_| app.inspected_child.is_none())
        .and_then(|block| rows.block_start_row(block));
    let offset = if app.inspected_child.is_none() && app.output_after_result() {
        max_scroll
    } else if let Some(offset) = pending_offset {
        offset.min(max_scroll)
    } else if app.following {
        max_scroll
    } else {
        max_scroll.saturating_sub(app.scroll_back)
    };
    frame.render_widget(Paragraph::new(rows.window(offset, area.height)), area);
}

#[cfg(test)]
pub(super) fn visual_scroll_limit(lines: &[Line<'static>], area: Rect) -> usize {
    rendered_rows(lines, area.width).saturating_sub(usize::from(area.height))
}

/// Rows `lines` occupy under the exact word-wrap arithmetic the paragraphs
/// render with. Uncached surfaces use this rather than a character-wrap guess,
/// which undercounts word-wrapped prose and truncates modal action bars. The
/// transcript uses the counts of its cached rows from the same wrapper.
pub(super) fn rendered_rows(lines: &[Line<'static>], width: u16) -> usize {
    wrapped_row_count(lines, width)
}

fn show_getting_started(app: &App) -> bool {
    app.transcript.is_empty()
        && app.status.activity == Activity::Idle
        && !app.has_live_work()
        && app.live_child_count() == 0
        && app.speculative_text().is_none()
        && app.visible_turn_summary().is_none()
}

fn append_turn_summary(lines: &mut Vec<Line<'static>>, preceding: bool, app: &App, theme: Theme) {
    if let Some(summary) = app.visible_turn_summary()
        && !matches!(
            app.status.activity,
            Activity::Working | Activity::Interrupting
        )
    {
        if preceding || !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::from(Span::styled(
            format!("{} {summary}", glyph::WORK),
            theme.style(Tone::Dim),
        )));
    }
}

#[cfg(test)]
pub(super) fn transcript_lines(app: &App, theme: Theme, width: u16) -> Vec<Line<'static>> {
    // The inspector borrows the transcript region rather than floating over
    // it: a child's history is read, scrolled, and selected exactly like the
    // root timeline, and one Esc gives the region back unchanged.
    if let Some(child) = &app.inspected_child {
        return child_lines(app, child, theme, width);
    }
    if show_getting_started(app) {
        return getting_started_lines(theme, width);
    }
    let mut lines = conversation_lines(
        app.transcript.blocks(),
        app.speculative_text(),
        theme,
        width,
        app.work_details,
    );

    append_turn_summary(&mut lines, false, app, theme);

    lines
}

fn getting_started_lines(theme: Theme, width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled("  Get started", theme.style(Tone::Heading))),
        Line::from(Span::styled(
            "  Type a task below and press Enter.",
            theme.style(Tone::Default),
        )),
        Line::default(),
    ];
    for command in getting_started_commands() {
        let invocation = format!("/{}", command.name);
        let description = clip_words(command.description, usize::from(width).saturating_sub(12));
        lines.push(Line::from(vec![
            Span::styled(format!("  {invocation:<10}"), theme.style(Tone::Code)),
            Span::styled(description, theme.style(Tone::Dim)),
        ]));
    }
    lines
}

fn conversation_lines(
    blocks: &[Block],
    speculative: Option<&str>,
    theme: Theme,
    width: u16,
    expanded: bool,
) -> Vec<Line<'static>> {
    let Some(text) = speculative else {
        return block_lines(blocks, theme, width, expanded);
    };
    // A commit can extend the current assistant block. Preview that same
    // boundary without promoting an attempt into the canonical transcript.
    let (preceding, text) = if let Some(Block::Assistant {
        text: body,
        open: true,
    }) = blocks.last()
    {
        (
            &blocks[..blocks.len() - 1],
            Cow::Owned(format!("{body}{text}")),
        )
    } else {
        (blocks, Cow::Borrowed(text))
    };
    let mut lines = block_lines(preceding, theme, width, expanded);
    if !lines.is_empty() {
        lines.push(Line::default());
    }
    lines.extend(render_assistant_lines(&text, theme, width));
    lines
}

fn suppressed_block(block: &Block) -> bool {
    // Reasoning is canonical model state, not a second assistant answer.
    // The anchored working row represents progress without
    // exposing raw provider reasoning as transcript prose.
    if matches!(block, Block::Reasoning { .. }) {
        return true;
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
        && is_redundant_tool_row(name, *status, display.as_deref())
    {
        return true;
    }
    false
}

/// Every transcript block as rendered rows.
///
/// The root timeline and a delegated child's history both come through
/// here. A child is an agent that reports back, not a different kind of
/// thing, so it must not get a second, thinner renderer that drifts from
/// this one — whatever the runtime chooses to report about it lands in the
/// same blocks and draws the same way.
fn block_lines(blocks: &[Block], theme: Theme, width: u16, expanded: bool) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for block in blocks {
        if suppressed_block(block) {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        let start = lines.len();
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
                lines.extend(render_assistant_lines(text, theme, width));
            }
            Block::Reasoning { .. } => {}
            Block::Tool {
                name,
                display,
                protected_summary,
                status,
                result_preview,
                user_command,
                started_at,
                enrichment,
                ..
            } => {
                let tone = match status {
                    ToolStatus::WaitingForApproval
                    | ToolStatus::Running
                    | ToolStatus::Unreported => Tone::Dim,
                    ToolStatus::Ok => Tone::Success,
                    ToolStatus::Failed | ToolStatus::Denied => Tone::Danger,
                };
                let mut call = if let Some(command) = user_command {
                    vec![Span::styled(
                        format!("! {command}"),
                        theme.style(Tone::Default),
                    )]
                } else {
                    vec![
                        Span::styled(
                            format!("{} ", glyph::TOOL),
                            theme.style(tone).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            tool_invocation(
                                name,
                                display.as_deref(),
                                enrichment,
                                protected_summary,
                            ),
                            theme.style(Tone::Heading),
                        ),
                    ]
                };
                if !matches!(status, ToolStatus::Ok) {
                    let status_text = match status {
                        ToolStatus::Running => started_at
                            .map(|started| format!("running {}", render_elapsed(started.elapsed())))
                            .unwrap_or_else(|| status.label().to_owned()),
                        _ => status.label().to_owned(),
                    };
                    call.push(Span::styled(format!(" {status_text}"), theme.style(tone)));
                }
                lines.push(Line::from(call));
                if !matches!(status, ToolStatus::Running | ToolStatus::WaitingForApproval) {
                    if let Some(preview) = result_preview {
                        let summary =
                            (status == &ToolStatus::Ok && !expanded && user_command.is_none())
                                .then(|| {
                                    display
                                        .as_ref()
                                        .and_then(|display| display.result_summary(preview))
                                })
                                .flatten();
                        lines.extend(nested_result_lines(
                            summary.as_deref().unwrap_or(preview),
                            expanded,
                            width,
                            theme,
                        ));
                    } else if matches!(status, ToolStatus::Ok) {
                        lines.extend(nested_result_lines("Completed", expanded, width, theme));
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
            Block::Notice { kind: source, text } if source.label() == "turn" => {
                for raw in text.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("  {raw}"),
                        theme.style(Tone::Dim),
                    )));
                }
            }
            Block::Notice { kind: source, text } => {
                for (index, raw) in text.lines().enumerate() {
                    if index == 0 {
                        lines.push(Line::from(vec![
                            Span::styled(format!("{} ", glyph::NOTICE), theme.style(Tone::Dim)),
                            Span::styled(source.label().into_owned(), theme.style(Tone::Heading)),
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
                lines.push(report_title("/status", theme));
                lines.extend(render_status_card(report, width, theme));
            }
            Block::Local(LocalResult::Diagnostics(report)) => {
                lines.push(report_title("/diagnostics", theme));
                lines.extend(render_diagnostics_report(report, width, theme));
            }
            Block::Local(LocalResult::Context(report)) => {
                lines.push(report_title("/context", theme));
                lines.extend(render_context_report(report, width, theme));
            }
            Block::Local(LocalResult::Help(report)) => {
                lines.push(report_title("/help", theme));
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
                lines.extend(render_review_report(report, width, theme));
            }
            Block::Local(LocalResult::Recovery(report)) => {
                lines.extend(render_recovery_report(report, width, theme));
            }
            Block::Local(LocalResult::Shell(report)) => {
                lines.push(Line::from(Span::styled(
                    "/shell",
                    theme.style(Tone::Command),
                )));
                lines.extend(render_shell_report(report, width, theme));
            }
            Block::Local(LocalResult::Message(report)) => {
                lines.push(Line::from(Span::styled(
                    format!("/{}", report.title()),
                    theme.style(Tone::Command),
                )));
                lines.extend(render_message_report(report, width, theme));
            }
        }
        if matches!(block, Block::User { .. } | Block::Notice { .. }) {
            let rendered = lines
                .drain(start..)
                .flat_map(|line| hanging_lines(line, width, 2))
                .collect::<Vec<_>>();
            lines.extend(rendered);
        } else if matches!(block, Block::Tool { .. }) {
            let call = lines.remove(start);
            let wrapped = hanging_lines(call, width, 2);
            drop(lines.splice(start..start, wrapped));
        }
    }
    lines
}

fn nested_result_lines(
    output: &str,
    expanded: bool,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    for (index, raw) in output.lines().enumerate() {
        let prefix = if index == 0 {
            format!("  {}  ", glyph::BRANCH)
        } else {
            "     ".to_owned()
        };
        rows.extend(hanging_lines(
            Line::from(vec![
                Span::styled(prefix, theme.style(Tone::Dim)),
                Span::styled(raw.to_owned(), theme.style(Tone::Dim)),
            ]),
            width,
            5,
        ));
    }
    if !expanded && rows.len() > 4 {
        let remaining = rows.len() - 4;
        rows.truncate(4);
        rows.extend(hanging_lines(
            Line::from(Span::styled(
                format!(
                    "     {} +{remaining} lines (ctrl+o to expand)",
                    glyph::ELIDED
                ),
                theme.style(Tone::Dim),
            )),
            width,
            5,
        ));
    }
    rows
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
    let state = summary.map_or(Cow::Borrowed("unknown"), |summary| summary.state.label());
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
                theme.style(summary.map_or(Tone::Dim, |summary| summary.state.tone())),
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
    // A child answers by streaming, like any agent. Its uncommitted text draws
    // exactly where the root timeline draws its own — held out of the
    // transcript until the attempt commits, so a retry cannot leave prose
    // behind.
    lines.extend(conversation_lines(
        blocks,
        speculative,
        theme,
        width,
        app.work_details,
    ));
    lines
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
/// the two are joined back together when the block's rendered rows are
/// refreshed.
fn tool_invocation(
    name: &str,
    display: Option<&ToolCallDisplay>,
    enrichment: &[String],
    protected_summary: &str,
) -> String {
    let Some(display) = display else {
        let label = tool_display_label(name)
            .map(str::to_owned)
            .unwrap_or_else(|| safe_tool_name(name));
        return format!("{label}({protected_summary})");
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
    let mut fields = vec![
        ("session", report.session.clone()),
        ("profile", report.profile.clone()),
        ("provider", report.provider.clone()),
        ("model", report.model.clone()),
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
    let label_width = reports::label_width(fields.iter().map(|(label, _)| *label), width);
    let value_width = usize::from(width).saturating_sub(label_width + 4);
    let mut lines = Vec::new();
    for (label, value) in fields {
        let value = if label == "project" {
            reports::left_shorten(&value, value_width)
        } else {
            value
        };
        lines.extend(report_field(
            label,
            &value,
            Tone::Default,
            label_width,
            width,
            theme,
        ));
    }
    lines.extend(reports::text(
        StatusReport::DIAGNOSTICS_HINT,
        width,
        theme.style(Tone::Default),
    ));
    lines
}

fn report_title(command: &str, theme: Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{} ", glyph::BULLET), theme.style(Tone::Dim)),
        Span::styled(command.to_owned(), theme.style(Tone::Command)),
    ])
}

fn report_field(
    label: &str,
    value: &str,
    tone: Tone,
    label_width: usize,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    reports::field(
        Line::from(Span::styled(label.to_owned(), theme.style(Tone::Dim))),
        value,
        theme.style(tone),
        label_width,
        width,
    )
}

fn render_diagnostics_report(
    report: &DiagnosticsReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let label_width = reports::label_width(
        report
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .filter_map(|row| match row {
                DiagnosticsRow::Field { label, .. } | DiagnosticsRow::Path { label, .. } => {
                    Some(label.as_str())
                }
                DiagnosticsRow::Line(_) => None,
            }),
        width,
    );
    let value_width = usize::from(width).saturating_sub(label_width + 4).max(1);
    let mut lines = Vec::new();
    for (index, section) in report.sections.iter().enumerate() {
        if index > 0 {
            lines.push(Line::default());
        }
        lines.extend(reports::text(
            &section.heading,
            width,
            theme.style(Tone::Heading),
        ));
        for row in &section.rows {
            match row {
                DiagnosticsRow::Field { label, value } => {
                    lines.extend(report_field(
                        label,
                        value,
                        Tone::Default,
                        label_width,
                        width,
                        theme,
                    ));
                }
                DiagnosticsRow::Path { label, value } => {
                    lines.extend(report_field(
                        label,
                        &reports::left_shorten(value, value_width),
                        Tone::Default,
                        label_width,
                        width,
                        theme,
                    ));
                }
                DiagnosticsRow::Line(content) => {
                    lines.extend(
                        reports::text(content, width, theme.style(Tone::Default))
                            .into_iter()
                            .map(|line| {
                                Line::from(render_inline_markdown(
                                    &line.to_string(),
                                    theme.style(Tone::Default),
                                    theme,
                                ))
                            }),
                    );
                }
            }
        }
    }
    lines
}

fn render_shell_report(report: &ShellReport, width: u16, theme: Theme) -> Vec<Line<'static>> {
    match &report.output {
        ShellOutput::Empty => render_prefixed_local_state(
            glyph::BULLET,
            ShellReport::EMPTY_MESSAGE,
            width,
            theme.style(Tone::Dim),
        ),
        ShellOutput::Output(output) if report.is_error => {
            render_prefixed_local_state(glyph::ERROR, output, width, theme.style(Tone::Danger))
        }
        ShellOutput::Output(output) => render_inline_text_lines(output, width, theme),
    }
}

fn render_message_report(report: &MessageReport, width: u16, theme: Theme) -> Vec<Line<'static>> {
    match report {
        MessageReport::Notice { message, .. } => render_inline_text_lines(message, width, theme),
        MessageReport::Empty { message, .. } => {
            render_prefixed_local_state(glyph::BULLET, message, width, theme.style(Tone::Dim))
        }
        MessageReport::Error { message, .. } => {
            render_prefixed_local_state(glyph::ERROR, message, width, theme.style(Tone::Danger))
        }
    }
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
/// owns wrapping and scrolling; no heading or source prefix selects a style.
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

/// Recovery modals retain their unstyled source presentation. Patch roles and
/// report kinds arrive as data; displayed prefixes never choose presentation.
pub(super) fn render_recovery_patch(patch: &[DiffLine]) -> Vec<Line<'static>> {
    patch
        .iter()
        .flat_map(|line| line.text.lines())
        .map(|raw| Line::from(raw.to_owned()))
        .collect()
}

pub(super) fn render_revert_preview(report: &RevertPreview) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("origin: {}", report.origin.label())),
        Line::default(),
    ];
    lines.extend(render_recovery_patch(&report.patch));
    lines
}

fn render_recovery_report(report: &RecoveryReport, width: u16, theme: Theme) -> Vec<Line<'static>> {
    match report {
        RecoveryReport::UndoConfirmation(preview) | RecoveryReport::RedoConfirmation(preview) => {
            let mut lines = vec![Line::from(Span::styled(
                format!("/{}", report.action().name()),
                theme.style(Tone::Command),
            ))];
            lines.extend(render_recovery_patch(&preview.patch));
            lines
        }
        RecoveryReport::RevertConfirmation(preview) => {
            let mut lines = vec![Line::from(Span::styled(
                "/revert",
                theme.style(Tone::Command),
            ))];
            lines.extend(render_revert_preview(preview));
            lines
        }
        RecoveryReport::PreviewError {
            action: RecoveryAction::Redo,
            message,
        } => {
            let mut lines = vec![Line::from(Span::styled(
                "/redo",
                theme.style(Tone::Command),
            ))];
            lines.extend(render_prefixed_local_state(
                glyph::ERROR,
                message,
                width,
                theme.style(Tone::Danger),
            ));
            lines
        }
        RecoveryReport::PreviewError { message, .. }
        | RecoveryReport::ApplyError { message, .. } => render_review_error(message, theme),
        RecoveryReport::RevertUsage => render_review_error(RecoveryReport::REVERT_USAGE, theme),
        RecoveryReport::Applied(applied) => render_report_notice(
            report.action().name(),
            &applied.render_value(),
            width,
            theme,
        ),
        RecoveryReport::Cancelled(action) => render_report_notice(
            action.name(),
            RecoveryReport::CANCELLED_MESSAGE,
            width,
            theme,
        ),
    }
}

fn render_review_report(report: &ReviewReport, width: u16, theme: Theme) -> Vec<Line<'static>> {
    match report {
        ReviewReport::Confirmation(preview) => {
            let mut lines = vec![Line::from(Span::styled(
                "/review",
                theme.style(Tone::Command),
            ))];
            lines.extend(render_review_preview(preview));
            lines
        }
        ReviewReport::Empty => render_review_notice(ReviewReport::EMPTY_MESSAGE, width, theme),
        ReviewReport::Error(message) => render_review_error(message, theme),
        ReviewReport::Start(start) => {
            let content = start.render_value();
            match start {
                ReviewStartReport::Started { .. } | ReviewStartReport::Queued { .. } => {
                    render_review_notice(&content, width, theme)
                }
                ReviewStartReport::Unavailable
                | ReviewStartReport::AtCapacity { .. }
                | ReviewStartReport::Failed(_) => render_review_error(&content, theme),
            }
        }
    }
}

fn render_review_notice(content: &str, width: u16, theme: Theme) -> Vec<Line<'static>> {
    render_report_notice("review", content, width, theme)
}

fn render_report_notice(
    source: &str,
    content: &str,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    content
        .lines()
        .enumerate()
        .map(|(index, raw)| {
            if index == 0 {
                Line::from(vec![
                    Span::styled(format!("{} ", glyph::NOTICE), theme.style(Tone::Dim)),
                    Span::styled(source.to_owned(), theme.style(Tone::Heading)),
                    Span::styled(" · ", theme.style(Tone::Dim)),
                    Span::styled(raw.to_owned(), theme.style(Tone::Default)),
                ])
            } else {
                Line::from(Span::styled(format!("  {raw}"), theme.style(Tone::Dim)))
            }
        })
        .flat_map(|line| hanging_lines(line, width, 2))
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
        if matches!(
            resume,
            AgentResumeReport::RequiresIdle | AgentResumeReport::Started { .. }
        ) {
            return render_report_notice(report.title(), &content, width, theme);
        }
        return content
            .lines()
            .enumerate()
            .map(|(index, raw)| {
                Line::from(Span::styled(
                    format!("{} {raw}", if index == 0 { glyph::ERROR } else { " " }),
                    theme.style(Tone::Danger),
                ))
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
                    child.durability.label(),
                    child.state.label(),
                    child.resumable,
                    child.turns_value(),
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
            child.summary.durability.label(),
            child.summary.state.label(),
            child.summary.turns_value(),
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
            return render_inline_text_lines(SkillsReport::EMPTY_MESSAGE, width, theme);
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
            return render_inline_text_lines(
                &SkillsReport::trusted_value(skill, digest),
                width,
                theme,
            );
        }
        SkillsReport::Indexed { groups, problems } => (groups, problems),
    };
    let mut lines = Vec::new();
    for group in groups {
        lines.extend(wrap_context_line(
            Line::from(Span::styled(
                group.layer.as_str(),
                theme.style(Tone::Default),
            )),
            usize::from(width).max(1),
        ));
        for entry in &group.entries {
            lines.extend(render_skills_free_text(
                &format!(
                    "  {} · {} · ",
                    entry.name,
                    entry.state.render_value(&entry.name),
                ),
                &entry.description,
                "",
                width,
                theme,
            ));
        }
    }
    if groups.is_empty() {
        lines.extend(render_inline_text_lines(
            SkillsReport::EMPTY_MESSAGE,
            width,
            theme,
        ));
    }
    if !problems.is_empty() {
        lines.extend(wrap_context_line(
            Line::from(Span::styled("not loaded", theme.style(Tone::Default))),
            usize::from(width).max(1),
        ));
        for problem in problems {
            lines.extend(render_skills_free_text(
                &format!("  {} · ", problem.name),
                &problem.reason,
                &format!(" · {}", problem.path),
                width,
                theme,
            ));
        }
    }
    lines
}

/// Wraps the complete row before styling only its free-text field.
fn render_skills_free_text(
    prefix: &str,
    text: &str,
    suffix: &str,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let content = format!("{prefix}{text}{suffix}");
    let text_end = prefix.len() + text.len();
    let mut offset = 0;
    let mut lines = Vec::new();
    for raw_line in content.split_inclusive('\n') {
        let raw = raw_line
            .strip_suffix('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .unwrap_or(raw_line);
        let mut row_offset = offset;
        for wrapped in wrap_text(raw, usize::from(width).max(1)) {
            let prefix_end = prefix.len().saturating_sub(row_offset).min(wrapped.len());
            let free_end = text_end.saturating_sub(row_offset).min(wrapped.len());
            let mut spans =
                render_inline_markdown(&wrapped[..prefix_end], theme.style(Tone::Default), theme);
            if prefix_end < free_end {
                spans.extend(render_inline_markdown(
                    &wrapped[prefix_end..free_end],
                    theme.style(Tone::Default),
                    theme,
                ));
            }
            if free_end < wrapped.len() {
                spans.push(Span::styled(
                    wrapped[free_end..].to_owned(),
                    theme.style(Tone::Default),
                ));
            }
            row_offset += wrapped.len();
            lines.push(Line::from(spans));
        }
        offset += raw_line.len();
    }
    lines
}

/// Wraps free text before applying inline Markdown.
fn render_inline_text_lines(content: &str, width: u16, theme: Theme) -> Vec<Line<'static>> {
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
    let commands = report
        .getting_started
        .iter()
        .chain(&report.primary)
        .chain(&report.advanced);
    let names = commands
        .map(|command| format!("/{}", command.name))
        .collect::<Vec<_>>();
    let name_width = reports::label_width(names.iter().map(String::as_str), width);
    let mut lines = reports::text(
        HelpReport::START_HERE_HEADING,
        width,
        theme.style(Tone::Heading),
    );
    lines.extend(reports::text(
        &report.introduction,
        width,
        theme.style(Tone::Dim),
    ));
    for command in &report.getting_started {
        lines.extend(render_help_command(command, name_width, width, theme));
    }
    for (heading, commands) in [
        (HelpReport::PRIMARY_HEADING, &report.primary),
        (HelpReport::ADVANCED_HEADING, &report.advanced),
    ] {
        lines.push(Line::default());
        lines.extend(reports::text(heading, width, theme.style(Tone::Heading)));
        for command in commands {
            lines.extend(render_help_command(command, name_width, width, theme));
        }
    }
    lines.push(Line::default());
    lines.extend(reports::text(
        HelpReport::KEYS_HEADING,
        width,
        theme.style(Tone::Heading),
    ));
    let key_width = reports::label_width(report.keys.iter().map(|key| key.key.as_str()), width);
    for key in &report.keys {
        lines.extend(report_field(
            &key.key,
            &key.description,
            Tone::Default,
            key_width,
            width,
            theme,
        ));
    }
    lines
}

fn render_help_command(
    command: &HelpCommand,
    name_width: usize,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let mut lines = reports::field(
        Line::from(Span::styled(
            format!("/{}", command.name),
            theme.style(Tone::Code),
        )),
        &command.description,
        theme.style(Tone::Dim),
        name_width,
        width,
    );
    if !command.argument_hint.is_empty() {
        lines.extend(reports::field(
            Line::default(),
            &command.argument_hint,
            theme.style(Tone::Dim),
            name_width,
            width,
        ));
    }
    lines
}

pub(super) fn render_context_report(
    report: &ContextReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let mut fields = vec![
        ("model window", report.model_window.clone(), Tone::Default),
        ("counting", report.counting.clone(), Tone::Warning),
        (
            "compaction",
            report.compaction.render_value(),
            match report.compaction {
                ContextCompaction::Applied { .. } => Tone::Success,
                ContextCompaction::Enabled { .. } => Tone::Default,
            },
        ),
        ("tool context", report.tool_context.clone(), Tone::Default),
        (
            "provider input (session)",
            report.provider_input.clone(),
            Tone::Default,
        ),
        (
            "cache read (session)",
            report.cache_read.clone(),
            Tone::Default,
        ),
        ("cache", report.cache.clone(), Tone::Default),
        ("reasoning", report.reasoning.clone(), Tone::Default),
        (
            "reasoning controls",
            report.reasoning_controls.clone(),
            Tone::Default,
        ),
    ];
    if !report.available_windows.is_empty() {
        fields.insert(
            0,
            (
                "available context windows",
                report.available_windows_value(),
                Tone::Default,
            ),
        );
    }
    let categories = report
        .categories
        .iter()
        .map(|category| {
            (
                category.kind,
                category.label.as_str(),
                category.value.as_str(),
            )
        })
        .chain([
            (
                ContextCategoryKind::Free,
                "free input",
                report.free_input.value.as_str(),
            ),
            (
                ContextCategoryKind::Reserve,
                "output/reasoning reserve",
                report.reserve.value.as_str(),
            ),
        ])
        .collect::<Vec<_>>();
    let labels = categories
        .iter()
        .map(|(_, label, _)| format!("  {label}"))
        .chain(fields.iter().map(|(label, _, _)| (*label).to_owned()))
        .collect::<Vec<_>>();
    let label_width = reports::label_width(labels.iter().map(String::as_str), width);
    let mut lines = reports::text(ContextReport::HEADING, width, theme.style(Tone::Heading));
    if !report.available_windows.is_empty() {
        let (label, value, tone) = fields.remove(0);
        lines.extend(report_field(label, &value, tone, label_width, width, theme));
    }
    lines.extend(reports::text(
        &report.summary,
        width,
        theme.style(Tone::Dim),
    ));
    lines.push(Line::default());
    for row in report.grid() {
        let mut spans = vec![Span::raw("  ")];
        for (index, kind) in row.into_iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw(" "));
            }
            let (glyph, tone) = context_category_style(kind);
            spans.push(Span::styled(glyph, theme.style(tone)));
        }
        if report.grid_is_empty() {
            spans.push(Span::raw(" "));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::default());
    lines.extend(reports::text(
        report.usage.category_heading(),
        width,
        theme.style(Tone::Dim).add_modifier(Modifier::ITALIC),
    ));
    for (kind, label, value) in categories {
        let (glyph, tone) = context_category_style(kind);
        lines.extend(reports::field(
            Line::from(vec![
                Span::styled(format!("{glyph} "), theme.style(tone)),
                Span::styled(label.to_owned(), theme.style(Tone::Default)),
            ]),
            value,
            theme.style(Tone::Dim),
            label_width,
            width,
        ));
    }
    for (label, value, tone) in fields {
        lines.extend(report_field(label, &value, tone, label_width, width, theme));
        if label == "tool context" {
            lines.extend(reports::text(
                ContextReport::OCCUPANCY_HINT,
                width,
                theme.style(Tone::Dim),
            ));
        }
    }
    lines
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

#[cfg(test)]
mod cache_tests {
    include!("tests/cache.rs");
}
