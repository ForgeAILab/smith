//! Local `/review` results and their plain-text rendering.
//!
//! Confirmation scopes, classified patches, and reviewer start outcomes cross
//! the host boundary as data. Terminal drawing belongs to `smith-tui`.

use crate::diff_report::{DiffLine, DiffReport};

/// A read-only review confirmation or its local inspection/start result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewReport {
    /// The selected Git scope has no changes.
    Empty,
    /// Git inspection was unavailable.
    Error(String),
    /// Provider-backed read-only review awaiting confirmation.
    Confirmation(ReviewPreview),
    /// The result of dispatching a confirmed reviewer.
    Start(ReviewStartReport),
}

impl ReviewReport {
    /// The existing message for a Git scope without changes.
    pub const EMPTY_MESSAGE: &str = DiffReport::EMPTY_MESSAGE;
    /// The existing explanation of the reviewer's workspace authority.
    pub const AUTHORITY_MESSAGE: &str =
        "The reviewer can read, list, and search but cannot edit or run shell commands.";

    /// Existing result title or notice source, selected from the result kind.
    pub fn title(&self) -> &str {
        match self {
            Self::Error(_)
            | Self::Start(
                ReviewStartReport::Unavailable
                | ReviewStartReport::AtCapacity { .. }
                | ReviewStartReport::Failed(_),
            ) => "error",
            Self::Empty
            | Self::Confirmation(_)
            | Self::Start(ReviewStartReport::Started { .. } | ReviewStartReport::Queued { .. }) => {
                "review"
            }
        }
    }
}

/// Scope and source patch shown before starting a provider-backed reviewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewPreview {
    /// Original Git scope used when the user confirms dispatch.
    pub scope: String,
    /// Existing inspection title, used only for display.
    pub title: String,
    /// Classified source lines in their existing order, with line endings.
    pub patch: Vec<DiffLine>,
}

/// Dispatch outcomes for the confirmed read-only reviewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewStartReport {
    /// The host has no coordinator to start a reviewer.
    Unavailable,
    /// The coordinator started the reviewer.
    Started {
        /// Reviewer child identity.
        child: String,
    },
    /// The coordinator queued the reviewer.
    Queued {
        /// Reviewer child identity.
        child: String,
    },
    /// The coordinator has no free child slot.
    AtCapacity {
        /// Number of running children.
        running: usize,
        /// Configured concurrency limit.
        limit: usize,
    },
    /// The coordinator rejected the reviewer.
    Failed(String),
}

impl ReviewStartReport {
    /// Existing outcome wording, without its notice or error marker.
    pub fn render_value(&self) -> String {
        match self {
            Self::Unavailable => {
                "read-only review is unavailable because delegation is not wired".to_owned()
            }
            Self::Started { child } => format!("read-only reviewer {child} started"),
            Self::Queued { child } => format!("read-only reviewer {child} queued"),
            Self::AtCapacity { running, limit } => format!(
                "review did not start: {running} children are already running (limit {limit})"
            ),
            Self::Failed(error) => format!("review did not start: {error}"),
        }
    }
}

/// Renders the existing transcript or confirmation body for plain capture.
/// Source patch line endings remain byte-identical.
pub fn render_plain(report: &ReviewReport) -> String {
    match report {
        ReviewReport::Empty => ReviewReport::EMPTY_MESSAGE.to_owned(),
        ReviewReport::Error(message) => message.clone(),
        ReviewReport::Confirmation(preview) => {
            let patch: String = preview
                .patch
                .iter()
                .map(|line| line.text.as_str())
                .collect();
            format!(
                "scope: {}\nprovider-backed: yes\nworkspace authority: read-only\n\
                 {}\n\n{patch}",
                preview.title,
                ReviewReport::AUTHORITY_MESSAGE,
            )
        }
        ReviewReport::Start(start) => start.render_value(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff_report::DiffLineKind;

    #[test]
    fn plain_review_keeps_confirmation_spacing_and_source_line_endings() {
        let report = ReviewReport::Confirmation(ReviewPreview {
            scope: "path#1".to_owned(),
            title: "an arbitrary inspection title".to_owned(),
            patch: vec![
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
            ],
        });
        assert_eq!(
            render_plain(&report),
            "scope: an arbitrary inspection title\nprovider-backed: yes\n\
             workspace authority: read-only\n\
             The reviewer can read, list, and search but cannot edit or run shell commands.\n\n\
             source without an addition prefix\r\n\n+source without a final newline",
        );
    }
}
