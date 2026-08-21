//! Composer, pending input, todos, hints, and identity footer.

use std::time::Duration;

use crate::app::{
    App, ChildCounts, ChildSummary, MAX_PENDING_PREVIEW_ENTRIES, Overlay, RunningTaskSummary,
};
use crate::status::{TokenCount, render_elapsed};
use crate::theme::{Theme, Tone, glyph};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use smith_runtime::client::{PlanItemProjection, PlanItemStatus};
use unicode_width::UnicodeWidthStr;

use super::helpers::*;
use super::layout::*;

pub(super) fn draw_composer(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
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
                spans.extend(paste_placeholder_spans(line, app, theme));
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

/// Splits one composer line so registered paste placeholders render accented,
/// making the collapsed chunk visually distinct from typed text.
pub(super) fn paste_placeholder_spans(line: &str, app: &App, theme: Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = line;
    loop {
        let earliest = app
            .attachment_placeholders()
            .filter_map(|placeholder| {
                rest.find(placeholder)
                    .map(|found| (found, placeholder.len()))
            })
            .min_by_key(|(found, _)| *found);
        match earliest {
            Some((found, length)) => {
                if found > 0 {
                    spans.push(Span::raw(rest[..found].to_owned()));
                }
                spans.push(Span::styled(
                    rest[found..found + length].to_owned(),
                    theme.style(Tone::Accent),
                ));
                rest = &rest[found + length..];
            }
            None => {
                if !rest.is_empty() {
                    spans.push(Span::raw(rest.to_owned()));
                }
                break;
            }
        }
    }
    spans
}

pub(super) fn draw_pending_input(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let sections = app.pending_input_previews();
    let mut lines = Vec::new();
    for section in &sections {
        let Some(first) = section.entries.first() else {
            continue;
        };
        let preview = first.split_whitespace().collect::<Vec<_>>().join(" ");
        lines.push(Line::from(vec![
            Span::styled(format!("  {}", section.label), theme.style(Tone::Heading)),
            Span::styled(" · process-local · ", theme.style(Tone::Dim)),
            Span::styled(preview, theme.style(Tone::Default)),
        ]));
    }
    for index in 1..MAX_PENDING_PREVIEW_ENTRIES {
        for section in &sections {
            if let Some(entry) = section.entries.get(index) {
                lines.push(Line::from(vec![
                    Span::styled("  › ", theme.style(Tone::Dim)),
                    Span::styled(
                        entry.split_whitespace().collect::<Vec<_>>().join(" "),
                        theme.style(Tone::Default),
                    ),
                ]));
            }
        }
    }
    for section in &sections {
        if section.overflow > 0 {
            lines.push(Line::from(Span::styled(
                format!(
                    "    +{} more {}",
                    section.overflow,
                    section.label.to_lowercase()
                ),
                theme.style(Tone::Dim),
            )));
        }
    }
    lines.truncate(usize::from(area.height));
    // Pending rows are fixed-height composer chrome. They clip horizontally
    // and never wrap into the composer or claim canonical transcript space.
    frame.render_widget(Paragraph::new(lines), area);
}

pub(super) fn draw_todos(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let Some(items) = app.plan.as_ref().and_then(|plan| plan.items.as_ref()) else {
        return;
    };
    // The pane retires once every item is completed and the turn is no
    // longer running, rather than pinning a finished list until the next
    // turn starts. `desired_todo_rows` in `layout.rs` reaches the same
    // verdict from the same two facts, so the reserved area and what this
    // function draws into it never disagree.
    if !app.is_busy()
        && items
            .iter()
            .all(|item| item.status == PlanItemStatus::Completed)
    {
        return;
    }
    let capacity = usize::from(area.height).saturating_sub(1);
    let open = items
        .iter()
        .filter(|item| item.status != PlanItemStatus::Completed);
    // `PlanItemProjection` carries no completion timestamp, so "most
    // recently completed" is read as the last completed item in authored
    // order — the only ordering the projection actually carries. This walk
    // also counts every completed item, so the collapsed row can report how
    // many sit behind the one it names.
    let mut completed_count = 0usize;
    let mut most_recently_completed = None;
    for item in items {
        if item.status == PlanItemStatus::Completed {
            completed_count += 1;
            most_recently_completed = Some(item);
        }
    }

    let mut lines = vec![Line::from(Span::styled(
        "  Todo",
        theme.style(Tone::Heading),
    ))];
    lines.extend(open.take(capacity).map(|item| todo_item_line(item, theme)));
    if let Some(item) = most_recently_completed {
        // Charged against the same capacity an open item would have used, so
        // the anchored row budget the collapsed row draws from is unchanged.
        if lines.len().saturating_sub(1) < capacity {
            lines.push(collapsed_todo_line(
                item,
                completed_count.saturating_sub(1),
                theme,
            ));
        }
    }
    // Todo rows are fixed-height composer chrome; long text clips instead of
    // wrapping and moving the input.
    frame.render_widget(Paragraph::new(lines), area);
}

/// One open (pending, in-progress, or cancelled) todo row.
///
/// A completed item reaches [`collapsed_todo_line`] instead, so the completed
/// arm here is unused today. It is kept total rather than `unreachable!`
/// anyway: this runs inside the draw path, and a panic there takes the whole
/// terminal session down mid-frame — a far worse outcome than one row drawn
/// the way it was drawn before the collapse existed.
fn todo_item_line(item: &PlanItemProjection, theme: Theme) -> Line<'static> {
    let (marker, tone) = match item.status {
        PlanItemStatus::Pending => ("[ ]", Tone::Default),
        PlanItemStatus::InProgress => ("[>]", Tone::Accent),
        PlanItemStatus::Cancelled => ("[-]", Tone::Dim),
        PlanItemStatus::Completed => ("[x]", Tone::Success),
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
}

/// The single collapsed row standing in for every completed item: the most
/// recently completed item's text, struck through and dim, naming how many
/// more are hidden behind it when more than one item is complete.
fn collapsed_todo_line(item: &PlanItemProjection, hidden: usize, theme: Theme) -> Line<'static> {
    let text = item.text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = if text.is_empty() {
        item.id.clone()
    } else {
        text
    };
    let struck = theme.style(Tone::Dim).add_modifier(Modifier::CROSSED_OUT);
    let mut spans = vec![
        Span::styled("  [x] ", theme.style(Tone::Dim)),
        Span::styled(text, struck),
    ];
    if hidden > 0 {
        spans.push(Span::styled(
            format!(" (+{hidden} done)"),
            theme.style(Tone::Dim),
        ));
    }
    Line::from(spans)
}

