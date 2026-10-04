//! Action-first approvals, with scrollable prepared-action detail.

use agent_runtime_core::security::SecurityResource;
use agent_runtime_core::tool::PreparedToolCall;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph};

use crate::app::{App, Overlay};
use crate::diff::{Change, EditReview};
use crate::theme::{Theme, Tone, glyph};

use super::modal::{
    authority_warning, deadline_text, modal_max_height, modal_width, security_resource_text,
};
use super::wrap::wrap_lines;

const DIFF_PREVIEW_LINES: usize = 4;
pub(super) const HORIZONTAL_PADDING: u16 = 2;

fn line(text: impl Into<String>, theme: Theme, tone: Tone) -> Line<'static> {
    Line::from(Span::styled(text.into(), theme.style(tone)))
}

fn diff_lines(
    review: &EditReview,
    expanded: bool,
    preview: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    let count = if expanded {
        review.changes.len()
    } else {
        preview
    };
    let mut lines: Vec<_> = review
        .changes
        .iter()
        .take(count)
        .map(|change| match change {
            Change::Context(text) => line(format!("  {text}"), theme, Tone::Dim),
            Change::Removed(text) => {
                line(format!("{} {text}", glyph::REMOVED), theme, Tone::Danger)
            }
            Change::Added(text) => line(format!("{} {text}", glyph::ADDED), theme, Tone::Success),
            Change::Skipped(count) => line(
                format!(
                    "… {count} unchanged line{}",
                    if *count == 1 { "" } else { "s" }
                ),
                theme,
                Tone::Dim,
            ),
        })
        .collect();
    let hidden = review.changes.len().saturating_sub(count);
    if hidden > 0 {
        lines.push(line(
            format!("… +{hidden} lines (ctrl+o to expand)"),
            theme,
            Tone::Dim,
        ));
    }
    lines
}

// Unknown tools still need their material arguments on the default view. Plain
// fields keep that evidence readable without making raw JSON the action.
fn material_lines(
    name: &str,
    value: &serde_json::Value,
    theme: Theme,
    lines: &mut Vec<Line<'static>>,
) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                let name = if name.is_empty() {
                    key.clone()
                } else {
                    format!("{name}.{key}")
                };
                material_lines(&name, value, theme, lines);
            }
        }
        serde_json::Value::Array(values) if !values.is_empty() => {
            for (index, value) in values.iter().enumerate() {
                material_lines(&format!("{name}[{index}]"), value, theme, lines);
            }
        }
        value => {
            let text = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned);
            for (index, text) in text.split('\n').enumerate() {
                let prefix = if index == 0 {
                    format!("{name}: ")
                } else {
                    "  ".to_owned()
                };
                lines.push(line(format!("{prefix}{text}"), theme, Tone::Default));
            }
        }
    }
}

fn host_shell(prepared: &PreparedToolCall) -> bool {
    prepared.tool() == "shell"
        && matches!(prepared.resource(), SecurityResource::Other { kind, .. } if kind == "host-shell")
}

fn session_choice(prepared: &PreparedToolCall, child_operation: bool) -> String {
    if child_operation {
        "Yes, and don't ask again for child agents this session".to_owned()
    } else if host_shell(prepared) {
        // The shell resource binds command, cwd, mode, and timeout. It is not
        // a command-prefix allowance, even when two commands share a verb.
        "Yes, don't ask again for this exact shell action this session".to_owned()
    } else {
        format!(
            "Yes, and don't ask again for `{}` in this target this session",
            prepared.tool(),
        )
    }
}

