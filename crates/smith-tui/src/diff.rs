//! A line differ, and the reviewable form of an `edit` approval.
//!
//! The `edit` tool replaces an exact string (`crates/smith-tools/src/edit.rs`),
//! so the material question at an approval is not "what are the arguments" but
//! "what will this file say afterwards". Rendering the escaped JSON blob asks a
//! user to authorize a mutation they cannot read.
//!
//! Two rules keep the answer honest:
//!
//! - **It refuses rather than approximates.** A different tool, malformed
//!   arguments, a binary or oversized payload, or a change with no line-level
//!   effect produces no review at all, and the modal falls back to rendering
//!   the raw arguments. A diff that lies about what will change is worse than
//!   no diff.
//! - **It is pure and deterministic.** Nothing here reads a file. The review
//!   describes the replacement the model proposed — the `old_string` region
//!   becoming `new_string` — not the whole file on disk, and the same
//!   arguments always produce the same lines.
//!
//! The review is computed once, when the request arrives, because the redraw
//! budget is 30 fps (`DESIGN.md` §6) and the arguments cannot change while the
//! modal is open.

use serde_json::Value;

use crate::theme::glyph;

/// The one tool whose arguments this module knows how to review.
const EDIT_TOOL: &str = "edit";

/// The most replacement text a review will consider, both sides together.
/// Beyond this the modal cannot show a useful fraction anyway, and the caller
/// is better served by the raw arguments than by a diff of a truncation.
const MAX_REVIEW_BYTES: usize = 16 * 1024;

/// The most lines a review will diff on either side. This is what bounds the
/// quadratic table below to a size a single frame can afford.
const MAX_REVIEW_LINES: usize = 400;

/// Unchanged lines kept on each side of a change. Two is enough to recognize
/// where a hunk landed without spending the modal on context.
const CONTEXT_LINES: usize = 2;

/// What a tab expands to.
///
/// A literal tab has a terminal-defined width, which would corrupt the column
/// grid `DESIGN.md` §3 depends on. Four spaces is not every editor's setting,
/// but it is deterministic and one column per cell.
const TAB: &str = "    ";

/// One line of a rendered diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A line both versions share.
    Context(String),
    /// A line only the old text has.
    Removed(String),
    /// A line only the new text has.
    Added(String),
    /// A run of unchanged lines omitted for being far from any change.
    Skipped(usize),
}

/// Diffs two texts by line.
///
/// The algorithm is a longest-common-subsequence walk over the lines that
/// remain after the identical head and tail are trimmed. At this size that
/// beats a Myers diff on every axis that matters: real edits change a few lines
/// inside a mostly-unchanged region, so trimming usually leaves a table of a
/// handful of cells, the result is minimal in the same sense Myers' is, and the
/// implementation is short enough to read and to test exhaustively. Ties break
/// toward the removal, so a replacement always reads `-` before `+`.
///
/// A trailing newline terminates the last line rather than starting an empty
/// one, so `"a\n"` and `"a"` diff as the same single line. Adding or removing a
/// final newline is therefore invisible here — [`EditReview::from_call`]
/// detects that case and declines to show a diff at all.
pub fn diff_lines(old: &str, new: &str) -> Vec<Change> {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);

    let prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = old_lines[prefix..]
        .iter()
        .rev()
        .zip(new_lines[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();

    let context = |line: &&str| Change::Context((*line).to_owned());
    let mut changes: Vec<Change> = old_lines[..prefix].iter().map(context).collect();
    changes.extend(subsequence(
        &old_lines[prefix..old_lines.len() - suffix],
        &new_lines[prefix..new_lines.len() - suffix],
    ));
    changes.extend(old_lines[old_lines.len() - suffix..].iter().map(context));
    changes
}

/// Splits text into display lines, tolerating CRLF and a missing final
/// newline.
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    // `split` yields a trailing empty element for a terminated final line.
    if lines.last().is_some_and(|last| last.is_empty()) {
        lines.pop();
    }
    lines
}