pub(super) fn overlay_hint(app: &App) -> Option<String> {
    match &app.overlay {
        Some(Overlay::Approval { .. }) => {
            let waiting = app.pending_approval_count().saturating_sub(1);
            Some(if waiting == 0 {
                "y allow once · a allow this target · n deny".to_owned()
            } else {
                format!("y allow once · a allow this target · n deny · {waiting} queued")
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
        Some(Overlay::HistorySearch { .. }) => {
            Some("ctrl+r older · enter use · esc cancel".to_owned())
        }
        Some(Overlay::UndoConfirm { .. }) => Some("y apply undo · n/esc cancel".to_owned()),
        Some(Overlay::RedoConfirm { .. }) => Some("y apply redo · n/esc cancel".to_owned()),
        Some(Overlay::RevertConfirm { .. }) => Some("y apply revert · n/esc cancel".to_owned()),
        Some(Overlay::ReviewConfirm { .. }) => Some("y start review · n/esc cancel".to_owned()),
        Some(Overlay::McpTrustConfirm { .. }) => {
            Some("y trust and connect · n/esc leave untrusted".to_owned())
        }
        Some(Overlay::SkillTrustConfirm { .. }) => {
            Some("y trust and activate · n/esc leave withheld".to_owned())
        }
        Some(Overlay::RotationConfirm { prompt, .. }) => {
            Some(if prompt.request().eligible.len() > 1 {
                "y switch and resend · 1-9 choose account · n/esc stay".to_owned()
            } else {
                "y switch and resend · n/esc stay".to_owned()
            })
        }
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
        // The inspector is a read-only view, not an overlay, but it owns the
        // same keys the identity footer would otherwise explain: while it is
        // open, the hint row says how to leave it and how to reply to the
        // child being read.
        None => app
            .inspected_child
            .as_ref()
            .map(|child| format!("↑↓ agents · esc back to main · enter continues {child}")),
    }
}

pub(super) fn draw_hint(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
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

pub(super) fn draw_control_hint(frame: &mut Frame<'_>, area: Rect, hint: &str, theme: Theme) {
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

pub(super) fn draw_identity_footer(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    // Identity lives at the point of action. During work, activity replaces
    // the idle mode label; no permanent header or focusable status region is
    // introduced.
    if app.ctrl_c_exit_hint_active() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  ", theme.style(Tone::Dim)),
                Span::styled("press Ctrl+C again to exit", theme.style(Tone::Warning)),
            ])),
            area,
        );
        return;
    }

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
    if area.width >= 100
        && let Some(reasoning) = &app.status.reasoning_hint
    {
        push_segment(
            &mut identity,
            theme,
            Span::styled(reasoning.clone(), theme.style(Tone::Reasoning)),
        );
    }
    if let Some(account) = app.status.render_account_footer() {
        push_segment(
            &mut identity,
            theme,
            Span::styled(account, theme.style(Tone::Dim)),
        );
    }
    if let Some(mcp) = app.status.mcp.render_footer() {
        push_segment(
            &mut identity,
            theme,
            Span::styled(
                mcp,
                theme.style(if app.status.mcp.failed > 0 {
                    Tone::Warning
                } else {
                    Tone::Dim
                }),
            ),
        );
    }
    if let Some(goal) = app.status.render_goal_footer() {
        push_segment(
            &mut identity,
            theme,
            Span::styled(goal, theme.style(Tone::Accent)),
        );
    }
    if let Some(tasks) = app.render_running_tasks_footer() {
        push_segment(
            &mut identity,
            theme,
            Span::styled(tasks, theme.style(Tone::Dim)),
        );
    }
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
                    plan.confidence == smith_runtime::client::EstimationConfidence::Exact
                }) {
                    Tone::Default
                } else {
                    Tone::Warning
                },
            ),
        ),
    );
    // CH is a completed-root-turn metric. Keep the pre-turn footer unchanged,
    // then show explicit `0%` or honest `?` once a canonical turn rollup
    // exists.
    if app.status.cache_projection.latest_completed().is_some() {
        let ch = app.status.render_cache_hit_rate();
        push_segment(
            &mut identity,
            theme,
            Span::styled(
                format!("CH {ch}"),
                theme.style(if ch == "?" { Tone::Warning } else { Tone::Dim }),
            ),
        );
    }
    if app.is_busy() {
        let mut controls = if app.composer.is_blank() {
            "Enter steer · Tab queue when typed · Esc interrupt".to_owned()
        } else {
            "Enter steer · Tab queue · Esc interrupt".to_owned()
        };
        if app.has_editable_queued_turn() {
            controls.push_str(" · Alt+↑ edit queue");
        }
        push_segment(
            &mut identity,
            theme,
            Span::styled(controls, theme.style(Tone::Dim)),
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(truncate(identity, area.width))),
        area,
    );
}

