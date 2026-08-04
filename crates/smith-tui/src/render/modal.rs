//! Approval, questionnaire, palette, search, and confirmation overlays.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::App;
use crate::commands;
use crate::diff::{Change, EditReview};
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
use ratatui::widgets::{Block as WidgetBlock, Borders, Clear, Paragraph, Wrap};

use super::helpers::*;
use super::layout::*;
use super::transcript::*;

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
        SecurityResource::Other { kind, id } => format!("{kind}:{id}"),
    }
}

pub(super) fn authority_warning(prepared: &PreparedToolCall) -> Option<String> {
    let permissions = prepared.required_permissions();
    let mut capabilities = Vec::new();
    if permissions.contains(&Permission::ProcessSpawn) {
        capabilities.push("process execution");
    }
    if permissions.contains(&Permission::FsDelete) {
        capabilities.push("file deletion");
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
    if permissions
        .iter()
        .any(|permission| matches!(permission, Permission::Other(_)))
    {
        capabilities.push("host-defined authority");
    }
    (!capabilities.is_empty()).then(|| format!("authority warning: {}", capabilities.join(", ")))
}

pub(super) fn deadline_text(deadline: Deadline) -> String {
    let Some(expires) = deadline.instant() else {
        return "no deadline".to_owned();
    };
    let millis = expires.as_millis();
    let local = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(millis) * 1_000_000)
        .map(|instant| {
            instant
                .to_offset(time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC))
        })
        .ok();
    let absolute = local.map_or_else(
        || format!("{millis}ms since epoch"),
        |instant| {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {}",
                instant.year(),
                u8::from(instant.month()),
                instant.day(),
                instant.hour(),
                instant.minute(),
                instant.second(),
                instant.offset()
            )
        },
    );
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

/// The diff body: one row per change, each marked with a sign as well as a
/// color, because color is a second channel and never the only one
/// (`DESIGN.md` §4).
pub(super) fn review_lines(review: &EditReview, theme: Theme) -> (Vec<Line<'static>>, usize) {
    let lines = review
        .changes
        .iter()
        .take(MAX_BODY_LINES)
        .map(|change| match change {
            Change::Context(text) => {
                Line::from(Span::styled(format!("  {text}"), theme.style(Tone::Dim)))
            }
            Change::Removed(text) => Line::from(Span::styled(
                format!("{} {text}", glyph::REMOVED),
                theme.style(Tone::Danger),
            )),
            Change::Added(text) => Line::from(Span::styled(
                format!("{} {text}", glyph::ADDED),
                theme.style(Tone::Success),
            )),
            Change::Skipped(count) => Line::from(Span::styled(
                format!("{} {count} unchanged lines", glyph::ELIDED),
                theme.style(Tone::Dim),
            )),
        })
        .collect();
    (lines, review.changes.len().saturating_sub(MAX_BODY_LINES))
}

