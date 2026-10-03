//! Local `/diff` results and their plain-text rendering.
//!
//! The host classifies source patch lines before crossing the client boundary.
//! Titles are display values; terminal drawing uses the outcome and line kinds.

/// One Git-scope or last-Smith-turn inspection result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffReport {
    /// Existing scope title, used only for display.
    pub title: String,
    /// Inspection availability and the classified source patch.
    pub outcome: DiffOutcome,
}

impl DiffReport {
    /// The existing message for a Git scope without changes.
    pub const EMPTY_MESSAGE: &str = "No changes in this scope.";
}

/// Inspection state, independent of the title or patch text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffOutcome {
    /// The selected Git scope has no changes.
    Empty,
    /// Git inspection or the recovery preview was unavailable.
    Error(String),
    /// Source lines in their existing order, including any recovery notes.
    Patch(Vec<DiffLine>),
}

/// A source patch line whose semantic kind is supplied by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// Patch role, independent of the displayed prefix.
    pub kind: DiffLineKind,
    /// Original source text, including its line ending when present.
    pub text: String,
}

/// Patch roles used to choose terminal presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// A unified patch's hunk header.
    Hunk,
    /// An added line.
    Addition,
    /// A removed line.
    Removal,
    /// A file, index, or before/after header.
    Metadata,
    /// Unchanged content, source notes, or other patch information.
    Context,
}

/// Renders the transcript body for the existing plain-text capture surface.
/// Line endings remain byte-identical; no title or prefix parsing is involved.
pub fn render_plain(report: &DiffReport) -> String {
    match &report.outcome {
        DiffOutcome::Empty => DiffReport::EMPTY_MESSAGE.to_owned(),
        DiffOutcome::Error(message) => message.clone(),
        DiffOutcome::Patch(lines) => lines.iter().map(|line| line.text.as_str()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_diff_retains_source_line_endings_and_ignores_display_prefixes() {
        let report = DiffReport {
            title: "an arbitrary display title".to_owned(),
            outcome: DiffOutcome::Patch(vec![
                DiffLine {
                    kind: DiffLineKind::Addition,
                    text: "source without an addition prefix\r\n".to_owned(),
                },
                DiffLine {
                    kind: DiffLineKind::Context,
                    text: "\n".to_owned(),
                },
                DiffLine {
                    kind: DiffLineKind::Removal,
                    text: "+source without a final newline".to_owned(),
                },
            ]),
        };
        assert_eq!(
            render_plain(&report),
            "source without an addition prefix\r\n\n+source without a final newline",
        );
    }
}
