//! Palette, search, and confirmation panes, plus shared approval wording.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::{App, ConfirmDialog};
use crate::commands;
use crate::theme::{Theme, Tone, glyph};
use agent_runtime_core::clock::Deadline;
use agent_runtime_core::security::SecurityResource;
use agent_runtime_core::tool::PreparedToolCall;
use agent_runtime_registry::Permission;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::approval::{INDENT, INDENT_WIDTH, choice_rows, indented, pane_title, wrap_choices};
use super::lists::{detail_line, list_row};

/// Command completion keeps the transcript visible by reserving at most five
/// choices for the selected window, plus its optional argument detail line.
const MAX_VISIBLE_PALETTE_ROWS: usize = 5;

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
                    commands::MenuRow::Skill(skill) if skill.disabled_reason.is_some() => {
                        format!(
                            "{} · {}",
                            skill.description,
                            skill.disabled_reason.as_deref().unwrap_or_default()
                        )
                    }
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
        commands::MenuRow::Skill(skill) => match &skill.disabled_reason {
            Some(reason) => format!("{hint} · {reason}"),
            None => format!("{hint} · activate skill"),
        },
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
    title: Line<'static>,
    warning: Vec<Line<'static>>,
    body: Vec<Line<'static>>,
    foot: Vec<Line<'static>>,
    body_height: u16,
    scroll_limit: usize,
}

/// Rows the inline confirmation wants at `width`; the layout may grant fewer.
pub(super) fn confirm_rows(dialog: &ConfirmDialog, width: u16) -> u16 {
    let layout = confirm_layout(Rect::new(0, 0, width, u16::MAX), dialog, Theme::new());
    let rows = 1 + layout.warning.len() + layout.body.len() + layout.foot.len();
    u16::try_from(rows).unwrap_or(u16::MAX)
}

fn confirm_layout(area: Rect, dialog: &ConfirmDialog, theme: Theme) -> ConfirmLayout {
    let inner = area.width.saturating_sub(INDENT_WIDTH * 2);
    let title = pane_title(&dialog.title, theme);
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
    let cancel = format!("{} ({})", dialog.cancel_label, dialog.cancel_key);
    let controls = choice_rows(&[&dialog.accept_label, &cancel], dialog.choice, theme);
    let mut foot = wrap_choices(&controls, area.width);
    let height = usize::from(area.height);
    let mut room = height.saturating_sub(1 + warning.len() + foot.len());
    // The separator belongs to the fixed footer, never the scrollable body.
    // On short panes it yields to the last visible body row.
    let mut separator = usize::from(!body.is_empty() && room > 1);
    room = room.saturating_sub(separator);
    if body.len() > room {
        room = room.saturating_sub(1);
        separator = usize::from(!body.is_empty() && room > 1);
        room = room.saturating_sub(separator);
        let limit = body.len().saturating_sub(room);
        foot.push(Line::from(Span::styled(
            format!(
                "{INDENT}shift+↑↓ review · {}/{}",
                dialog.scroll.min(limit) + 1,
                limit + 1
            ),
            theme.style(Tone::Dim),
        )));
    }
    if separator > 0 {
        foot.insert(0, Line::default());
    }
    let scroll_limit = body.len().saturating_sub(room);
    ConfirmLayout {
        title,
        warning: indented(warning),
        body: indented(body),
        foot,
        body_height: u16::try_from(room).unwrap_or(u16::MAX),
        scroll_limit,
    }
}

pub(super) fn confirm_scroll_limit(area: Rect, dialog: &ConfirmDialog, theme: Theme) -> usize {
    confirm_layout(area, dialog, theme).scroll_limit
}

/// Draws a confirmation inline above the composer, like the questionnaire.
pub(super) fn draw_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &ConfirmDialog,
    theme: Theme,
) {
    if area.is_empty() {
        return;
    }
    let ConfirmLayout {
        title,
        warning,
        body,
        foot,
        body_height,
        scroll_limit,
    } = confirm_layout(area, dialog, theme);
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(title), Rect { height: 1, ..area });
    let inner = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(1),
    );
    let warning_height = u16::try_from(warning.len())
        .unwrap_or(u16::MAX)
        .min(inner.height);
    frame.render_widget(
        Paragraph::new(warning),
        Rect::new(inner.x, inner.y, inner.width, warning_height),
    );
    let body_y = inner.y + warning_height;
    let body_height = body_height.min(inner.height.saturating_sub(warning_height));
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
