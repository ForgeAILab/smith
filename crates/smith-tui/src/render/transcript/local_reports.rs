use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use smith_client::agent_report::exact_resume_label;
use smith_client::context_report::{ContextCategoryKind, ContextCompaction, ContextReport};
use smith_client::diagnostics_report::{DiagnosticsReport, DiagnosticsRow};
use smith_client::goal_report::GoalReport;
use smith_client::help_report::{HelpCommand, HelpReport};
use smith_client::message_report::MessageReport;
use smith_client::shell_report::{ShellOutput, ShellReport};
use smith_client::status_report::{StatusGoal, StatusReport};
use smith_client::timeline_report::{TimelineEntry, TimelinePlan, TimelineReport};

use crate::theme::{Theme, Tone, glyph};

use super::super::helpers::wrap_text;
use super::super::reports;
use super::text::{context_field, render_inline_text_lines, report_field, wrap_context_line};
use super::{render_inline_markdown, render_prefixed_local_state};

pub(in crate::render) fn render_status_card(
    report: &StatusReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let mut fields = vec![
        ("session", report.session.clone()),
        ("profile", report.profile.clone()),
        ("provider", report.provider.clone()),
        ("model", report.model.clone()),
        ("permission", report.permission.clone()),
        ("reasoning", report.reasoning.clone()),
        ("reasoning controls", report.reasoning_controls.clone()),
        ("prompt cache", report.prompt_cache.clone()),
        ("cache maintenance", report.cache_maintenance.clone()),
        ("resume checkpoint", report.resume_checkpoint.clone()),
        ("project", report.project.clone()),
        ("Git", report.git.clone()),
    ];
    match &report.goal {
        StatusGoal::None => fields.push(("goal", "none".to_owned())),
        StatusGoal::Unavailable(error) => {
            fields.push(("goal", format!("unavailable ({error})")));
        }
        StatusGoal::Active(goal) => fields.extend([
            ("goal", goal.objective.clone()),
            ("status", goal.status.clone()),
            ("tokens", goal.tokens.clone()),
            ("budget", goal.budget.clone()),
            ("active elapsed", goal.active_elapsed.clone()),
            ("reason", goal.reason.clone()),
            ("id", goal.id.clone()),
        ]),
    }
    fields.extend([
        ("children", report.children.to_string()),
        ("usage", report.usage.clone()),
        ("cost", report.cost.clone()),
    ]);
    let label_width = reports::label_width(fields.iter().map(|(label, _)| *label), width);
    let value_width = usize::from(width).saturating_sub(label_width + 4);
    let mut lines = Vec::new();
    for (label, value) in fields {
        let value = if label == "project" {
            reports::left_shorten(&value, value_width)
        } else {
            value
        };
        lines.extend(report_field(
            label,
            &value,
            Tone::Default,
            label_width,
            width,
            theme,
        ));
    }
    lines.extend(reports::text(
        StatusReport::DIAGNOSTICS_HINT,
        width,
        theme.style(Tone::Default),
    ));
    lines
}

pub(super) fn render_diagnostics_report(
    report: &DiagnosticsReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let label_width = reports::label_width(
        report
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .filter_map(|row| match row {
                DiagnosticsRow::Field { label, .. } | DiagnosticsRow::Path { label, .. } => {
                    Some(label.as_str())
                }
                DiagnosticsRow::Line(_) => None,
            }),
        width,
    );
    let value_width = usize::from(width).saturating_sub(label_width + 4).max(1);
    let mut lines = Vec::new();
    for (index, section) in report.sections.iter().enumerate() {
        if index > 0 {
            lines.push(Line::default());
        }
        lines.extend(reports::text(
            &section.heading,
            width,
            theme.style(Tone::Heading),
        ));
        for row in &section.rows {
            match row {
                DiagnosticsRow::Field { label, value } => {
                    lines.extend(report_field(
                        label,
                        value,
                        Tone::Default,
                        label_width,
                        width,
                        theme,
                    ));
                }
                DiagnosticsRow::Path { label, value } => {
                    lines.extend(report_field(
                        label,
                        &reports::left_shorten(value, value_width),
                        Tone::Default,
                        label_width,
                        width,
                        theme,
                    ));
                }
                DiagnosticsRow::Line(content) => {
                    lines.extend(
                        reports::text(content, width, theme.style(Tone::Default))
                            .into_iter()
                            .map(|line| {
                                Line::from(render_inline_markdown(
                                    &line.to_string(),
                                    theme.style(Tone::Default),
                                    theme,
                                ))
                            }),
                    );
                }
            }
        }
    }
    lines
}

