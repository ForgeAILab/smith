//! Surface layout and rendering implementation.
//!
//! Drawing is a pure function of [`App`] plus the frame area. Nothing here
//! mutates application state. Render-only cached rows are keyed by width, so
//! resizing refreshes their wrapping before layout or drawing uses them.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
#[cfg(test)]
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use smith_runtime::client::{PlanItemStatus, PlanSensitivity};

use crate::app::{App, Overlay};
use crate::picker::{compact_resource_picker_rows, draw_compact_resource_picker};
use crate::theme::{Theme, Tone};
#[cfg(test)]
use crate::transcript::MAX_LOCAL_RESULT_BYTES;

use super::approval::{approval_scroll_limit, draw_approval};
use super::composer::*;
use super::modal::*;
use super::transcript::*;

/// The smallest terminal Smith will draw a working surface in. Below this a
/// half-rendered coding surface is worse than an honest refusal.
pub const MIN_WIDTH: u16 = 40;
/// See [`MIN_WIDTH`].
pub const MIN_HEIGHT: u16 = 10;

/// The most body lines a modal builds before the height budget takes over.
///
/// A pathological argument blob would otherwise cost a `Line` per JSON line on
/// every frame, and no terminal is tall enough to show them.
pub(super) const MAX_BODY_LINES: usize = 128;

/// Maximum public todo items kept immediately above the composer.
const MAX_VISIBLE_TODOS: usize = 5;

/// Draws the whole client without changing application state.
///
/// Interactive hosts should apply [`layout()`] before drawing so scroll input
/// is bounded by the current terminal geometry. This pure entry point remains
/// useful for snapshots and other read-only renderers.
pub fn draw(frame: &mut Frame<'_>, app: &App, theme: Theme) {
    draw_surface(frame, app, theme);
}

/// Scroll state computed for the current surface geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceLayout {
    transcript: Option<TranscriptScroll>,
    approval: Option<(u16, u16)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TranscriptScroll {
    limit: usize,
    back: usize,
    following: bool,
    to_block: Option<usize>,
    result_revision: Option<u64>,
}

impl SurfaceLayout {
    /// Applies the computed bounds before the application is drawn read-only.
    pub fn apply(self, app: &mut App) {
        if let Some(scroll) = self.transcript {
            app.scroll_limit = scroll.limit;
            app.scroll_back = scroll.back;
            app.following = scroll.following;
            app.scroll_to_block = scroll.to_block;
            app.result_scroll_revision = scroll.result_revision;
        }
        if let Some((scroll, limit)) = self.approval {
            app.approval_scroll = scroll;
            app.approval_scroll_limit = limit;
        }
    }
}

/// Computes viewport bounds and follow/result positioning without changing App.
pub fn layout(area: Rect, app: &App, theme: Theme) -> SurfaceLayout {
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        return SurfaceLayout {
            transcript: None,
            approval: None,
        };
    }
    let transcript = transcript_rect(area, app);
    let rows = transcript_rows(app, theme, transcript.width);
    let limit = rows.scroll_limit(transcript);
    let newest = app.inspected_child.is_none() && app.output_after_result();
    let mut scroll = TranscriptScroll {
        limit,
        back: app.scroll_back,
        following: app.following,
        to_block: app.scroll_to_block,
        result_revision: app.result_scroll_revision,
    };
    if newest {
        scroll.back = 0;
        scroll.following = true;
        scroll.to_block = None;
        scroll.result_revision = None;
    }
    if scroll.following {
        scroll.back = 0;
    } else {
        // Preserve the row at the top while output grows or the width changes.
        let offset = app
            .scroll_limit
            .saturating_sub(scroll.back.min(app.scroll_limit));
        scroll.back = limit.saturating_sub(offset.min(limit));
        if limit == 0 && scroll.result_revision.is_none() {
            scroll.following = true;
            scroll.to_block = None;
        } else if scroll.back == 0 && scroll.result_revision.is_none() {
            scroll.following = true;
        }
    }
    if app.inspected_child.is_none()
        && let Some(block) = scroll.to_block.take()
        && let Some(offset) = rows.block_start_row(block)
    {
        scroll.back = limit.saturating_sub(offset);
        scroll.following = scroll.back == 0;
        if scroll.following {
            scroll.result_revision = None;
        }
    }
    let approval = matches!(app.overlay, Some(Overlay::Approval { .. })).then(|| {
        let limit = approval_scroll_limit(area, app, theme);
        (app.approval_scroll.min(limit), limit)
    });
    SurfaceLayout {
        transcript: Some(scroll),
        approval,
    }
}

