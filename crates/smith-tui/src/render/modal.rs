//! Approval, questionnaire, palette, search, and confirmation overlays.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::{App, ConfirmDialog};
use crate::commands;
use crate::questionnaire::{QuestionnaireFocus, QuestionnaireState};
use crate::theme::{Theme, Tone, glyph};
use agent_runtime_core::clock::Deadline;
use agent_runtime_core::security::SecurityResource;
use agent_runtime_core::tool::PreparedToolCall;
use agent_runtime_registry::Permission;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WidgetBlock, Borders, Clear, Padding, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::helpers::*;
use super::layout::*;
use super::lists::{detail_line, list_row};
use super::transcript::rendered_rows;

/// Command completion keeps the transcript visible by reserving at most five
/// choices for the selected window, plus its optional argument detail line.
const MAX_VISIBLE_PALETTE_ROWS: usize = 5;

pub(super) fn draw_questionnaire(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &QuestionnaireState,
    theme: Theme,
) {
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

pub(super) fn security_resource_text(resource: &SecurityResource) -> String {
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
        SecurityResource::Other { kind, id } if kind == "external-service" => {
            format!("external service {id}")
        }
        SecurityResource::Other { kind, id } => format!("{kind}:{id}"),
    }
}

pub(super) fn authority_warning(prepared: &PreparedToolCall) -> Option<String> {
    let permissions = prepared.required_permissions();
    let delegation = smith_tools::display::project_delegation_approval_display(prepared);
    let mut capabilities = Vec::new();
    if permissions.contains(&Permission::ProcessSpawn) {
        capabilities.push("process execution");
    }
    if permissions.contains(&Permission::FsDelete) {
        capabilities.push("file deletion");
    }
    if permissions.contains(&Permission::ExternalRead) {
        capabilities.push("external service read");
    }
    if permissions.contains(&Permission::ExternalWrite) {
        capabilities.push("possible external service mutation");
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
    if let Some(display) = &delegation
        && permissions.contains(&Permission::other("agent.delegate"))
    {
        capabilities.push(display.warning);
    }
    if permissions.iter().any(|permission| {
        matches!(permission, Permission::Other(name)
            if delegation.is_none() || name.as_ref() != "agent.delegate")
    }) {
        capabilities.push("host-defined authority");
    }
    (!capabilities.is_empty()).then(|| format!("authority warning: {}", capabilities.join(", ")))
}

pub(super) fn deadline_text(deadline: Deadline) -> String {
    let Some(expires) = deadline.instant() else {
        return "no deadline".to_owned();
    };
    let millis = expires.as_millis();
    let absolute = crate::time_display::local_timestamp(millis);
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

/// A modal's content, grouped by what it is willing to lose first.
///
/// A modal is capped at 60% of the terminal's height (`DESIGN.md` §2), so on a
/// short terminal something has to go. The order is not cosmetic: an approval
/// whose keys or whose subject scrolled out of view cannot be answered, and
/// there is no default action to fall back on.
pub(super) struct ModalContent {
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
pub(super) fn fit_rows(
    lines: Vec<Line<'static>>,
    width: usize,
    budget: usize,
) -> (Vec<Line<'static>>, usize) {
    let mut used = 0;
    let mut kept = Vec::new();
    let mut dropped = 0;
    for line in lines {
        let rows = wrapped_rows(std::slice::from_ref(&line), width);
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
pub(super) fn wrapped_rows(lines: &[Line<'static>], width: usize) -> usize {
    rendered_rows(lines, u16::try_from(width).unwrap_or(u16::MAX))
}

pub(super) fn draw_palette(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    selected: usize,
    error: Option<&str>,
    theme: Theme,
) {
    let matches = commands::matches_with(app.composer.text(), &app.command_catalog);
    let mut lines = Vec::new();
    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no matching commands",
            theme.style(Tone::Warning),
        )));
    } else {
        let error_rows = usize::from(error.is_some());
        let selected = selected.min(matches.len().saturating_sub(1));
        let detail = palette_detail(matches[selected]);
        let detail_rows = usize::from(!detail.is_empty());
        let capacity = usize::from(area.height).saturating_sub(error_rows + detail_rows);
        let visible = capacity.min(MAX_VISIBLE_PALETTE_ROWS).min(matches.len());
        let start = selected
            .saturating_sub(visible / 2)
            .min(matches.len().saturating_sub(visible));
        let name_width = matches
            .iter()
            .skip(start)
            .take(visible)
            .map(|command| command.name().width() + 1)
            .max()
            .unwrap_or(0);
        for (index, command) in matches.into_iter().enumerate().skip(start).take(visible) {
            lines.push(list_row(
                &format!("/{}", command.name()),
                &match command {
                    commands::MenuRow::File(entry)
                        if entry.state != smith_client::file_commands::CommandState::Runnable =>
                    {
                        format!(
                            "{} · {}",
                            command.description(),
                            entry.state.reason(command.name())
                        )
                    }
                    _ => command.description().to_owned(),
                },
                command.layer_label().unwrap_or_default(),
                index == selected,
                name_width,
                area.width,
                if index == selected {
                    Tone::Accent
                } else {
                    Tone::Default
                },
                theme,
            ));
            if index == selected && !detail.is_empty() {
                lines.push(detail_line(&detail, name_width, area.width, theme));
            }
        }
    }
    if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            format!("  {} {error}", glyph::ERROR),
            theme.style(Tone::Danger),
        )));
    }
    // Deliberately do not wrap palette rows. The command label and selection
    // marker stay on the left, while long hints/descriptions yield to the
    // terminal edge instead of pushing the error or selected row away.
    frame.render_widget(Paragraph::new(lines), area);
}