fn compose(
    app: &App,
    theme: Theme,
    preview: usize,
) -> (String, Vec<Line<'static>>, Vec<Line<'static>>) {
    let Some(Overlay::Approval { prompt, review }) = &app.overlay else {
        return (String::new(), Vec::new(), Vec::new());
    };
    let prepared = prompt.prepared();
    let arguments = prepared.arguments();
    let delegation = smith_tools::display::project_delegation_approval_display(prepared);
    let path = smith_tools::display::approval_path_display(prepared);
    let resource = path
        .clone()
        .unwrap_or_else(|| security_resource_text(prepared.resource()));
    let title = if let Some(display) = &delegation {
        display.title.to_owned()
    } else {
        match prepared.tool() {
            "shell" => "Bash command".to_owned(),
            "edit" => "Edit file".to_owned(),
            _ => prepared.display().title.clone(),
        }
    };
    let mut body = Vec::new();
    if let Some(display) = &delegation {
        body.extend(
            display
                .lines
                .iter()
                .map(|text| line(text, theme, Tone::Default)),
        );
        if let Some(turns) = display.turn_limit {
            body.push(line(format!("turn limit {turns}"), theme, Tone::Dim));
        }
        if let Some(tokens) = display.token_limit {
            body.push(line(
                format!("token limit {}", smith_client::compact_tokens(tokens)),
                theme,
                Tone::Dim,
            ));
        }
        if let Some(millis) = display.time_limit_ms {
            let duration = if millis.is_multiple_of(60_000) {
                format!("{} min", millis / 60_000)
            } else if millis.is_multiple_of(1_000) {
                format!("{} s", millis / 1_000)
            } else {
                format!("{millis} ms")
            };
            body.push(line(format!("time limit {duration}"), theme, Tone::Dim));
        }
    } else if prepared.tool() == "shell" {
        if let Some(command) = arguments.get("command").and_then(serde_json::Value::as_str) {
            body.extend(
                command
                    .split('\n')
                    .map(|text| line(text, theme, Tone::Default)),
            );
        } else {
            body.push(line(prepared.display().title.clone(), theme, Tone::Heading));
            material_lines("", arguments, theme, &mut body);
        }
    } else {
        if path.is_some() {
            body.push(line(resource.clone(), theme, Tone::Danger));
        }
        if let Some(review) = review {
            body.push(line(review.summary(), theme, Tone::Dim));
            body.extend(diff_lines(review, app.work_details, preview, theme));
        } else {
            if prepared.tool() == "edit" {
                // The fixed title names the action; the first body line names
                // the target. Keep material edit arguments without repeating it.
                let mut material = arguments.clone();
                if let Some(fields) = material.as_object_mut() {
                    fields.remove("path");
                }
                material_lines("", &material, theme, &mut body);
            } else {
                body.push(line(prepared.display().title.clone(), theme, Tone::Heading));
                if let Some(detail) = &prepared.display().detail {
                    body.extend(
                        detail
                            .split('\n')
                            .map(|text| line(text, theme, Tone::Default)),
                    );
                }
                material_lines("", arguments, theme, &mut body);
            }
        }
    }

    let mut place = if prepared.tool() == "shell" {
        // Prepared shell cwd is canonicalized by the tool. The resource is
        // opaque for host shells; its digest belongs with the identity detail.
        let cwd = arguments.get("cwd").and_then(serde_json::Value::as_str);
        let shown = prepared
            .display()
            .title
            .strip_prefix("Run unsandboxed host shell in ");
        format!("in {}", cwd.or(shown).unwrap_or(&resource))
    } else if delegation.is_some() || path.is_some() {
        String::new()
    } else {
        format!("at {resource}")
    };
    if prepared.tool() == "shell" {
        if let Some(timeout) = arguments
            .get("timeout_ms")
            .and_then(serde_json::Value::as_u64)
        {
            let duration = if timeout.is_multiple_of(60_000) {
                format!("{} min", timeout / 60_000)
            } else if timeout.is_multiple_of(1_000) {
                format!("{} s", timeout / 1_000)
            } else {
                format!("{timeout} ms")
            };
            place.push_str(&format!(" · up to {duration}"));
        } else {
            place.push_str(" · no execution timeout supplied");
        }
        if arguments
            .get("run_in_background")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            place.push_str(" · background");
        }
    }
    if prompt.deadline().instant().is_some() {
        if !place.is_empty() {
            place.push_str(" · ");
        }
        place.push_str(&format!("deadline {}", deadline_text(prompt.deadline())));
    }
    if !place.is_empty() {
        body.push(line(place, theme, Tone::Dim));
    }
    if host_shell(prepared) {
        body.push(line(
            "Warning: Runs outside the sandbox with your files, environment and credentials, child processes, network, and data egress.",
            theme,
            Tone::Warning,
        ));
    } else if let Some(warning) = authority_warning(prepared) {
        body.push(line(
            format!(
                "Warning: {}",
                warning.trim_start_matches("authority warning: ")
            ),
            theme,
            Tone::Warning,
        ));
    }

    if app.work_details {
        body.push(line(
            format!("identity: {}", prepared.fingerprint().as_str()),
            theme,
            Tone::Dim,
        ));
        let permissions = prepared
            .required_permissions()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        body.push(line(
            format!("permissions: {}", permissions.join(", ")),
            theme,
            Tone::Dim,
        ));
        body.push(line("raw arguments:", theme, Tone::Dim));
        let raw = serde_json::to_string_pretty(arguments).unwrap_or_else(|_| arguments.to_string());
        body.extend(raw.lines().map(|text| line(text, theme, Tone::Dim)));
    }

    let waiting = app.queued_prompt_count();
    let mut hint = if app.work_details {
        "ctrl+o fold"
    } else {
        "ctrl+o details"
    }
    .to_owned();
    if waiting > 0 {
        hint.push_str(&format!(" · {waiting} more waiting"));
    }
    let foot = vec![
        line("Do you want to proceed?", theme, Tone::Heading),
        line("  y  Yes", theme, Tone::Success),
        line(
            format!("  a  {}", session_choice(prepared, delegation.is_some())),
            theme,
            Tone::Warning,
        ),
        line("  n  No (esc)", theme, Tone::Default),
        line(hint, theme, Tone::Dim),
    ];
    (title, body, foot)
}