/// The LCS walk over the differing middle.
fn subsequence(old: &[&str], new: &[&str]) -> Vec<Change> {
    let (rows, columns) = (old.len(), new.len());
    let stride = columns + 1;
    let mut table = vec![0usize; (rows + 1) * stride];
    for row in (0..rows).rev() {
        for column in (0..columns).rev() {
            table[row * stride + column] = if old[row] == new[column] {
                table[(row + 1) * stride + column + 1] + 1
            } else {
                table[(row + 1) * stride + column].max(table[row * stride + column + 1])
            };
        }
    }

    let mut changes = Vec::new();
    let (mut row, mut column) = (0, 0);
    while row < rows && column < columns {
        if old[row] == new[column] {
            changes.push(Change::Context(old[row].to_owned()));
            row += 1;
            column += 1;
        } else if table[(row + 1) * stride + column] >= table[row * stride + column + 1] {
            changes.push(Change::Removed(old[row].to_owned()));
            row += 1;
        } else {
            changes.push(Change::Added(new[column].to_owned()));
            column += 1;
        }
    }
    changes.extend(
        old[row..]
            .iter()
            .map(|line| Change::Removed((*line).to_owned())),
    );
    changes.extend(
        new[column..]
            .iter()
            .map(|line| Change::Added((*line).to_owned())),
    );
    changes
}

/// Collapses runs of context further than `context` lines from any change into
/// a single [`Change::Skipped`].
///
/// Without this a change buried under three hundred identical lines would be
/// elided by the modal's height bound while its context filled the screen —
/// the review would be technically complete and practically useless.
fn condense(changes: Vec<Change>, context: usize) -> Vec<Change> {
    let changed: Vec<bool> = changes
        .iter()
        .map(|change| !matches!(change, Change::Context(_)))
        .collect();

    let mut condensed = Vec::with_capacity(changes.len());
    let mut skipped = 0usize;
    for (index, change) in changes.into_iter().enumerate() {
        let near = changed[index.saturating_sub(context)..(index + context + 1).min(changed.len())]
            .iter()
            .any(|&changed| changed);
        if near {
            if skipped > 0 {
                condensed.push(Change::Skipped(skipped));
                skipped = 0;
            }
            condensed.push(change);
        } else {
            skipped += 1;
        }
    }
    if skipped > 0 {
        condensed.push(Change::Skipped(skipped));
    }
    condensed
}

/// A reviewable rendering of an `edit` call, ready for the approval modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditReview {
    /// The file the edit targets, exactly as the model named it.
    pub path: String,
    /// Whether the call creates the file rather than editing an existing one.
    pub creates: bool,
    /// Whether the replacement applies at every occurrence, not just one.
    pub replace_all: bool,
    /// How many lines the edit removes.
    pub removed: usize,
    /// How many lines the edit adds.
    pub added: usize,
    /// The condensed changes, first line first.
    pub changes: Vec<Change>,
}

impl EditReview {
    /// Reviews a tool call, or returns `None` when it cannot be reviewed
    /// truthfully.
    ///
    /// Every rejection here sends the modal back to rendering raw arguments,
    /// which is correct but unreadable — and still better than a diff the user
    /// would trust and that would be wrong.
    pub fn from_call(tool: &str, arguments: &Value) -> Option<Self> {
        if tool != EDIT_TOOL {
            return None;
        }

        let object = arguments.as_object()?;
        let path = object.get("path")?.as_str()?;
        let old = object.get("old_string")?.as_str()?;
        let new = object.get("new_string")?.as_str()?;
        let replace_all = object
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if old.len() + new.len() > MAX_REVIEW_BYTES {
            return None;
        }
        if [path, old, new].iter().any(|text| is_binary(text)) {
            return None;
        }
        if line_count(old) > MAX_REVIEW_LINES || line_count(new) > MAX_REVIEW_LINES {
            return None;
        }

        let changes = diff_lines(&expand_tabs(old), &expand_tabs(new));
        let removed = changes
            .iter()
            .filter(|change| matches!(change, Change::Removed(_)))
            .count();
        let added = changes
            .iter()
            .filter(|change| matches!(change, Change::Added(_)))
            .count();

        // Nothing shows at line granularity — a line-ending or final-newline
        // change, say. An all-context "diff" would claim the edit is a no-op.
        if removed == 0 && added == 0 {
            return None;
        }

        Some(Self {
            path: path.to_owned(),
            creates: old.is_empty(),
            replace_all,
            removed,
            added,
            changes: condense(changes, CONTEXT_LINES),
        })
    }