fn palette_detail(command: commands::MenuRow<'_>) -> String {
    let hint = command.argument_hint().unwrap_or_default();
    match command {
        commands::MenuRow::File(entry)
            if entry.state != smith_client::file_commands::CommandState::Runnable =>
        {
            let reason = entry.state.reason(command.name());
            if hint.is_empty() {
                reason
            } else {
                format!("{hint} · {reason}")
            }
        }
        _ => hint.to_owned(),
    }
}

pub(super) fn desired_palette_rows(app: &App, selected: usize, error: Option<&str>) -> u16 {
    let matches = commands::matches_with(app.composer.text(), &app.command_catalog);
    let detail = matches
        .get(selected.min(matches.len().saturating_sub(1)))
        .is_some_and(|command| !palette_detail(*command).is_empty());
    let rows = matches.len().clamp(1, MAX_VISIBLE_PALETTE_ROWS)
        + usize::from(detail)
        + usize::from(error.is_some());
    u16::try_from(rows).unwrap_or(u16::MAX)
}

pub(super) fn draw_history_search(
    frame: &mut Frame<'_>,
    area: Rect,
    query: &crate::line_input::LineInput,
    matched: Option<&str>,
    theme: Theme,
) {
    let query_empty = query.is_empty();
    let (visible, cursor) = query.viewport(usize::from(area.width).saturating_sub(18));
    let query = if query_empty { "type query" } else { &visible };
    let result = matched.map_or_else(
        || {
            if query_empty {
                "  history is unchanged until a query matches".to_owned()
            } else {
                "  no matching history".to_owned()
            }
        },
        |entry| format!("› {}", entry.replace('\n', " ↵ ")),
    );
    let lines = vec![
        Line::from(vec![
            Span::styled("  reverse search  ", theme.style(Tone::Heading)),
            Span::styled(query.to_owned(), theme.style(Tone::Accent)),
        ]),
        Line::from(Span::styled(
            result,
            theme.style(if matched.is_some() {
                Tone::Default
            } else {
                Tone::Dim
            }),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), area);
    if area.width > 18 && area.height > 0 {
        frame.set_cursor_position((area.x + 18 + cursor as u16, area.y));
    }
}

struct ConfirmLayout {
    area: Rect,
    warning: Vec<Line<'static>>,
    body: Vec<Line<'static>>,
    foot: Vec<Line<'static>>,
    body_height: u16,
    scroll_limit: usize,
}

fn confirm_layout(area: Rect, dialog: &ConfirmDialog, theme: Theme) -> ConfirmLayout {
    let width = modal_width(area);
    let inner = width.saturating_sub(2 + super::approval::HORIZONTAL_PADDING * 2);
    let warning = dialog
        .warning
        .as_ref()
        .map_or_else(Vec::new, |(text, tone)| {
            super::wrap::wrap_lines(
                &[Line::from(Span::styled(text.clone(), theme.style(*tone)))],
                inner,
            )
        });
    let body = dialog
        .body
        .iter()
        .cloned()
        .map(Line::from)
        .collect::<Vec<_>>();
    let body = super::wrap::wrap_lines(&body, inner);
    let controls = Line::from(vec![
        Span::styled("y", theme.style(dialog.accept_tone)),
        Span::styled(
            format!(" {}   ", dialog.accept_label),
            theme.style(Tone::Dim),
        ),
        Span::styled(dialog.cancel_key, theme.style(Tone::Success)),
        Span::styled(format!(" {}", dialog.cancel_label), theme.style(Tone::Dim)),
    ]);
    let mut foot = super::wrap::wrap_lines(&[controls], inner);
    let wanted = warning.len() + body.len() + foot.len() + usize::from(!body.is_empty()) + 2;
    // Like approvals, the height ceiling yields enough room for the fixed
    // warning, decision keys, and one body row on short terminals.
    let minimum = warning.len() + foot.len() + usize::from(!body.is_empty()) + 2;
    let mut height = wanted
        .min(usize::from(modal_max_height(area)).max(minimum))
        .min(usize::from(area.height));
    let mut room = height.saturating_sub(2 + warning.len() + foot.len());
    // The separator belongs to the fixed footer, never the scrollable body.
    // On short terminals it yields to the last visible body row.
    let mut separator = usize::from(!body.is_empty() && room > 1);
    room = room.saturating_sub(separator);
    if body.len() > room {
        height = height.max(minimum + 1).min(usize::from(area.height));
        room = height.saturating_sub(3 + warning.len() + foot.len());
        separator = usize::from(!body.is_empty() && room > 1);
        room = room.saturating_sub(separator);
        let limit = body.len().saturating_sub(room);
        foot.push(Line::from(Span::styled(
            format!("↑↓ review · {}/{}", dialog.scroll.min(limit) + 1, limit + 1),
            theme.style(Tone::Dim),
        )));
    }
    if separator > 0 {
        foot.insert(0, Line::default());
    }
    let scroll_limit = body.len().saturating_sub(room);
    let height = u16::try_from(height).unwrap_or(u16::MAX);
    ConfirmLayout {
        area: Rect::new(
            area.x + area.width.saturating_sub(width) / 2,
            area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        ),
        warning,
        body,
        foot,
        body_height: u16::try_from(room).unwrap_or(u16::MAX),
        scroll_limit,
    }
}

pub(super) fn confirm_scroll_limit(area: Rect, dialog: &ConfirmDialog, theme: Theme) -> usize {
    confirm_layout(area, dialog, theme).scroll_limit
}

pub(super) fn draw_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &ConfirmDialog,
    theme: Theme,
) {
    let ConfirmLayout {
        area,
        warning,
        body,
        foot,
        body_height,
        scroll_limit,
    } = confirm_layout(area, dialog, theme);
    let block = WidgetBlock::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(super::approval::HORIZONTAL_PADDING))
        .border_style(theme.style(dialog.tone))
        .title(Span::styled(
            format!(" {} ", dialog.title),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let warning_height = u16::try_from(warning.len())
        .unwrap_or(u16::MAX)
        .min(inner.height);
    frame.render_widget(
        Paragraph::new(warning),
        Rect::new(inner.x, inner.y, inner.width, warning_height),
    );
    let body_y = inner.y + warning_height;
    let visible = body
        .into_iter()
        .skip(dialog.scroll.min(scroll_limit))
        .take(usize::from(body_height))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(visible),
        Rect::new(inner.x, body_y, inner.width, body_height),
    );
    frame.render_widget(
        Paragraph::new(foot),
        Rect::new(
            inner.x,
            body_y + body_height,
            inner.width,
            inner.height.saturating_sub(warning_height + body_height),
        ),
    );
}

/// A modal's width: centered, at most 72 columns (`DESIGN.md` §2).
pub(super) fn modal_width(area: Rect) -> u16 {
    area.width
        .saturating_sub(4)
        .min(72)
        .max(MIN_WIDTH.min(area.width))
}

/// The tallest a modal may be: 60% of the height (`DESIGN.md` §2), and never
/// more than the viewport, so an overlay cannot spill off screen.
pub(super) fn modal_max_height(area: Rect) -> u16 {
    (area.height.saturating_mul(3) / 5).max(3).min(area.height)
}

pub(super) fn draw_modal(
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
        Paragraph::new(super::wrap::wrap_lines(
            &lines,
            modal.width.saturating_sub(2),
        ))
        .block(
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
