//! Local recovery and session-restore reports and plain-text rendering.
//!
//! Confirmation patches and apply outcomes cross the host boundary as data.
//! Terminal drawing belongs to `smith-tui`.

use crate::diff_report::DiffLine;

/// Metadata-only reconciliation after restoring a saved session.
/// Startup notices and headless output render the same report, retaining
/// each surface's existing wording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreReport {
    /// A saved turn was interrupted because the active tools changed.
    ActivationChanged {
        /// The interrupted turn's stable identity.
        turn: String,
    },
    /// Process-owned work was interrupted and was not restarted.
    EphemeralWork {
        /// Existing recovery reason label.
        reason: String,
        /// Number of interrupted children.
        children: usize,
        /// Number of interrupted monitors.
        monitors: usize,
        /// Number of interrupted background tasks.
        tasks: usize,
    },
}

impl RestoreReport {
    /// Existing transcript notice source.
    pub fn source(&self) -> &'static str {
        match self {
            Self::ActivationChanged { .. } => "session restored",
            Self::EphemeralWork { .. } => "recovery",
        }
    }
}

/// Renders the existing startup notice body, omitting empty process recovery.
pub fn render_restore_plain(report: &RestoreReport) -> Option<String> {
    match report {
        RestoreReport::ActivationChanged { turn } => Some(format!(
            "Available tools changed since turn {turn} was saved. Your conversation is restored; the unfinished action was not retried. Check previous changes before continuing. Any active goal is paused; use /goal resume to continue it.",
        )),
        RestoreReport::EphemeralWork {
            children,
            monitors,
            tasks,
            ..
        } => {
            let mut work = Vec::new();
            if *children > 0 {
                work.push(format!(
                    "{children} prior {}",
                    if *children == 1 { "child" } else { "children" }
                ));
            }
            if *monitors > 0 {
                work.push(format!(
                    "{monitors} prior {}",
                    if *monitors == 1 {
                        "monitor"
                    } else {
                        "monitors"
                    }
                ));
            }
            if *tasks > 0 {
                work.push(format!(
                    "{tasks} prior background {}",
                    if *tasks == 1 { "task" } else { "tasks" }
                ));
            }
            if work.is_empty() {
                return None;
            }
            Some(format!(
                "{} interrupted when the prior Smith process exited · not restarted",
                work.join(" and ")
            ))
        }
    }
}

/// Renders one existing headless metadata line without the stderr prefix.
/// Background tasks trigger the recovery line but retain its existing
/// child/monitor-only counts.
pub fn render_restore_headless_plain(report: &RestoreReport) -> Option<String> {
    match report {
        RestoreReport::ActivationChanged { turn } => Some(format!(
            "session restored: tools changed since turn {turn}; unfinished action not retried; check previous changes before continuing"
        )),
        RestoreReport::EphemeralWork {
            reason,
            children,
            monitors,
            tasks,
        } => (*children > 0 || *monitors > 0 || *tasks > 0).then(|| {
            format!(
                "recovery {reason} · {children} child(ren) interrupted · {monitors} monitor(s) interrupted · not restarted"
            )
        }),
    }
}

/// The recovery operation, independent of its displayed title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Restore the newest attributable Smith turn.
    Undo,
    /// Reapply the newest exact undone turn.
    Redo,
    /// Revert a selected file or hunk.
    Revert,
}

impl RecoveryAction {
    /// Existing command name and notice source.
    pub fn name(self) -> &'static str {
        match self {
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Revert => "revert",
        }
    }
}

/// Classified source patch shown before undoing or redoing a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryPreview {
    /// Original source lines, including turn notes and line endings.
    pub patch: Vec<DiffLine>,
}

/// Attribution of a selected revert, supplied by the host's change ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertOrigin {
    /// The latest Smith turn owns the selected path.
    Smith,
    /// The host cannot attribute the selected path to Smith.
    Unknown,
}

impl RevertOrigin {
    /// Existing attribution label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Smith => "Smith",
            Self::Unknown => "unknown",
        }
    }
}

/// Selected scope, stale-preview identity, and reverse patch for a revert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevertPreview {
    /// Original file or `file#hunk` scope used on confirmation.
    pub scope: String,
    /// Fingerprint checked immediately before applying the revert.
    pub fingerprint: String,
    /// Attribution supplied independently of the patch text.
    pub origin: RevertOrigin,
    /// Classified source lines in their original order, with line endings.
    pub patch: Vec<DiffLine>,
}

/// A successfully applied recovery operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryApplied {
    /// The last turn's exact edits were restored.
    Undo,
    /// The newest exact undone turn was reapplied.
    Redo,
    /// The selected change was reverted and recorded for recovery.
    Revert {
        /// Original file or hunk scope.
        scope: String,
    },
}

impl RecoveryApplied {
    /// Existing success wording, without the sourced-notice marker.
    pub fn render_value(&self) -> String {
        match self {
            Self::Undo => "restored the edits Smith made in the last turn".to_owned(),
            Self::Redo => "newest exact undone Smith turn was reapplied".to_owned(),
            Self::Revert { scope } => {
                format!("`{scope}` reverted · recoverable with /undo")
            }
        }
    }
}

