use ratatui::text::{Line, Span};
use smith_client::diff_report::{DiffLineKind, DiffOutcome, DiffReport};
use smith_client::recovery_report::{RecoveryAction, RecoveryReport};
use smith_client::review_report::{ReviewReport, ReviewStartReport};

use crate::theme::{Theme, Tone, glyph};

use super::super::helpers::{hanging_lines, wrap_text};
use super::{
    render_prefixed_local_state, render_recovery_patch, render_revert_preview,
    render_review_preview,
};

pub(super) fn render_diff_report(
    report: &DiffReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    let patch = match &report.outcome {
        DiffOutcome::Empty => {
            return render_prefixed_local_state(
                glyph::BULLET,
                DiffReport::EMPTY_MESSAGE,
                width,
                theme.style(Tone::Dim),
            );
        }
        DiffOutcome::Error(message) => {
            return render_prefixed_local_state(
                glyph::ERROR,
                message,
                width,
                theme.style(Tone::Danger),
            );
        }
        DiffOutcome::Patch(lines) => lines,
    };
    let available = usize::from(width).max(1);
    patch
        .iter()
        .flat_map(|line| {
            let tone = match line.kind {
                DiffLineKind::Hunk => Tone::Code,
                DiffLineKind::Addition => Tone::Success,
                DiffLineKind::Removal => Tone::Danger,
                DiffLineKind::Metadata => Tone::Dim,
                DiffLineKind::Context => Tone::Default,
            };
            line.text
                .lines()
                .flat_map(move |raw| wrap_text(raw, available))
                .map(move |wrapped| {
                    if wrapped.is_empty() {
                        Line::default()
                    } else {
                        Line::from(Span::styled(wrapped, theme.style(tone)))
                    }
                })
        })
        .collect()
}

pub(super) fn render_recovery_report(
    report: &RecoveryReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    match report {
        RecoveryReport::UndoConfirmation(preview) | RecoveryReport::RedoConfirmation(preview) => {
            let mut lines = vec![Line::from(Span::styled(
                format!("/{}", report.action().name()),
                theme.style(Tone::Command),
            ))];
            lines.extend(render_recovery_patch(&preview.patch));
            lines
        }
        RecoveryReport::RevertConfirmation(preview) => {
            let mut lines = vec![Line::from(Span::styled(
                "/revert",
                theme.style(Tone::Command),
            ))];
            lines.extend(render_revert_preview(preview));
            lines
        }
        RecoveryReport::PreviewError {
            action: RecoveryAction::Redo,
            message,
        } => {
            let mut lines = vec![Line::from(Span::styled(
                "/redo",
                theme.style(Tone::Command),
            ))];
            lines.extend(render_prefixed_local_state(
                glyph::ERROR,
                message,
                width,
                theme.style(Tone::Danger),
            ));
            lines
        }
        RecoveryReport::PreviewError { message, .. }
        | RecoveryReport::ApplyError { message, .. } => render_review_error(message, theme),
        RecoveryReport::RevertUsage => render_review_error(RecoveryReport::REVERT_USAGE, theme),
        RecoveryReport::Applied(applied) => render_report_notice(
            report.action().name(),
            &applied.render_value(),
            width,
            theme,
        ),
        RecoveryReport::Cancelled(action) => render_report_notice(
            action.name(),
            RecoveryReport::CANCELLED_MESSAGE,
            width,
            theme,
        ),
    }
}

pub(super) fn render_review_report(
    report: &ReviewReport,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    match report {
        ReviewReport::Confirmation(preview) => {
            let mut lines = vec![Line::from(Span::styled(
                "/review",
                theme.style(Tone::Command),
            ))];
            lines.extend(render_review_preview(preview));
            lines
        }
        ReviewReport::Empty => render_review_notice(ReviewReport::EMPTY_MESSAGE, width, theme),
        ReviewReport::Error(message) => render_review_error(message, theme),
        ReviewReport::Start(start) => {
            let content = start.render_value();
            match start {
                ReviewStartReport::Started { .. } | ReviewStartReport::Queued { .. } => {
                    render_review_notice(&content, width, theme)
                }
                ReviewStartReport::Unavailable
                | ReviewStartReport::AtCapacity { .. }
                | ReviewStartReport::Failed(_) => render_review_error(&content, theme),
            }
        }
    }
}

fn render_review_notice(content: &str, width: u16, theme: Theme) -> Vec<Line<'static>> {
    render_report_notice("review", content, width, theme)
}

pub(super) fn render_report_notice(
    source: &str,
    content: &str,
    width: u16,
    theme: Theme,
) -> Vec<Line<'static>> {
    content
        .lines()
        .enumerate()
        .map(|(index, raw)| {
            if index == 0 {
                Line::from(vec![
                    Span::styled(format!("{} ", glyph::NOTICE), theme.style(Tone::Dim)),
                    Span::styled(source.to_owned(), theme.style(Tone::Heading)),
                    Span::styled(" · ", theme.style(Tone::Dim)),
                    Span::styled(raw.to_owned(), theme.style(Tone::Default)),
                ])
            } else {
                Line::from(Span::styled(format!("  {raw}"), theme.style(Tone::Dim)))
            }
        })
        .flat_map(|line| hanging_lines(line, width, 2))
        .collect()
}

fn render_review_error(message: &str, theme: Theme) -> Vec<Line<'static>> {
    message
        .lines()
        .enumerate()
        .map(|(index, raw)| {
            let marker = if index == 0 { glyph::ERROR } else { " " };
            Line::from(Span::styled(
                format!("{marker} {raw}"),
                theme.style(Tone::Danger),
            ))
        })
        .collect()
}