pub(super) fn render_shell_report(
    report: &ShellReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    match &report.output {
        ShellOutput::Empty => render_prefixed_local_state(
            glyph::BULLET,
            ShellReport::EMPTY_MESSAGE,
            width,
            theme.style(Tone::Dim),
        ),
        ShellOutput::Output(output) if report.is_error => {
            render_prefixed_local_state(glyph::ERROR, output, width, theme.style(Tone::Danger))
        }
        ShellOutput::Output(output) => render_inline_text_lines(output, width, theme),
    }
}

pub(super) fn render_message_report(
    report: &MessageReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    match report {
        MessageReport::Notice { message, .. } => render_inline_text_lines(message, width, theme),
        MessageReport::Empty { message, .. } => {
            render_prefixed_local_state(glyph::BULLET, message, width, theme.style(Tone::Dim))
        }
        MessageReport::Error { message, .. } => {
            render_prefixed_local_state(glyph::ERROR, message, width, theme.style(Tone::Danger))
        }
    }
}

pub(super) fn render_goal_report(
    report: &GoalReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let goal = match report {
        GoalReport::Empty => {
            return render_prefixed_local_state(
                glyph::BULLET,
                GoalReport::EMPTY_MESSAGE,
                width,
                theme.style(Tone::Dim),
            );
        }
        GoalReport::Unavailable(error) => {
            return render_prefixed_local_state(
                glyph::ERROR,
                error,
                width,
                theme.style(Tone::Danger),
            );
        }
        GoalReport::Cleared => {
            return wrap_context_line(
                Line::from(Span::styled(
                    GoalReport::CLEARED_MESSAGE,
                    theme.style(Tone::Default),
                )),
                usize::from(width).max(1),
            );
        }
        GoalReport::Snapshot(goal) => goal,
    };
    let available = usize::from(width).max(1);
    let mut lines = Vec::new();
    for raw in goal.objective.split('\n') {
        for wrapped in wrap_text(raw.strip_suffix('\r').unwrap_or(raw), available) {
            lines.push(Line::from(render_inline_markdown(
                &wrapped,
                theme.style(Tone::Default),
                theme,
            )));
        }
    }
    for (label, value) in [
        ("status", goal.status.clone()),
        (
            "tokens",
            format!(
                "{} · {}",
                goal.charged_tokens_value(),
                goal.usage_provenance
            ),
        ),
        ("budget", goal.budget_value()),
        ("active elapsed", goal.active_elapsed.clone()),
        ("reason", goal.reason_value()),
        (
            "id",
            format!("{} · generation {}", goal.id, goal.generation),
        ),
    ] {
        lines.extend(wrap_context_line(
            context_field(label, value, Tone::Default, theme),
            available,
        ));
    }
    lines
}

