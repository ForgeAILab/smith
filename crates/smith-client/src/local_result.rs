//! Typed local command results, independent of their presentation.
//!
//! Commands migrate one at a time from [`LocalResult::Text`] to reports.
//! Terminal drawing belongs to `smith-tui`.

use crate::context_report::ContextReport;
use crate::help_report::HelpReport;
use crate::status_report::StatusReport;
use crate::timeline_report::TimelineReport;

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
    /// The session's context occupancy snapshot.
    Context(Box<ContextReport>),
    /// The command guide derived from the registry.
    Help(Box<HelpReport>),
    /// The session's root, child, and recovery timeline.
    Timeline(Box<TimelineReport>),
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
            Self::Context(_) => "context",
            Self::Help(_) => "help",
            Self::Timeline(_) => "timeline",
            Self::Text { title, .. } => title,
        }
    }

    /// The result's semantic state.
    pub fn state(&self) -> LocalResultState {
        match self {
            Self::Status(_) | Self::Context(_) | Self::Help(_) => LocalResultState::Info,
            Self::Timeline(report) => match report.as_ref() {
                TimelineReport::Empty => LocalResultState::Empty,
                TimelineReport::Unavailable(_) => LocalResultState::Error,
                TimelineReport::Entries(_) => LocalResultState::Info,
            },
            Self::Text { state, .. } => *state,
        }
    }
}