/// Computes and applies layout, then draws the client read-only.
pub fn draw_synced(frame: &mut Frame<'_>, app: &mut App, theme: Theme) {
    layout(frame.area(), app, theme).apply(app);
    draw(frame, app, theme);
}

fn draw_surface(frame: &mut Frame<'_>, app: &App, theme: Theme) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        draw_too_small(frame, area, theme);
        return;
    }

    let composer_rows = composer_rows(app, area.width).saturating_add(2);
    let agents_rows = agents_rows(app, area, composer_rows);
    let anchored = anchored_rows(app, area, composer_rows, agents_rows);
    let [
        transcript,
        compact,
        pending,
        todos,
        working,
        composer,
        hint,
        agents,
    ] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(anchored.compact),
        Constraint::Length(anchored.pending),
        Constraint::Length(anchored.todos),
        Constraint::Length(working_rows(app)),
        Constraint::Length(composer_rows),
        Constraint::Length(hint_rows(app)),
        Constraint::Length(agents_rows),
    ])
    .areas(area);
    draw_transcript(frame, transcript, app, theme);
    if anchored.compact > 0 {
        match &app.overlay {
            Some(Overlay::Shortcuts) => draw_shortcuts(frame, compact, theme),
            Some(Overlay::Palette {
                selected, error, ..
            }) => draw_palette(frame, compact, app, *selected, error.as_deref(), theme),
            Some(Overlay::ResourcePicker { picker, .. }) => {
                draw_compact_resource_picker(frame, compact, picker, theme);
            }
            Some(Overlay::HistorySearch { query, matched, .. }) => {
                draw_history_search(frame, compact, query, matched.as_deref(), theme);
            }
            _ => unreachable!("compact rows require a compact interaction"),
        }
    }
    if anchored.pending > 0 {
        draw_pending_input(frame, pending, app, theme);
    }
    if anchored.todos > 0 {
        draw_todos(frame, todos, app, theme);
    }
    if working_rows(app) > 0 {
        draw_working(frame, working, app, theme);
    }
    draw_composer(frame, composer, app, theme);
    draw_hint(frame, hint, app, theme);
    if agents_rows > 0 {
        draw_agents(frame, agents, app, theme);
    }

    match &app.overlay {
        Some(Overlay::Approval { .. }) => draw_approval(frame, area, app, theme),
        Some(Overlay::Questionnaire { state }) => {
            draw_questionnaire(frame, area, state, theme);
        }
        Some(
            Overlay::Palette { .. }
            | Overlay::ResourcePicker { .. }
            | Overlay::HistorySearch { .. }
            | Overlay::Shortcuts,
        ) => {}
        Some(Overlay::UndoConfirm { report }) => {
            draw_recovery_confirm(
                frame,
                area,
                "undo last Smith turn",
                "apply undo",
                render_recovery_patch(&report.patch),
                theme,
            );
        }
        Some(Overlay::RedoConfirm { report }) => {
            draw_redo_confirm(frame, area, report, theme);
        }
        Some(Overlay::RevertConfirm { report }) => {
            draw_recovery_confirm(
                frame,
                area,
                "revert selected change",
                "apply revert",
                render_revert_preview(report),
                theme,
            );
        }
        Some(Overlay::ReviewConfirm { report }) => {
            draw_review_confirm(frame, area, report, theme);
        }
        Some(Overlay::McpTrustConfirm { server, content }) => {
            draw_mcp_trust_confirm(frame, area, server, content, theme);
        }
        Some(Overlay::SkillTrustConfirm { skill, content }) => {
            draw_skill_trust_confirm(frame, area, skill, content, theme);
        }
        Some(Overlay::RotationConfirm { content, .. }) => {
            draw_rotation_confirm(frame, area, content, theme);
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

    // Last, over everything: the highlight belongs on whatever the user can
    // actually see, and painting it before the overlays would let a modal
    // cover the very cells it marks.
    paint_selection(frame, app, theme);
}

/// Reads the highlighted text out of a drawn frame.
///
/// Must be called on a frame `draw_synced` has already filled, from inside the
/// same draw closure: the selection addresses rendered cells, and the buffer is
/// the only place their text exists. Returns `None` when the drag covered
/// nothing but blank cells.
pub fn selected_text(frame: &mut Frame<'_>, app: &App) -> Option<String> {
    let selection = app.selection.as_ref()?;
    let area = frame.area();
    if selection.stale_after_redraw(area) {
        return None;
    }
    crate::selection::text_from_buffer(selection, frame.buffer_mut(), area)
}

/// Marks the selected cells in place, over the finished frame.
///
/// Restyling drawn cells rather than reserving a widget is what lets a
/// selection cross the transcript, composer, and hint row in one drag: it
/// applies to the surface as rendered, with no widget needing to know it
/// exists.
fn paint_selection(frame: &mut Frame<'_>, app: &App, theme: Theme) {
    let Some(selection) = &app.selection else {
        return;
    };
    let area = frame.area();
    if selection.stale_after_redraw(area) {
        return;
    }
    let style = theme.selection();
    let buffer = frame.buffer_mut();
    for row in area.y..area.y.saturating_add(area.height) {
        let Some(span) = selection.span_on_row(row, area) else {
            continue;
        };
        // Snapped to whole glyphs so a drag that stops on the second cell of a
        // wide character still highlights the character the copy will take.
        let (from, to) = crate::selection::snap_span_to_glyphs(buffer, area, row, span);
        for column in from..to {
            // Patched, not replaced: the cell keeps its own color and the
            // highlight reads as a highlight rather than a repaint.
            buffer[(column, row)].set_style(style);
        }
    }
}

/// The transcript rect under the same vertical layout `draw_surface` renders
/// with.
fn transcript_rect(area: Rect, app: &App) -> Rect {
    let composer_rows = composer_rows(app, area.width).saturating_add(2);
    let agents_rows = agents_rows(app, area, composer_rows);
    let anchored = anchored_rows(app, area, composer_rows, agents_rows);
    let [transcript, _, _, _, _, _, _, _] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(anchored.compact),
        Constraint::Length(anchored.pending),
        Constraint::Length(anchored.todos),
        Constraint::Length(working_rows(app)),
        Constraint::Length(composer_rows),
        Constraint::Length(hint_rows(app)),
        Constraint::Length(agents_rows),
    ])
    .areas(area);
    transcript
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AnchoredRows {
    compact: u16,
    pending: u16,
    todos: u16,
}

