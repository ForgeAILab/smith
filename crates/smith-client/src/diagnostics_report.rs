//! The local `/diagnostics` snapshot and its plain-text rendering.
//!
//! Named sections group aligned fields and free text in display order.
//! The host supplies rows as fields, paths, or free text; clients do not
//! parse prose to recover their structure.

/// Detailed session information captured when `/diagnostics` is invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsReport {
    /// The named groups of diagnostic rows, in display order.
    pub sections: Vec<DiagnosticsSection>,
}

/// A headed group of diagnostic rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsSection {
    /// Visible group heading.
    pub heading: String,
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
    /// A labeled path, shortened from the left by column-based renderers.
    Path {
        /// Field label, without its separating colon.
        label: String,
        /// Full path value, without the label or its separating space.
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
        .map(|section| {
            std::iter::once(section.heading.clone())
                .chain(section.rows.iter().map(|row| match row {
                    DiagnosticsRow::Field { label, value }
                    | DiagnosticsRow::Path { label, value } => format!("{label}: {value}"),
                    DiagnosticsRow::Line(line) => line.clone(),
                }))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