/// The fallback body: the raw arguments, exactly as before this modal learned
/// to read edits. Anything that is not a reviewable edit lands here, because a
/// diff that lies about what will change is worse than no diff.
pub(super) fn argument_lines(
    arguments: &serde_json::Value,
    theme: Theme,
) -> (Vec<Line<'static>>, usize) {
    let arguments =
        serde_json::to_string_pretty(arguments).unwrap_or_else(|_| arguments.to_string());
    let lines: Vec<Line<'static>> = arguments
        .lines()
        .take(MAX_BODY_LINES)
        .map(|raw| Line::from(Span::styled(raw.to_owned(), theme.style(Tone::Dim))))
        .collect();
    (
        lines,
        arguments.lines().count().saturating_sub(MAX_BODY_LINES),
    )
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
    let matches = commands::matches(app.composer.text());
    let mut lines = Vec::new();
    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no matching commands",
            theme.style(Tone::Warning),
        )));
    } else {
        let capacity = usize::from(area.height).saturating_sub(usize::from(error.is_some()));
        let visible = capacity.min(matches.len());
        let start = selected
            .saturating_sub(visible / 2)
            .min(matches.len().saturating_sub(visible));
        for (index, command) in matches.into_iter().enumerate().skip(start).take(visible) {
            let marker = if index == selected { "› " } else { "  " };
            let hint = if command.argument_hint.is_empty() {
                String::new()
            } else {
                format!(" {}", command.argument_hint)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{marker}/{}{hint}", command.name),
                    theme.style(if index == selected {
                        Tone::Accent
                    } else {
                        Tone::Default
                    }),
                ),
                Span::styled(format!("  {}", command.description), theme.style(Tone::Dim)),
            ]));
        }
    }
    if let Some(error) = error {
        lines.push(Line::from(Span::styled(
            format!("  {} {error}", glyph::ERROR),
            theme.style(Tone::Danger),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

pub(super) fn desired_palette_rows(app: &App, error: Option<&str>) -> u16 {
    let matches = commands::matches(app.composer.text()).len().max(1);
    u16::try_from(matches.saturating_add(usize::from(error.is_some()))).unwrap_or(u16::MAX)
}

pub(super) fn draw_history_search(
    frame: &mut Frame<'_>,
    area: Rect,
    query: &str,
    matched: Option<&str>,
    theme: Theme,
) {
    let query_empty = query.is_empty();
    let query = if query_empty { "type query" } else { query };
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
}

pub(super) fn draw_recovery_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    content: &str,
    theme: Theme,
) {
    let action = if title.starts_with("revert") {
        "apply revert"
    } else {
        "apply undo"
    };
    let mut lines = vec![Line::from(Span::styled(
        "No action is selected by default. Review the complete reverse patch.",
        theme.style(Tone::Warning),
    ))];
    lines.extend(
        content
            .lines()
            .take(MAX_BODY_LINES.saturating_sub(2))
            .map(|line| Line::from(line.to_owned())),
    );
    lines.push(Line::from(vec![
        Span::styled("y", theme.style(Tone::Danger)),
        Span::styled(format!(" {action}   "), theme.style(Tone::Dim)),
        Span::styled("n/esc", theme.style(Tone::Success)),
        Span::styled(" cancel", theme.style(Tone::Dim)),
    ]));
    draw_modal(frame, area, title, lines, theme, Tone::Warning);
}

pub(super) fn draw_review_confirm(frame: &mut Frame<'_>, area: Rect, content: &str, theme: Theme) {
    let mut lines = content
        .lines()
        .take(MAX_BODY_LINES.saturating_sub(2))
        .map(|line| Line::from(line.to_owned()))
        .collect::<Vec<_>>();
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("y", theme.style(Tone::Accent)),
        Span::styled(" start provider-backed review   ", theme.style(Tone::Dim)),
        Span::styled("n/esc", theme.style(Tone::Success)),
        Span::styled(" cancel", theme.style(Tone::Dim)),
    ]));
    draw_modal(frame, area, "read-only review", lines, theme, Tone::Accent);
}

pub(super) fn draw_agent_confirm(frame: &mut Frame<'_>, area: Rect, content: &str, theme: Theme) {
    draw_child_continuation_confirm(
        frame,
        area,
        "read-only child agent",
        " start child and spend provider tokens   ",
        content,
        theme,
    );
}

pub(super) fn draw_child_continuation_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    action: &str,
    content: &str,
    theme: Theme,
) {
    let mut lines = content
        .lines()
        .take(MAX_BODY_LINES.saturating_sub(2))
        .map(|line| Line::from(line.to_owned()))
        .collect::<Vec<_>>();
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("y", theme.style(Tone::Accent)),
        Span::styled(action.to_owned(), theme.style(Tone::Dim)),
        Span::styled("n/esc", theme.style(Tone::Success)),
        Span::styled(" cancel", theme.style(Tone::Dim)),
    ]));
    draw_modal(frame, area, title, lines, theme, Tone::Accent);
}

pub(super) fn draw_exit_confirm(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    pending_approval: bool,
    pending_questionnaire: bool,
    theme: Theme,
) {
    let mut lines = vec![
        Line::from(Span::styled(
            "quit with work in progress?",
            theme.style(Tone::Heading),
        )),
        Line::default(),
    ];
    if app.is_busy() {
        lines.push(Line::from(Span::raw("· a turn is still running")));
    }
    if pending_approval {
        lines.push(Line::from(Span::raw("· an approval is pending")));
    }
    if pending_questionnaire {
        lines.push(Line::from(Span::raw("· a questionnaire is pending")));
    }
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::styled("y", theme.style(Tone::Danger)),
        Span::styled(" quit   ", theme.style(Tone::Dim)),
        Span::styled("n", theme.style(Tone::Success)),
        Span::styled(" keep working", theme.style(Tone::Dim)),
    ]));

    draw_modal(frame, area, "exit", lines, theme, Tone::Warning);
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
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
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