/// A recovery confirmation, local inspection failure, or apply outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryReport {
    /// Reverse patch awaiting explicit undo confirmation.
    UndoConfirmation(RecoveryPreview),
    /// Forward patch awaiting explicit redo confirmation.
    RedoConfirmation(RecoveryPreview),
    /// Selective reverse patch awaiting explicit revert confirmation.
    RevertConfirmation(RevertPreview),
    /// The host could not produce the requested preview.
    PreviewError {
        /// Requested operation; redo retains its titled error presentation.
        action: RecoveryAction,
        /// Existing host error wording.
        message: String,
    },
    /// No selective-revert scope was supplied.
    RevertUsage,
    /// A confirmed operation succeeded.
    Applied(RecoveryApplied),
    /// A confirmed operation failed.
    ApplyError {
        /// Attempted operation.
        action: RecoveryAction,
        /// Existing host error wording.
        message: String,
    },
    /// The user explicitly cancelled the previewed operation.
    Cancelled(RecoveryAction),
}

impl RecoveryReport {
    /// Existing missing-scope usage message.
    pub const REVERT_USAGE: &str =
        "usage: /revert FILE or /revert FILE#HUNK; use /diff to choose a scope";
    /// Existing cancellation notice body.
    pub const CANCELLED_MESSAGE: &str = "cancelled";

    /// Requested operation, selected from the report's kind.
    pub fn action(&self) -> RecoveryAction {
        match self {
            Self::UndoConfirmation(_) | Self::Applied(RecoveryApplied::Undo) => {
                RecoveryAction::Undo
            }
            Self::RedoConfirmation(_) | Self::Applied(RecoveryApplied::Redo) => {
                RecoveryAction::Redo
            }
            Self::RevertConfirmation(_)
            | Self::RevertUsage
            | Self::Applied(RecoveryApplied::Revert { .. }) => RecoveryAction::Revert,
            Self::PreviewError { action, .. }
            | Self::ApplyError { action, .. }
            | Self::Cancelled(action) => *action,
        }
    }

    /// Existing local result title or notice source.
    pub fn title(&self) -> &'static str {
        match self {
            Self::PreviewError {
                action: RecoveryAction::Redo,
                ..
            } => "redo",
            Self::PreviewError { .. } | Self::RevertUsage | Self::ApplyError { .. } => "error",
            _ => self.action().name(),
        }
    }

    /// Whether the result uses the existing sourced-notice presentation.
    pub fn is_notice(&self) -> bool {
        matches!(self, Self::Applied(_) | Self::Cancelled(_))
    }
}

/// Renders the existing transcript or confirmation body for plain capture.
/// Source patch line endings remain byte-identical.
pub fn render_plain(report: &RecoveryReport) -> String {
    match report {
        RecoveryReport::UndoConfirmation(preview) | RecoveryReport::RedoConfirmation(preview) => {
            patch_text(&preview.patch)
        }
        RecoveryReport::RevertConfirmation(preview) => format!(
            "origin: {}\n\n{}",
            preview.origin.label(),
            patch_text(&preview.patch),
        ),
        RecoveryReport::PreviewError { message, .. }
        | RecoveryReport::ApplyError { message, .. } => message.clone(),
        RecoveryReport::RevertUsage => RecoveryReport::REVERT_USAGE.to_owned(),
        RecoveryReport::Applied(applied) => applied.render_value(),
        RecoveryReport::Cancelled(_) => RecoveryReport::CANCELLED_MESSAGE.to_owned(),
    }
}

fn patch_text(patch: &[DiffLine]) -> String {
    patch.iter().map(|line| line.text.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff_report::DiffLineKind;

    #[test]
    fn plain_recovery_keeps_source_bytes_and_typed_attribution() {
        let patch = vec![
            DiffLine {
                kind: DiffLineKind::Addition,
                text: "origin: unknown\r\n".to_owned(),
            },
            DiffLine {
                kind: DiffLineKind::Context,
                text: "\n".to_owned(),
            },
            DiffLine {
                kind: DiffLineKind::Removal,
                text: "+source without a final newline".to_owned(),
            },
        ];
        let source = "origin: unknown\r\n\n+source without a final newline";
        for report in [
            RecoveryReport::UndoConfirmation(RecoveryPreview {
                patch: patch.clone(),
            }),
            RecoveryReport::RedoConfirmation(RecoveryPreview {
                patch: patch.clone(),
            }),
        ] {
            assert_eq!(render_plain(&report), source);
        }
        for origin in [RevertOrigin::Smith, RevertOrigin::Unknown] {
            let report = RecoveryReport::RevertConfirmation(RevertPreview {
                scope: "path#1".to_owned(),
                fingerprint: "exact-preview".to_owned(),
                origin,
                patch: patch.clone(),
            });
            assert_eq!(
                render_plain(&report),
                format!("origin: {}\n\n{source}", origin.label())
            );
        }
    }
}