// ---------------------------------------------------------------------------
// Delegated-agents panel
// ---------------------------------------------------------------------------

/// Maximum delegated-agent and background-task rows the panel shows before
/// eliding the rest.
pub(super) const MAX_VISIBLE_AGENTS: usize = 6;

/// Rows the agents panel wants: a spacer, the main row, one row per child
/// and running background task, and an elision row on overflow.
pub(super) fn desired_agents_rows(app: &App) -> u16 {
    let entries = app.visible_children().len() + app.running_tasks.len();
    if entries == 0 {
        return 0;
    }
    let rows = 2 + entries.min(MAX_VISIBLE_AGENTS) + usize::from(entries > MAX_VISIBLE_AGENTS);
    u16::try_from(rows).unwrap_or(u16::MAX)
}

/// Draws the delegated-work panel beneath the hint row.
///
/// One row per agent or background task the session knows about: marker,
/// identity, latest bounded activity, and a right-aligned clock. The `main`
/// row anchors the list so delegated work always reads relative to the
/// conversation the composer serves.
pub(super) fn draw_agents(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    if area.height == 0 || (desired_agents_rows(app) == 0) {
        return;
    }
    let mut lines = vec![Line::default()];
    let main_current = app.inspected_child.is_none();
    let (marker, style) = if main_current {
        (
            glyph::AGENT_CURRENT,
            theme.style(Tone::Default).add_modifier(Modifier::BOLD),
        )
    } else {
        (glyph::AGENT_OTHER, theme.style(Tone::Dim))
    };
    lines.push(Line::from(Span::styled(format!("  {marker} main"), style)));
    // Live rows first: when the panel elides, a finished child gives way to
    // a child or task still working.
    let children = app.visible_children();
    let (live, settled): (Vec<_>, Vec<_>) =
        children.iter().partition(|(_, summary)| summary.is_live());
    let rows = live
        .iter()
        .map(|(child_id, summary)| agent_row(app, child_id, summary, area.width, theme))
        .chain(
            app.running_tasks
                .iter()
                .map(|task| task_row(app, task, area.width, theme)),
        )
        .chain(
            settled
                .iter()
                .map(|(child_id, summary)| agent_row(app, child_id, summary, area.width, theme)),
        );
    lines.extend(rows.take(MAX_VISIBLE_AGENTS));
    let hidden = (children.len() + app.running_tasks.len()).saturating_sub(MAX_VISIBLE_AGENTS);
    if hidden > 0 {
        lines.push(Line::from(Span::styled(
            format!("    {} {hidden} more", glyph::ELIDED),
            theme.style(Tone::Dim),
        )));
    }
    lines.truncate(usize::from(area.height));
    // Agent rows are fixed-height chrome; long text clips instead of
    // wrapping so the composer never moves.
    frame.render_widget(Paragraph::new(lines), area);
}