struct ApprovalLayout {
    area: Rect,
    title: String,
    body: Vec<Line<'static>>,
    foot: Vec<Line<'static>>,
    scroll_limit: u16,
}

fn approval_layout(area: Rect, app: &App, theme: Theme) -> ApprovalLayout {
    let width = if area.width < 60 {
        area.width
    } else {
        modal_width(area)
    };
    let inner = width.saturating_sub(2 + HORIZONTAL_PADDING * 2);
    let (title, body, mut controls) = compose(app, theme, DIFF_PREVIEW_LINES);
    let mut body = wrap_lines(&body, inner);
    let mut foot = wrap_lines(&controls, inner);
    if !app.work_details
        && matches!(
            app.overlay,
            Some(Overlay::Approval {
                review: Some(_),
                ..
            })
        )
    {
        // Fold the diff further before letting it push the place, warning, or
        // deadline out of the default view on a short terminal.
        for preview in (1..DIFF_PREVIEW_LINES).rev() {
            if body.len() + foot.len() + 2 <= usize::from(area.height) {
                break;
            }
            body = wrap_lines(&compose(app, theme, preview).1, inner);
        }
    }
    let wanted = u16::try_from(body.len() + foot.len() + 2).unwrap_or(u16::MAX);
    // The normal modal ceiling yields to decision evidence on short screens.
    // Extra detail scrolls; controls never scroll away with the JSON or diff.
    let minimum = if app.work_details {
        u16::try_from(foot.len() + 5).unwrap_or(u16::MAX)
    } else {
        wanted
    };
    let height = wanted
        .min(modal_max_height(area).max(minimum))
        .min(area.height);
    let modal = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    let mut room = usize::from(height.saturating_sub(2)).saturating_sub(foot.len());
    if body.len() > room {
        // Replace the detail hint rather than spending a scarce row that
        // would otherwise hold the exact target on a ten-row screen.
        if let Some(hint) = controls.last_mut() {
            hint.spans
                .push(Span::styled(" · ↑↓ review", theme.style(Tone::Dim)));
        }
        foot = wrap_lines(&controls, inner);
        if foot.len() + 1 > usize::from(height.saturating_sub(2)) {
            if let Some(hint) = controls.last_mut() {
                hint.spans.pop();
                hint.spans
                    .push(Span::styled(" · ↑↓", theme.style(Tone::Dim)));
            }
            foot = wrap_lines(&controls, inner);
        }
        room = usize::from(height.saturating_sub(2)).saturating_sub(foot.len());
    }
    let scroll_limit = u16::try_from(body.len().saturating_sub(room)).unwrap_or(u16::MAX);
    ApprovalLayout {
        area: modal,
        title,
        body,
        foot,
        scroll_limit,
    }
}

pub(super) fn approval_scroll_limit(area: Rect, app: &App, theme: Theme) -> u16 {
    approval_layout(area, app, theme).scroll_limit
}

pub(super) fn draw_approval(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    let ApprovalLayout {
        area,
        title,
        body,
        foot,
        scroll_limit,
    } = approval_layout(area, app, theme);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .padding(Padding::horizontal(HORIZONTAL_PADDING))
        .border_style(theme.style(Tone::Warning))
        .title(Span::styled(
            format!(" {title} "),
            theme.style(Tone::Heading),
        ));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let foot_height = u16::try_from(foot.len())
        .unwrap_or(u16::MAX)
        .min(inner.height);
    let body_height = inner.height.saturating_sub(foot_height);
    frame.render_widget(
        Paragraph::new(body).scroll((app.approval_scroll.min(scroll_limit), 0)),
        Rect::new(inner.x, inner.y, inner.width, body_height),
    );
    frame.render_widget(
        Paragraph::new(foot),
        Rect::new(inner.x, inner.y + body_height, inner.width, foot_height),
    );
}