fn desired_compact_rows(app: &App, width: u16) -> u16 {
    match &app.overlay {
        Some(Overlay::Shortcuts) => {
            u16::try_from(shortcuts_lines(width, Theme::new()).len().saturating_add(1))
                .unwrap_or(u16::MAX)
        }
        Some(Overlay::Palette {
            selected, error, ..
        }) => desired_palette_rows(app, *selected, error.as_deref()),
        Some(Overlay::ResourcePicker { picker, .. }) => compact_resource_picker_rows(picker),
        Some(Overlay::HistorySearch { .. }) => 2,
        _ => 0,
    }
}

fn anchored_rows(app: &App, area: Rect, composer_rows: u16, agents_rows: u16) -> AnchoredRows {
    let compact_desired = desired_compact_rows(app, area.width);
    let available = area
        .height
        .saturating_sub(composer_rows)
        .saturating_sub(hint_rows(app))
        .saturating_sub(agents_rows)
        .saturating_sub(working_rows(app))
        .saturating_sub(3);
    let pending_desired = desired_pending_input_rows(app);
    let todo_desired = desired_todo_rows(app);
    if matches!(app.overlay, Some(Overlay::Approval { .. })) {
        return AnchoredRows {
            compact: 0,
            pending: 0,
            todos: 0,
        };
    }
    let (compact, pending, todos) = if compact_desired > 0 {
        // Compact interactions temporarily own the anchored pane. The todo
        // projection remains in App state and returns unchanged when the
        // interaction closes.
        (compact_desired.min(available), 0, 0)
    } else {
        let anchored_available = available.min(9);
        if pending_desired > 0 && todo_desired > 0 {
            let pending = pending_desired.min(5).min(anchored_available);
            let todos = todo_desired.min(anchored_available.saturating_sub(pending));
            (0, pending, todos)
        } else if pending_desired > 0 {
            (0, pending_desired.min(anchored_available), 0)
        } else {
            (0, 0, todo_desired.min(anchored_available))
        }
    };
    AnchoredRows {
        compact,
        pending,
        todos,
    }
}

