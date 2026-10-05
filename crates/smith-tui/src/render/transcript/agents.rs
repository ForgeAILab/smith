use std::borrow::Cow;

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use smith_client::agent_report::{
    AgentReport, AgentResumeReport, AgentSnapshot, exact_resume_label,
};

use crate::app::App;
use crate::status::render_elapsed;
use crate::theme::{Theme, Tone, glyph};

use super::super::helpers::hanging_lines;
use super::super::markdown::render_assistant_lines;
use super::super::reports;
use super::blocks::conversation_lines;
use super::changes::render_report_notice;
use super::render_prefixed_local_state;

/// The inspected child's read-only view: one identity heading, then the
/// history the client kept for it.
///
/// Nothing here is invented. The child owns its own canonical transcript in
/// the runtime; what the parent process receives about it is bounded, so
/// what this draws is bounded too — but it is drawn by the same renderer as
/// the root timeline, because a delegated child is an agent that reports
/// back, not a different kind of thing.
pub(super) fn child_lines(app: &App, child: &str, theme: Theme, width: u16) -> Vec<Line<'static>> {
    let summary = app.children.get(child);
    let detail = app.inspected_detail();
    let state = summary.map_or(Cow::Borrowed("unknown"), |summary| summary.state.label());
    let elapsed = app
        .child_elapsed(child)
        .map(|elapsed| format!(" \u{b7} {}", render_elapsed(elapsed)))
        .unwrap_or_default();
    let mut lines = hanging_lines(
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
                if detail.is_some() {
                    elapsed
                } else {
                    format!(" \u{b7} {state}{elapsed}")
                },
                theme.style(summary.map_or(Tone::Dim, |summary| summary.state.tone())),
            ),
        ]),
        width,
        2,
    );
    lines.push(Line::default());
    if let Some(detail) = detail {
        lines.extend(render_agent_inspector(detail, width, theme));
        lines.push(Line::default());
    }

    let blocks = app.child_blocks(child);
    let speculative = app.child_speculative_text(child);
    if blocks.is_empty() && speculative.is_none() {
        // A child restored from a durable record has a state but no live
        // history in this process. Saying so beats an empty pane.
        if detail.is_none_or(|detail| detail.last_result.is_none()) {
            lines.extend(reports::text(
                "no activity recorded in this session",
                width,
                theme.style(Tone::Dim),
            ));
        }
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

pub(super) fn render_agent_report(
    report: &AgentReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
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
            .flat_map(|line| hanging_lines(line, width, 2))
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
        AgentReport::Parent => lines.extend(reports::text(
            AgentReport::PARENT_MESSAGE,
            width,
            theme.style(Tone::Default),
        )),
        AgentReport::List(children) => {
            for child in children {
                let content = format!(
                    "{} · {} · {} · {} · {} · {} tokens",
                    child.child,
                    child.durability.label(),
                    child.state.label(),
                    exact_resume_label(child.resumable),
                    child.turns_label(),
                    child.tokens_value(),
                );
                lines.extend(reports::text(&content, width, theme.style(Tone::Default)));
            }
        }
        AgentReport::Inspector(child) => lines.extend(render_agent_inspector(child, width, theme)),
        AgentReport::Resume(_) => {}
    }
    lines
}

/// Inspector fields word-wrap with indentation on every continuation row.
fn render_agent_inspector(child: &AgentSnapshot, width: u16, theme: Theme) -> Vec<Line<'static>> {
    let mut lines = [
        format!(
            "session {} · {} · {} · {} · {} tokens · {}",
            child.session,
            child.summary.durability.label(),
            child.summary.state.label(),
            child.summary.turns_label(),
            child.summary.tokens_value(),
            child.workspace,
        ),
        format!(
            "{}{}",
            exact_resume_label(child.summary.resumable),
            child
                .incompatibility
                .as_deref()
                .map(|reason| format!(" · incompatible: {reason}"))
                .unwrap_or_default(),
        ),
        "continue: type a follow-up below".to_owned(),
    ]
    .into_iter()
    .flat_map(|content| reports::text(&content, width, theme.style(Tone::Dim)))
    .collect::<Vec<_>>();
    if child.summary.resumable {
        lines.extend(reports::text(
            &format!("exact recovery: /agent resume {}", child.summary.child),
            width,
            theme.style(Tone::Dim),
        ));
    }
    if let Some(result) = &child.last_result {
        lines.extend(reports::text("result", width, theme.style(Tone::Heading)));
        let mut result_lines = render_assistant_lines(result, theme, width, false);
        if let Some(first) = result_lines.first_mut() {
            first.spans[0].content = "  ".into();
        }
        lines.extend(result_lines);
    }
    lines
}