pub(super) fn render_timeline_report(
    report: &TimelineReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let entries = match report {
        TimelineReport::Empty => {
            return render_prefixed_local_state(
                glyph::BULLET,
                TimelineReport::EMPTY_MESSAGE,
                width,
                theme.style(Tone::Dim),
            );
        }
        TimelineReport::Unavailable(error) => {
            return render_prefixed_local_state(
                glyph::ERROR,
                &format!("timeline unavailable: {error}"),
                width,
                theme.style(Tone::Danger),
            );
        }
        TimelineReport::Entries(entries) => entries,
    };
    let available = usize::from(width).max(1);
    let mut lines = Vec::new();
    for entry in entries {
        let content = match entry {
            TimelineEntry::RootTurn {
                turn,
                finish,
                plan,
                passed_gates,
                failed_gates,
            } => format!(
                "root {turn} · {finish} · {} · gates {passed_gates} passed/{failed_gates} failed",
                plan.as_ref()
                    .map_or_else(|| "plan none".to_owned(), TimelinePlan::render_value),
            ),
            TimelineEntry::RootManifest {
                turn,
                provider,
                model,
                activated_capabilities,
            } => format!(
                "root {turn} · committed · {provider}/{model} · {activated_capabilities} activated capability/capabilities",
            ),
            TimelineEntry::ChildEvent { child, event } => {
                format!("child {child} · {}", event.render_value())
            }
            TimelineEntry::ChildSnapshot {
                child,
                session,
                durability,
                state,
                resumable,
                turns,
            } => format!(
                "child {child} · session {session} · {durability} · {state} · {} · {turns}",
                exact_resume_label(*resumable),
            ),
            TimelineEntry::Recovery { number, detail } => {
                format!("recovery recovery-{number} · {detail}")
            }
        };
        for raw in content.lines() {
            for wrapped in wrap_text(raw, available) {
                lines.push(Line::from(render_inline_markdown(
                    &wrapped,
                    theme.style(Tone::Default),
                    theme,
                )));
            }
        }
    }
    lines
}

pub(in crate::render) fn render_help_report(
    report: &HelpReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let commands = report
        .getting_started
        .iter()
        .chain(&report.primary)
        .chain(&report.advanced);
    let names = commands
        .map(|command| format!("/{}", command.name))
        .collect::<Vec<_>>();
    let name_width = reports::label_width(names.iter().map(String::as_str), width);
    let mut lines = reports::text(
        HelpReport::START_HERE_HEADING,
        width,
        theme.style(Tone::Heading),
    );
    lines.extend(reports::text(
        &report.introduction,
        width,
        theme.style(Tone::Dim),
    ));
    for command in &report.getting_started {
        lines.extend(render_help_command(command, name_width, width, theme));
    }
    for (heading, commands) in [
        (HelpReport::PRIMARY_HEADING, &report.primary),
        (HelpReport::ADVANCED_HEADING, &report.advanced),
    ] {
        lines.push(Line::default());
        lines.extend(reports::text(heading, width, theme.style(Tone::Heading)));
        for command in commands {
            lines.extend(render_help_command(command, name_width, width, theme));
        }
    }
    lines.push(Line::default());
    lines.extend(reports::text(
        HelpReport::KEYS_HEADING,
        width,
        theme.style(Tone::Heading),
    ));
    let key_width = reports::label_width(report.keys.iter().map(|key| key.key.as_str()), width);
    for key in &report.keys {
        lines.extend(report_field(
            &key.key,
            &key.description,
            Tone::Default,
            key_width,
            width,
            theme,
        ));
    }
    lines
}

fn render_help_command(
    command: &HelpCommand,
    name_width: usize,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let mut lines = reports::field(
        Line::from(Span::styled(
            format!("/{}", command.name),
            theme.style(Tone::Code),
        )),
        &command.description,
        theme.style(Tone::Dim),
        name_width,
        width,
    );
    if !command.argument_hint.is_empty() {
        lines.extend(reports::field(
            Line::default(),
            &command.argument_hint,
            theme.style(Tone::Dim),
            name_width,
            width,
        ));
    }
    lines
}

