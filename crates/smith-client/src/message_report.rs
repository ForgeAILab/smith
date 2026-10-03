//! Free-text local notices, empty results, and failures.

/// A local message whose presentation is explicit, without inferred structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageReport {
    /// An informational message.
    Notice {
        /// Display title; never used to select presentation.
        title: String,
        /// Free-text message.
        message: String,
    },
    /// A successful command with no matching data.
    Empty {
        /// Display title; never used to select presentation.
        title: String,
        /// Free-text explanation.
        message: String,
    },
    /// A local command failure.
    Error {
        /// Display title; never used to select presentation.
        title: String,
        /// Free-text failure explanation.
        message: String,
    },
}

impl MessageReport {
    /// The display title, independent of the message's semantic outcome.
    pub fn title(&self) -> &str {
        match self {
            Self::Notice { title, .. } | Self::Empty { title, .. } | Self::Error { title, .. } => {
                title
            }
        }
    }
}

/// Renders the existing transcript body without terminal dependencies.
pub fn render_plain(report: &MessageReport) -> String {
    match report {
        MessageReport::Notice { message, .. }
        | MessageReport::Empty { message, .. }
        | MessageReport::Error { message, .. } => message.clone(),
    }
}
