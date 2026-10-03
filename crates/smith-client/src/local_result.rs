//! Typed local command results, independent of their presentation.
//!
//! Commands migrate one at a time from [`LocalResult::Text`] to reports.
//! Terminal drawing belongs to `smith-tui`.

use crate::status_report::StatusReport;

/// Semantic state of a local command result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalResultState {
    /// Informational output.
    Info,
    /// A successful command with no matching data.
    Empty,
    /// A local command that could not produce its result.
    Error,
}

/// A host or client command's local output, excluded from model history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalResult {
    /// The session's status snapshot.
    Status(Box<StatusReport>),
    /// Transitional output for commands that have not migrated to reports.
    Text {
        /// Command or result title.
        title: String,
        /// Display content.
        body: String,
        /// Text-visible result state.
        state: LocalResultState,
    },
}

impl LocalResult {
    /// Command or result title, without inferring the report's type.
    pub fn title(&self) -> &str {
        match self {
            Self::Status(_) => "status",
            Self::Text { title, .. } => title,
        }
    }

    /// The result's semantic state.
    pub fn state(&self) -> LocalResultState {
        match self {
            Self::Status(_) => LocalResultState::Info,
            Self::Text { state, .. } => *state,
        }
    }
}
