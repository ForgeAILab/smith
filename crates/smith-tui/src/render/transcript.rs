//! Transcript, Markdown, tool, status, and local-result rendering.

mod agents;
mod blocks;
mod changes;
mod integrations;
mod local_reports;
mod previews;
mod text;

use std::borrow::Cow;
use std::cell::Ref;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use smith_client::commands::getting_started_commands;

use crate::app::App;
use crate::status::{Activity, render_elapsed};
use crate::theme::{Theme, Tone, glyph};
use crate::transcript::{Block, ToolStatus};

#[cfg(test)]
use crate::transcript::LocalResult;

use super::lists::clip_words;
use super::markdown::render_assistant_lines;
use super::wrap::{wrap_lines, wrapped_row_count};
use agents::child_lines;
#[cfg(test)]
use blocks::conversation_lines;
use blocks::{block_lines, suppressed_block};

pub(super) use crate::transcript::safe_tool_name;
pub(super) use local_reports::{render_context_report, render_help_report, render_status_card};
pub(super) use previews::{render_recovery_patch, render_revert_preview, render_review_preview};
pub(super) use text::{render_inline_markdown, render_prefixed_local_state};

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
                let suppressed = suppressed_block(block, app.work_details);
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
        tail.extend(render_assistant_lines(&text, theme, width, true));
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
    let visible = rows.window(offset, area.height);
    if theme.uses_hyperlinks() {
        frame.render_widget(
            Paragraph::new(
                visible
                    .iter()
                    .map(crate::hyperlink::plain_line)
                    .collect::<Vec<_>>(),
            ),
            area,
        );
        crate::hyperlink::apply(frame.buffer_mut(), area, &visible);
    } else {
        frame.render_widget(Paragraph::new(visible), area);
    }
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

#[cfg(test)]
mod cache_tests {
    include!("tests/cache.rs");
}
