//! Local harness feedback review, excluded from model history.

/// A row in the newest-first feedback listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackRow {
    /// Stable local report identifier.
    pub id: String,
    /// Elapsed age captured by the host.
    pub age: String,
    /// Local review disposition.
    pub status: String,
    /// Report category.
    pub kind: String,
    /// Affected Smith subsystem.
    pub area: String,
    /// Report title.
    pub title: String,
    /// Number of occurrences.
    pub count: u64,
}

/// Host-captured feedback, with no dependency on the feedback module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedbackReport {
    /// Newest-first reports and unreadable filenames.
    List {
        /// Valid report rows.
        rows: Vec<FeedbackRow>,
        /// Files skipped while reading the store.
        malformed: Vec<String>,
    },
    /// Full local report, including evidence.
    Detail {
        /// Stable report identifier.
        id: String,
        /// Host-formatted fields and evidence.
        content: String,
    },
    /// Exact public issue text before hand-off.
    Preview {
        /// Public issue title.
        title: String,
        /// Public issue body.
        body: String,
    },
    /// No stored reports.
    Empty,
    /// Store or build availability failure.
    Error(String),
    /// Local disposition or browser hand-off outcome.
    Outcome(String),
}

/// Renders literal report text for plain captures and terminal clients.
pub fn render_plain(report: &FeedbackReport) -> String {
    match report {
        FeedbackReport::List { rows, malformed } => {
            let mut lines = rows
                .iter()
                .map(|r| {
                    format!(
                        "{} · {} · {} · {} · {} · {} · count {}",
                        r.id, r.age, r.status, r.kind, r.area, r.title, r.count
                    )
                })
                .collect::<Vec<_>>();
            lines.extend(
                malformed
                    .iter()
                    .map(|name| format!("Malformed report: {name}")),
            );
            lines.join("\n")
        }
        FeedbackReport::Detail { id, content } => format!("{id}\n{content}"),
        FeedbackReport::Preview { title, body } => format!("GitHub issue title: {title}\n\n{body}"),
        FeedbackReport::Empty => "No harness feedback reports stored.".into(),
        FeedbackReport::Error(error) => format!("Harness feedback: {error}"),
        FeedbackReport::Outcome(message) => message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feedback_list_names_malformed_and_preserves_rows() {
        let report = FeedbackReport::List {
            rows: vec![FeedbackRow {
                id: "hf-1".into(),
                age: "2m ago".into(),
                status: "queued".into(),
                kind: "bug".into(),
                area: "tools".into(),
                title: "Failed edit".into(),
                count: 3,
            }],
            malformed: vec!["broken.md".into()],
        };
        assert_eq!(
            render_plain(&report),
            "hf-1 · 2m ago · queued · bug · tools · Failed edit · count 3\nMalformed report: broken.md"
        );
    }
    #[test]
    fn feedback_preview_preserves_exact_public_text() {
        assert_eq!(
            render_plain(&FeedbackReport::Preview {
                title: "Title".into(),
                body: "## What happened\n<literal>\n\nBody".into()
            }),
            "GitHub issue title: Title\n\n## What happened\n<literal>\n\nBody"
        );
    }
    #[test]
    fn feedback_empty_error_and_detail() {
        assert!(render_plain(&FeedbackReport::Empty).contains("No harness"));
        assert_eq!(
            render_plain(&FeedbackReport::Error("missing".into())),
            "Harness feedback: missing"
        );
        assert_eq!(
            render_plain(&FeedbackReport::Detail {
                id: "hf-1".into(),
                content: "all evidence".into()
            }),
            "hf-1\nall evidence"
        );
    }
}

#[cfg(test)]
mod command_tests {
    use crate::commands::{Command, FeedbackAction, HostCommand, parse};
    #[test]
    fn feedback_routes_locally_and_rejects_extra_arguments() {
        assert_eq!(
            parse("/feedback").unwrap().command,
            Command::Host(HostCommand::Feedback(FeedbackAction::Show(None)))
        );
        assert_eq!(
            parse("/feedback hf-1").unwrap().command,
            Command::Host(HostCommand::Feedback(FeedbackAction::Show(Some(
                "hf-1".into()
            ))))
        );
        assert!(parse("/feedback hf-1 file").is_err());
    }
}