fn agent_row(
    app: &App,
    child_id: &str,
    summary: &ChildSummary,
    width: u16,
    theme: Theme,
) -> Line<'static> {
    let current = app.inspected_child.as_deref() == Some(child_id);
    let marker = if current {
        glyph::AGENT_CURRENT
    } else {
        glyph::AGENT_OTHER
    };
    // While a child works its detail IS the activity; once it settles, the
    // lifecycle label carries the outcome and the detail explains it.
    let mut activity = match (summary.state.as_str(), &summary.detail) {
        ("running" | "working" | "resuming", Some(detail)) => detail.clone(),
        (state, Some(detail)) => format!("{state} {} {detail}", glyph::SEPARATOR),
        (state, None) => state.to_owned(),
    };
    // Everything this row adds beyond the child's bare id lands in the
    // activity text, which clips first — never in `identity` — so a long
    // profile, projection, or count can never push the docked clock off
    // screen; see `child-agents`'s "Delegated-work panel reports
    // substantive child activity".
    if let Some(profile) = &summary.profile {
        activity = format!("{profile} {} {activity}", glyph::SEPARATOR);
    }
    if let Some(counts) = app.child_counts(child_id) {
        activity = format!(
            "{activity} {} {}",
            glyph::SEPARATOR,
            render_child_counts(counts)
        );
    }
    // The selected row is already bold and marked; letting it keep its state
    // colour means the eye does not lose a failure by landing on it.
    let tone = child_state_tone(summary.state.as_str());
    let mut style = theme.style(tone);
    if current {
        style = style.add_modifier(Modifier::BOLD);
    }
    panel_row(
        marker,
        child_id,
        &activity,
        app.child_elapsed(child_id),
        style,
        width,
        theme,
    )
}

/// Coordinator-owned turn/token counts as a compact panel fragment.
///
/// Smith computes no count of its own here — `counts` already came from the
/// delegation coordinator on the host's poll-on-redraw; this only formats
/// what it was handed, the same unbounded-turn convention the transcript's
/// spawn enrichment uses.
fn render_child_counts(counts: ChildCounts) -> String {
    let turns = if counts.max_turns == u32::MAX {
        counts.turns_used.to_string()
    } else {
        format!("{}/{}", counts.turns_used, counts.max_turns)
    };
    format!(
        "{turns} turns · {} tokens",
        TokenCount::reported(counts.tokens_used).render()
    )
}

/// One background shell task's panel row: its stable id and command hint.
fn task_row(app: &App, task: &RunningTaskSummary, width: u16, theme: Theme) -> Line<'static> {
    panel_row(
        glyph::AGENT_OTHER,
        &task.task_id,
        &task.command_hint,
        app.task_elapsed(&task.task_id),
        theme.style(Tone::Default),
        width,
        theme,
    )
}

/// Lays out one panel row with the clock docked at the right edge,
/// Claude-style; the activity clips first so a long result can never push
/// the time off screen.
fn panel_row(
    marker: &str,
    identity: &str,
    activity: &str,
    elapsed: Option<Duration>,
    style: Style,
    width: u16,
    theme: Theme,
) -> Line<'static> {
    let activity = activity.split_whitespace().collect::<Vec<_>>().join(" ");
    let clock = elapsed.map(render_elapsed).unwrap_or_default();
    let width = usize::from(width);
    let reserved = if clock.is_empty() {
        0
    } else {
        clock.width() + 2
    };
    let left = clip_line(
        format!("  {marker} {identity}  {activity}"),
        width.saturating_sub(reserved),
    );
    let mut spans = vec![Span::styled(left.clone(), style)];
    if !clock.is_empty() {
        let pad = width.saturating_sub(left.width() + clock.width());
        spans.push(Span::styled(
            format!("{}{clock}", " ".repeat(pad)),
            theme.style(Tone::Dim),
        ));
    }
    Line::from(spans)
}