fn desired_pending_input_rows(app: &App) -> u16 {
    let rows = app
        .pending_input_previews()
        .iter()
        .map(|section| section.entries.len() + usize::from(section.overflow > 0))
        .sum::<usize>();
    u16::try_from(rows.min(9)).unwrap_or(9)
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
    // The pane retires once every item is completed and the turn is no
    // longer running. `draw_todos` in `composer.rs` reaches the same verdict
    // from the same two facts, so this reserves no row it would leave blank.
    if !app.is_busy()
        && items
            .iter()
            .all(|item| item.status == PlanItemStatus::Completed)
    {
        return 0;
    }
    // The collapsed completed row is charged against the same budget an
    // uncollapsed item would have used, so the anchored row budget stays
    // exactly what it costs today: open items plus at most one row standing
    // in for every completed item, capped at `MAX_VISIBLE_TODOS`.
    let open_count = items
        .iter()
        .filter(|item| item.status != PlanItemStatus::Completed)
        .count();
    let collapsed_row = usize::from(
        items
            .iter()
            .any(|item| item.status == PlanItemStatus::Completed),
    );
    let visible = open_count
        .saturating_add(collapsed_row)
        .min(MAX_VISIBLE_TODOS);
    u16::try_from(visible.saturating_add(1)).unwrap_or(u16::MAX)
}

fn hint_rows(app: &App) -> u16 {
    if has_stacked_control_hint(app) { 2 } else { 1 }
}

/// Agents-panel rows, bounded so the transcript keeps its minimum height.
fn agents_rows(app: &App, area: Rect, composer_rows: u16) -> u16 {
    let ceiling = area
        .height
        .saturating_sub(composer_rows)
        .saturating_sub(hint_rows(app))
        .saturating_sub(working_rows(app))
        // A list owns the anchored pane; agent chrome yields before its five
        // choices and selected detail line on a short terminal.
        .saturating_sub(desired_compact_rows(app, area.width))
        .saturating_sub(3);
    desired_agents_rows(app).min(ceiling)
}

pub(super) fn has_stacked_control_hint(app: &App) -> bool {
    matches!(app.overlay, Some(Overlay::ResourcePicker { .. }))
}

fn draw_too_small(frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let text = format!("terminal too small (need {MIN_WIDTH}×{MIN_HEIGHT})");
    frame.render_widget(Paragraph::new(text).style(theme.style(Tone::Warning)), area);
}

fn composer_rows(app: &App, width: u16) -> u16 {
    let (lines, _) = composer_content(app, Theme::new(), width);
    u16::try_from(lines.len().clamp(1, 8)).unwrap_or(1)
}

// ---------------------------------------------------------------------------
// Transcript
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Composer and hint
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Consequential overlays
// ---------------------------------------------------------------------------

include!("tests/mod.rs");
