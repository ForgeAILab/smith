use ratatui::text::Line;
use smith_client::diff_report::DiffLine;
use smith_client::recovery_report::RevertPreview;
use smith_client::review_report::{ReviewPreview, ReviewReport};

/// Keeps the confirmation's existing unstyled patch presentation. The modal
/// owns wrapping and scrolling; no heading or source prefix selects a style.
pub(in crate::render) fn render_review_preview(report: &ReviewPreview) -> Vec<Line<'static>> {
    let mut lines = format!("scope: {}\n", report.title)
        .lines()
        .map(|raw| Line::from(raw.to_owned()))
        .collect::<Vec<_>>();
    lines.extend([
        Line::from("provider-backed: yes"),
        Line::from("workspace authority: read-only"),
        Line::from(ReviewReport::AUTHORITY_MESSAGE),
        Line::default(),
    ]);
    lines.extend(
        report
            .patch
            .iter()
            .flat_map(|line| line.text.lines())
            .map(|raw| Line::from(raw.to_owned())),
    );
    lines
}

/// Recovery modals retain their unstyled source presentation. Patch roles and
/// report kinds arrive as data; displayed prefixes never choose presentation.
pub(in crate::render) fn render_recovery_patch(patch: &[DiffLine]) -> Vec<Line<'static>> {
    patch
        .iter()
        .flat_map(|line| line.text.lines())
        .map(|raw| Line::from(raw.to_owned()))
        .collect()
}

pub(in crate::render) fn render_revert_preview(report: &RevertPreview) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("origin: {}", report.origin.label())),
        Line::default(),
    ];
    lines.extend(render_recovery_patch(&report.patch));
    lines
}
