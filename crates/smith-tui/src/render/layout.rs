//! Surface layout and rendering implementation.
//!
//! Drawing is a pure function of [`App`] plus the frame area. Nothing here
//! mutates state or caches across frames, which is what makes resize correct by
//! construction: every wrap is recomputed at the new width.

use agent_runtime_core::event::PlanSensitivity;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::{App, Overlay};
use crate::picker::{compact_resource_picker_rows, draw_compact_resource_picker};
use crate::theme::{Theme, Tone, glyph};
#[cfg(test)]
use crate::transcript::MAX_LOCAL_RESULT_BYTES;

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
        app.composer_pointer_area = None;
        draw_too_small(frame, area, theme);
        return;
    }

    let (transcript, composer) = surface_rects(area, app);
    // The same inset `draw_composer` applies, recorded so mouse clicks can be
    // mapped back onto composer text.
    app.composer_pointer_area = Some(Rect::new(
        composer.x,
        composer.y.saturating_add(1),
        composer.width,
        composer.height.saturating_sub(2),
    ));
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
    let [transcript, compact, pending, todos, composer, hint] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(anchored.compact),
        Constraint::Length(anchored.pending),
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
    draw_composer(frame, composer, app, theme);
    draw_hint(frame, hint, app, theme);

    match &app.overlay {
        Some(Overlay::Approval { prompt, review }) => {
            draw_approval(frame, area, prompt, review.as_ref(), theme);
        }
        Some(Overlay::Questionnaire { state }) => {
            draw_questionnaire(frame, area, state, theme);
        }
        Some(
            Overlay::Palette { .. }
            | Overlay::ResourcePicker { .. }
            | Overlay::HistorySearch { .. },
        ) => {}
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

/// The transcript and composer rects under the same vertical layout
/// `draw_surface` renders with.
fn surface_rects(area: Rect, app: &App) -> (Rect, Rect) {
    let composer_rows = composer_rows(app, area.width).saturating_add(2);
    let anchored = anchored_rows(app, area, composer_rows);
    let [transcript, _, _, _, composer, _] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(anchored.compact),
        Constraint::Length(anchored.pending),
        Constraint::Length(anchored.todos),
        Constraint::Length(composer_rows),
        Constraint::Length(hint_rows(app)),
    ])
    .areas(area);
    (transcript, composer)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AnchoredRows {
    compact: u16,
    pending: u16,
    todos: u16,
}

fn anchored_rows(app: &App, area: Rect, composer_rows: u16) -> AnchoredRows {
    let compact_desired = match &app.overlay {
        Some(Overlay::Palette { error, .. }) => desired_palette_rows(app, error.as_deref()),
        Some(Overlay::ResourcePicker { picker, .. }) => compact_resource_picker_rows(picker),
        Some(Overlay::HistorySearch { .. }) => 2,
        _ => 0,
    };
    let available = area
        .height
        .saturating_sub(composer_rows)
        .saturating_sub(hint_rows(app))
        .saturating_sub(3);
    let pending_desired = desired_pending_input_rows(app);
    let todo_desired = desired_todo_rows(app);
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
    u16::try_from(items.len().min(MAX_VISIBLE_TODOS).saturating_add(1)).unwrap_or(u16::MAX)
}

fn hint_rows(app: &App) -> u16 {
    if has_stacked_control_hint(app) { 2 } else { 1 }
}

pub(super) fn has_stacked_control_hint(app: &App) -> bool {
    matches!(app.overlay, Some(Overlay::ResourcePicker { .. }))
}

fn draw_too_small(frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let text = format!("terminal too small (need {MIN_WIDTH}×{MIN_HEIGHT})");
    frame.render_widget(Paragraph::new(text).style(theme.style(Tone::Warning)), area);
}

fn composer_rows(app: &App, width: u16) -> u16 {
    // Mirrors `draw_composer`'s marker prefix so the height budget counts the
    // same lines the renderer wraps.
    let lines: Vec<Line<'static>> = app
        .composer
        .lines()
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let marker = if index == 0 { glyph::USER } else { " " };
            Line::from(format!("{marker} {line}"))
        })
        .collect();
    u16::try_from(rendered_rows(&lines, width).clamp(1, 8)).unwrap_or(1)
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
