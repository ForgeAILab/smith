use ratatui::text::{Line, Span};
use smith_client::mcp_report::McpReport;
use smith_client::skills_report::SkillsReport;

use crate::theme::{Theme, Tone, glyph};

use super::super::helpers::wrap_text;
use super::text::{context_field, render_inline_text_lines, wrap_context_line};
use super::{render_inline_markdown, render_prefixed_local_state};

pub(super) fn render_mcp_report(
    report: &McpReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let servers = match report {
        McpReport::Empty { guidance } => {
            return render_mcp_text(
                if *guidance {
                    McpReport::EMPTY_GUIDANCE
                } else {
                    McpReport::EMPTY_MESSAGE
                },
                width,
                theme,
            );
        }
        McpReport::Unavailable => {
            return render_prefixed_local_state(
                glyph::ERROR,
                McpReport::EMPTY_MESSAGE,
                width,
                theme.style(Tone::Danger),
            );
        }
        McpReport::Error(error) => {
            return render_prefixed_local_state(
                glyph::ERROR,
                error,
                width,
                theme.style(Tone::Danger),
            );
        }
        McpReport::Trusted { server, digest } => {
            return render_mcp_text(&McpReport::trusted_value(server, digest), width, theme);
        }
        McpReport::Servers(servers) => servers,
    };
    let available = usize::from(width).max(1);
    let mut lines = Vec::new();
    for server in servers {
        lines.extend(render_mcp_text(
            &format!(
                "{} · {} · {} · {}",
                server.name,
                server.transport,
                server.state.render_value(&server.name),
                server.source,
            ),
            width,
            theme,
        ));
        for rejected in &server.rejected {
            lines.extend(wrap_context_line(
                context_field("  refused a tool", rejected.clone(), Tone::Default, theme),
                available,
            ));
        }
        for value in &server.values {
            lines.extend(wrap_context_line(
                Line::from(vec![
                    Span::styled(
                        format!("  {} {} ← ", value.kind.label(), value.name),
                        theme.style(Tone::Default),
                    ),
                    Span::styled(
                        value
                            .credential
                            .as_deref()
                            .unwrap_or("value withheld")
                            .to_owned(),
                        theme.style(Tone::Default),
                    ),
                ]),
                available,
            ));
        }
    }
    lines
}

/// Retains character wrapping before inline Markdown for the typed MCP fields.
fn render_mcp_text(content: &str, width: u16, theme: Theme) -> Vec<Line<'static>> {
    content
        .lines()
        .flat_map(|raw| {
            wrap_text(raw, usize::from(width).max(1))
                .into_iter()
                .map(|wrapped| {
                    Line::from(render_inline_markdown(
                        &wrapped,
                        theme.style(Tone::Default),
                        theme,
                    ))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(super) fn render_skills_report(
    report: &SkillsReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let (groups, problems) = match report {
        SkillsReport::Empty => {
            return render_inline_text_lines(SkillsReport::EMPTY_MESSAGE, width, theme);
        }
        SkillsReport::Error(error) => {
            return render_prefixed_local_state(
                glyph::ERROR,
                error,
                width,
                theme.style(Tone::Danger),
            );
        }
        SkillsReport::Trusted { skill, digest } => {
            return render_inline_text_lines(
                &SkillsReport::trusted_value(skill, digest),
                width,
                theme,
            );
        }
        SkillsReport::Indexed { groups, problems } => (groups, problems),
    };
    let mut lines = Vec::new();
    for group in groups {
        lines.extend(wrap_context_line(
            Line::from(Span::styled(
                group.layer.as_str(),
                theme.style(Tone::Default),
            )),
            usize::from(width).max(1),
        ));
        for entry in &group.entries {
            lines.extend(render_skills_free_text(
                &format!(
                    "  {} · {} · ",
                    entry.name,
                    entry.state.render_value(&entry.name),
                ),
                &entry.description,
                "",
                width,
                theme,
            ));
        }
    }
    if groups.is_empty() {
        lines.extend(render_inline_text_lines(
            SkillsReport::EMPTY_MESSAGE,
            width,
            theme,
        ));
    }
    if !problems.is_empty() {
        lines.extend(wrap_context_line(
            Line::from(Span::styled("not loaded", theme.style(Tone::Default))),
            usize::from(width).max(1),
        ));
        for problem in problems {
            lines.extend(render_skills_free_text(
                &format!("  {} · ", problem.name),
                &problem.reason,
                &format!(" · {}", problem.path),
                width,
                theme,
            ));
        }
    }
    lines
}

/// Wraps the complete row before styling only its free-text field.
fn render_skills_free_text(
    prefix: &str,
    text: &str,
    suffix: &str,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let content = format!("{prefix}{text}{suffix}");
    let text_end = prefix.len() + text.len();
    let mut offset = 0;
    let mut lines = Vec::new();
    for raw_line in content.split_inclusive('\n') {
        let raw = raw_line
            .strip_suffix('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .unwrap_or(raw_line);
        let mut row_offset = offset;
        for wrapped in wrap_text(raw, usize::from(width).max(1)) {
            let prefix_end = prefix.len().saturating_sub(row_offset).min(wrapped.len());
            let free_end = text_end.saturating_sub(row_offset).min(wrapped.len());
            let mut spans =
                render_inline_markdown(&wrapped[..prefix_end], theme.style(Tone::Default), theme);
            if prefix_end < free_end {
                spans.extend(render_inline_markdown(
                    &wrapped[prefix_end..free_end],
                    theme.style(Tone::Default),
                    theme,
                ));
            }
            if free_end < wrapped.len() {
                spans.push(Span::styled(
                    wrapped[free_end..].to_owned(),
                    theme.style(Tone::Default),
                ));
            }
            row_offset += wrapped.len();
            lines.push(Line::from(spans));
        }
        offset += raw_line.len();
    }
    lines
}
