//! Action-first approvals, drawn inline above the composer, with scrollable
//! prepared-action detail.

use agent_runtime_core::security::SecurityResource;
use agent_runtime_core::tool::PreparedToolCall;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::{App, Overlay};
use crate::diff::{Change, EditReview};
use crate::theme::{Theme, Tone, glyph};

use super::modal::{authority_warning, deadline_text, security_resource_text};
use super::questionnaire::choice_row;
use super::wrap::wrap_lines;

const DIFF_PREVIEW_LINES: usize = 4;
/// Indent shared with the compact pickers and the questionnaire.
pub(super) const INDENT: &str = "  ";
pub(super) const INDENT_WIDTH: u16 = 2;

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
        } else if prepared.tool() == "edit" {
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
    let session = session_choice(prepared, delegation.is_some());
    let mut foot = vec![line(
        format!("{INDENT}Do you want to proceed?"),
        theme,
        Tone::Heading,
    )];
    foot.extend(choice_rows(
        &["Yes", &session, "No (esc)"],
        app.approval_choice,
        theme,
    ));
    foot.push(line(format!("{INDENT}{hint}"), theme, Tone::Dim));
    (title, body, foot)
}

/// The first row of every inline prompt, matching the picker and
/// questionnaire headings: indented, in the heading tone. Severity belongs to
/// the warning rows, not the title.
pub(super) fn pane_title(text: &str, theme: Theme) -> Line<'static> {
    line(format!("{INDENT}{text}"), theme, Tone::Heading)
}

/// A numbered choice list shared by every inline prompt, drawn exactly like
/// the questionnaire's rows: `❯ 1. Yes`.
pub(super) fn choice_rows(labels: &[&str], selected: usize, theme: Theme) -> Vec<Line<'static>> {
    labels
        .iter()
        .enumerate()
        .map(|(index, label)| choice_row(index, label, index == selected, false, theme))
        .collect()
}

/// Wraps choice rows so a long label hangs under its first word, not under
/// the `❯ 1. ` marker. Other lines wrap normally.
pub(super) fn wrap_choices(lines: &[Line<'static>], width: u16) -> Vec<Line<'static>> {
    let mut wrapped = Vec::new();
    for line in lines {
        let marker = line.spans.first().map(|span| span.content.as_ref());
        let is_choice = matches!(marker, Some("❯ " | "  "))
            && line
                .spans
                .get(1)
                .is_some_and(|span| span.content.ends_with(". "));
        if !is_choice {
            wrapped.extend(wrap_lines(std::slice::from_ref(line), width));
            continue;
        }
        let hang = line.spans[..2]
            .iter()
            .map(|span| span.content.chars().count())
            .sum::<usize>();
        let label = Line::from(line.spans[2..].to_vec());
        let rows = wrap_lines(&[label], width.saturating_sub(hang as u16));
        for (index, mut row) in rows.into_iter().enumerate() {
            if index == 0 {
                let mut spans = line.spans[..2].to_vec();
                spans.extend(row.spans);
                row.spans = spans;
            } else {
                row.spans.insert(0, Span::raw(" ".repeat(hang)));
            }
            wrapped.push(row);
        }
    }
    wrapped
}

/// The same choices on one row, for a pane too short for the full list.
fn compact_choices(labels: &[&str], selected: usize, theme: Theme) -> Line<'static> {
    let mut spans = vec![Span::styled("❯ ", theme.style(Tone::Accent))];
    for (index, label) in labels.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", theme.style(Tone::Dim)));
        }
        spans.push(Span::styled(
            format!("{}. {label}", index + 1),
            theme.style(if index == selected {
                Tone::Accent
            } else {
                Tone::Default
            }),
        ));
    }
    Line::from(spans)
}

pub(super) fn indented(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|mut line| {
            line.spans.insert(0, Span::raw(INDENT));
            line
        })
        .collect()
}

struct ApprovalLayout {
    title: Line<'static>,
    body: Vec<Line<'static>>,
    foot: Vec<Line<'static>>,
    scroll_limit: u16,
}

/// Rows the inline approval wants at `width`; the layout may grant fewer.
pub(super) fn approval_rows(app: &App, width: u16) -> u16 {
    let inner = width.saturating_sub(INDENT_WIDTH * 3);
    let (_, body, foot) = compose(app, Theme::new(), DIFF_PREVIEW_LINES);
    let rows = 1 + wrap_lines(&body, inner).len() + wrap_choices(&foot, width).len();
    u16::try_from(rows).unwrap_or(u16::MAX)
}

fn approval_layout(area: Rect, app: &App, theme: Theme) -> ApprovalLayout {
    let inner = area.width.saturating_sub(INDENT_WIDTH * 3);
    let height = usize::from(area.height);
    let (title, body, mut controls) = compose(app, theme, DIFF_PREVIEW_LINES);
    let title = pane_title(&title, theme);
    let mut body = wrap_lines(&body, inner);
    // A pane too short for the title, one evidence row, and the full choice
    // list keeps the same keys on one line.
    let compact = 2 + wrap_choices(&controls, area.width).len() > height;
    if compact {
        controls = vec![compact_choices(
            &["Yes", "Session", "No (esc)"],
            app.approval_choice,
            theme,
        )];
    }
    let mut foot = wrap_choices(&controls, area.width);
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
            if 1 + body.len() + foot.len() <= height {
                break;
            }
            body = wrap_lines(&compose(app, theme, preview).1, inner);
        }
    }
    let room = height.saturating_sub(1 + foot.len());
    // The one-row choice list has no room for a review hint; paging still works.
    if body.len() > room
        && !compact
        && let Some(hint) = controls.last_mut()
    {
        // Extra detail scrolls; controls never scroll away with the JSON or diff.
        hint.spans
            .push(Span::styled(" · shift+↑↓ review", theme.style(Tone::Dim)));
        foot = wrap_choices(&controls, area.width);
    }
    let room = height.saturating_sub(1 + foot.len());
    let scroll_limit = u16::try_from(body.len().saturating_sub(room)).unwrap_or(u16::MAX);
    ApprovalLayout {
        title,
        body: indented(indented(body)),
        foot,
        scroll_limit,
    }
}

pub(super) fn approval_scroll_limit(area: Rect, app: &App, theme: Theme) -> u16 {
    approval_layout(area, app, theme).scroll_limit
}

/// Draws the approval inline above the composer, like the questionnaire.
pub(super) fn draw_approval(frame: &mut Frame<'_>, area: Rect, app: &App, theme: Theme) {
    if area.is_empty() {
        return;
    }
    let ApprovalLayout {
        title,
        body,
        foot,
        scroll_limit,
    } = approval_layout(area, app, theme);
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(title), Rect { height: 1, ..area });
    let foot_height = u16::try_from(foot.len())
        .unwrap_or(u16::MAX)
        .min(area.height.saturating_sub(1));
    let body_height = area.height.saturating_sub(1 + foot_height);
    frame.render_widget(
        Paragraph::new(body).scroll((app.approval_scroll.min(scroll_limit), 0)),
        Rect::new(area.x, area.y + 1, area.width, body_height),
    );
    frame.render_widget(
        Paragraph::new(foot),
        Rect::new(area.x, area.y + 1 + body_height, area.width, foot_height),
    );
}