    /// A one-line summary of the change's shape and scope.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.creates {
            parts.push("new file".to_owned());
        } else {
            parts.push(format!("{} removed", self.removed));
        }
        parts.push(format!("{} added", self.added));
        if self.replace_all {
            // The diff shows one occurrence; the edit applies to all of them.
            parts.push("at every occurrence".to_owned());
        }
        parts.join(&format!(" {} ", glyph::SEPARATOR))
    }
}

/// Whether text carries anything that cannot be drawn on a character grid.
fn is_binary(text: &str) -> bool {
    text.chars()
        .any(|ch| ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t')
}

fn line_count(text: &str) -> usize {
    text.split('\n').count()
}

fn expand_tabs(text: &str) -> String {
    text.replace('\t', TAB)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn removed(changes: &[Change]) -> Vec<&str> {
        changes
            .iter()
            .filter_map(|change| match change {
                Change::Removed(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn added(changes: &[Change]) -> Vec<&str> {
        changes
            .iter()
            .filter_map(|change| match change {
                Change::Added(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn edit(arguments: Value) -> Option<EditReview> {
        EditReview::from_call("edit", &arguments)
    }

    #[test]
    fn identical_texts_produce_no_removals_or_additions() {
        let changes = diff_lines("a\nb\nc\n", "a\nb\nc\n");
        assert_eq!(
            changes,
            vec![
                Change::Context("a".into()),
                Change::Context("b".into()),
                Change::Context("c".into()),
            ]
        );
    }

    #[test]
    fn a_pure_insertion_marks_only_the_new_line() {
        let changes = diff_lines("a\nc\n", "a\nb\nc\n");
        assert_eq!(removed(&changes), Vec::<&str>::new());
        assert_eq!(added(&changes), vec!["b"]);
        assert_eq!(changes.len(), 3);
    }

    #[test]
    fn a_pure_deletion_marks_only_the_removed_line() {
        let changes = diff_lines("a\nb\nc\n", "a\nc\n");
        assert_eq!(removed(&changes), vec!["b"]);
        assert_eq!(added(&changes), Vec::<&str>::new());
    }

    #[test]
    fn a_replacement_shows_the_removal_before_the_addition() {
        let changes = diff_lines("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(
            changes,
            vec![
                Change::Context("a".into()),
                Change::Removed("b".into()),
                Change::Added("B".into()),
                Change::Context("c".into()),
            ]
        );
    }

    #[test]
    fn empty_input_diffs_as_a_whole_side_added_or_removed() {
        assert_eq!(diff_lines("", ""), Vec::new());
        assert_eq!(added(&diff_lines("", "a\nb\n")), vec!["a", "b"]);
        assert_eq!(removed(&diff_lines("a\nb\n", "")), vec!["a", "b"]);
    }

    #[test]
    fn a_missing_trailing_newline_does_not_invent_a_line() {
        // The last line is the same line whether or not it is terminated.
        assert_eq!(
            diff_lines("a\nb", "a\nb\n"),
            vec![Change::Context("a".into()), Change::Context("b".into()),]
        );
        let changes = diff_lines("a\nb", "a\nc");
        assert_eq!(removed(&changes), vec!["b"]);
        assert_eq!(added(&changes), vec!["c"]);
    }

    #[test]
    fn crlf_endings_never_leak_a_carriage_return_into_a_line() {
        let changes = diff_lines("a\r\nb\r\n", "a\r\nc\r\n");
        assert_eq!(removed(&changes), vec!["b"]);
        assert_eq!(added(&changes), vec!["c"]);
        assert!(
            changes.iter().all(|change| match change {
                Change::Context(text) | Change::Removed(text) | Change::Added(text) =>
                    !text.contains('\r'),
                Change::Skipped(_) => true,
            }),
            "a carriage return would corrupt the column grid: {changes:?}"
        );
    }

    #[test]
    fn a_line_that_appears_twice_is_matched_once_not_twice() {
        let changes = diff_lines("x\nx\n", "x\ny\nx\n");
        assert_eq!(
            changes,
            vec![
                Change::Context("x".into()),
                Change::Added("y".into()),
                Change::Context("x".into()),
            ]
        );

        // The reverse direction must remove exactly one of the pair.
        let changes = diff_lines("a\nb\na\n", "a\n");
        assert_eq!(removed(&changes), vec!["b", "a"]);
    }

    #[test]
    fn context_far_from_any_change_collapses_into_a_skip() {
        let old: String = (0..20).map(|n| format!("line {n}\n")).collect();
        let new = old.replace("line 10\n", "line ten\n");
        let changes = condense(diff_lines(&old, &new), CONTEXT_LINES);

        assert!(
            changes.len() < 12,
            "the review must not be buried in context: {changes:?}"
        );
        assert!(
            matches!(changes.first(), Some(Change::Skipped(8))),
            "{changes:?}"
        );
        assert!(
            matches!(changes.last(), Some(Change::Skipped(7))),
            "{changes:?}"
        );
        assert_eq!(removed(&changes), vec!["line 10"]);
    }

    #[test]
    fn a_non_edit_call_is_not_reviewable() {
        assert!(EditReview::from_call("shell", &json!({"command": "rm -rf build"})).is_none());
        assert!(
            EditReview::from_call(
                "write",
                &json!({"path": "a.rs", "old_string": "a", "new_string": "b"})
            )
            .is_none()
        );
    }

    #[test]
    fn malformed_edit_arguments_are_not_reviewable() {
        assert!(edit(json!({"path": "a.rs", "new_string": "b"})).is_none());
        assert!(edit(json!({"path": 7, "old_string": "a", "new_string": "b"})).is_none());
        assert!(edit(json!({"path": "a.rs", "old_string": ["a"], "new_string": "b"})).is_none());
        assert!(edit(json!("not an object")).is_none());
        assert!(edit(Value::Null).is_none());
    }

    #[test]
    fn an_oversized_or_binary_payload_is_not_reviewable() {
        let huge = "x".repeat(MAX_REVIEW_BYTES + 1);
        assert!(edit(json!({"path": "a.rs", "old_string": huge, "new_string": "b"})).is_none());

        let many = "x\n".repeat(MAX_REVIEW_LINES + 1);
        assert!(edit(json!({"path": "a.rs", "old_string": many, "new_string": "b"})).is_none());

        assert!(
            edit(json!({"path": "a.bin", "old_string": "a\u{0}b", "new_string": "c"})).is_none()
        );
    }

    #[test]
    fn a_change_invisible_at_line_granularity_is_not_reviewable() {
        // Rewriting CRLF as LF is a real edit that a line diff cannot show, so
        // no diff is offered rather than one claiming nothing changes.
        assert!(
            edit(json!({"path": "a.rs", "old_string": "a\r\n", "new_string": "a\n"})).is_none()
        );
    }

    #[test]
    fn a_replacement_reviews_as_its_removed_and_added_lines() {
        let review = edit(json!({
            "path": "src/retry.rs",
            "old_string": "fn retry() {\n    once();\n}\n",
            "new_string": "fn retry(limit: u32) {\n    once();\n}\n",
        }))
        .expect("a reviewable edit");

        assert_eq!(review.path, "src/retry.rs");
        assert!(!review.creates);
        assert_eq!(review.removed, 1);
        assert_eq!(review.added, 1);
        assert_eq!(review.summary(), "1 removed · 1 added");
    }

    #[test]
    fn creating_a_file_reviews_as_all_additions() {
        let review = edit(json!({
            "path": "src/new.rs",
            "old_string": "",
            "new_string": "pub mod a;\npub mod b;\n",
        }))
        .expect("a reviewable creation");

        assert!(review.creates);
        assert_eq!(review.added, 2);
        assert_eq!(review.summary(), "new file · 2 added");
    }

    #[test]
    fn the_summary_says_when_every_occurrence_is_replaced() {
        let review = edit(json!({
            "path": "a.rs",
            "old_string": "= 1;",
            "new_string": "= 2;",
            "replace_all": true,
        }))
        .expect("a reviewable edit");
        assert_eq!(
            review.summary(),
            "1 removed · 1 added · at every occurrence"
        );
    }

    #[test]
    fn tabs_expand_so_the_column_grid_survives() {
        let review = edit(json!({
            "path": "Makefile",
            "old_string": "\techo old\n",
            "new_string": "\techo new\n",
        }))
        .expect("a reviewable edit");

        assert_eq!(added(&review.changes), vec!["    echo new"]);
        assert!(
            review
                .changes
                .iter()
                .all(|change| !matches!(change, Change::Added(text) if text.contains('\t'))),
            "a literal tab has terminal-defined width"
        );
    }
}
