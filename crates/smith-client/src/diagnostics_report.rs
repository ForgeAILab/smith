//! The local `/diagnostics` snapshot and its plain-text rendering.
//!
//! Sections retain the existing row order without adding visible headings.
//! The host supplies rows as fields or free text; clients do not parse prose
//! to recover their structure.

/// Detailed session information captured when `/diagnostics` is invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsReport {
    /// The existing groups of diagnostic rows, in display order.
    pub sections: Vec<DiagnosticsSection>,
}

/// An unheaded group of rows in the existing diagnostics output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsSection {
    /// Label/value fields and free-text lines, in display order.
    pub rows: Vec<DiagnosticsRow>,
}

/// A diagnostic row whose presentation is explicit at the host boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticsRow {
    /// A label and its display value, including any existing indentation.
    Field {
        /// Field label, without its separating colon.
        label: String,
        /// Display value, without the label or its separating space.
        value: String,
    },
    /// Free text, including any existing indentation or line breaks.
    Line(String),
}

/// Renders the transcript body for the existing plain-text capture surface.
/// No terminal libraries or title/label parsing are involved.
pub fn render_plain(report: &DiagnosticsReport) -> String {
    report
        .sections
        .iter()
        .flat_map(|section| &section.rows)
        .map(|row| match row {
            DiagnosticsRow::Field { label, value } => format!("{label}: {value}"),
            DiagnosticsRow::Line(line) => line.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