pub(in crate::render) fn render_context_report(
    report: &ContextReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let mut fields = vec![
        ("model window", report.model_window.clone(), Tone::Default),
        ("counting", report.counting.clone(), Tone::Warning),
        (
            "compaction",
            report.compaction.render_value(),
            match report.compaction {
                ContextCompaction::Applied { .. } => Tone::Success,
                ContextCompaction::Enabled { .. } => Tone::Default,
            },
        ),
        ("tool context", report.tool_context.clone(), Tone::Default),
        (
            "provider input (session)",
            report.provider_input.clone(),
            Tone::Default,
        ),
        (
            "cache read (session)",
            report.cache_read.clone(),
            Tone::Default,
        ),
        ("cache", report.cache.clone(), Tone::Default),
        ("reasoning", report.reasoning.clone(), Tone::Default),
        (
            "reasoning controls",
            report.reasoning_controls.clone(),
            Tone::Default,
        ),
    ];
    if !report.available_windows.is_empty() {
        fields.insert(
            0,
            (
                "available context windows",
                report.available_windows_value(),
                Tone::Default,
            ),
        );
    }
    let categories = report
        .categories
        .iter()
        .map(|category| {
            (
                category.kind,
                category.label.as_str(),
                category.value.as_str(),
            )
        })
        .chain([
            (
                ContextCategoryKind::Free,
                "free input",
                report.free_input.value.as_str(),
            ),
            (
                ContextCategoryKind::Reserve,
                "output/reasoning reserve",
                report.reserve.value.as_str(),
            ),
        ])
        .collect::<Vec<_>>();
    let labels = categories
        .iter()
        .map(|(_, label, _)| format!("  {label}"))
        .chain(fields.iter().map(|(label, _, _)| (*label).to_owned()))
        .collect::<Vec<_>>();
    let label_width = reports::label_width(labels.iter().map(String::as_str), width);
    let mut lines = reports::text(ContextReport::HEADING, width, theme.style(Tone::Heading));
    if !report.available_windows.is_empty() {
        let (label, value, tone) = fields.remove(0);
        lines.extend(report_field(label, &value, tone, label_width, width, theme));
    }
    lines.extend(reports::text(
        &report.summary,
        width,
        theme.style(Tone::Dim),
    ));
    lines.push(Line::default());
    for row in report.grid() {
        let mut spans = vec![Span::raw("  ")];
        for (index, kind) in row.into_iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw(" "));
            }
            let (glyph, tone) = context_category_style(kind);
            spans.push(Span::styled(glyph, theme.style(tone)));
        }
        if report.grid_is_empty() {
            spans.push(Span::raw(" "));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::default());
    lines.extend(reports::text(
        report.usage.category_heading(),
        width,
        theme.style(Tone::Dim).add_modifier(Modifier::ITALIC),
    ));
    for (kind, label, value) in categories {
        let (glyph, tone) = context_category_style(kind);
        lines.extend(reports::field(
            Line::from(vec![
                Span::styled(format!("{glyph} "), theme.style(tone)),
                Span::styled(label.to_owned(), theme.style(Tone::Default)),
            ]),
            value,
            theme.style(Tone::Dim),
            label_width,
            width,
        ));
    }
    for (label, value, tone) in fields {
        lines.extend(report_field(label, &value, tone, label_width, width, theme));
        if label == "tool context" {
            lines.extend(reports::text(
                ContextReport::OCCUPANCY_HINT,
                width,
                theme.style(Tone::Dim),
            ));
        }
    }
    lines
}

fn context_category_style(kind: ContextCategoryKind) -> (&'static str, Tone) {
    match kind {
        ContextCategoryKind::System => (glyph::CONTEXT_SYSTEM, Tone::Accent),
        ContextCategoryKind::Tool => (glyph::CONTEXT_TOOL, Tone::Warning),
        ContextCategoryKind::History => (glyph::CONTEXT_HISTORY, Tone::Command),
        ContextCategoryKind::Summary => (glyph::CONTEXT_SUMMARY, Tone::Success),
        ContextCategoryKind::Input => (glyph::CONTEXT_INPUT, Tone::Accent),
        ContextCategoryKind::Other => (glyph::CONTEXT_OTHER, Tone::Default),
        ContextCategoryKind::Free => (glyph::CONTEXT_FREE, Tone::Dim),
        ContextCategoryKind::Reserve => (glyph::CONTEXT_RESERVE, Tone::Dim),
    }
}
