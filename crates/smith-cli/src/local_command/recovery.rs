//! Builds recovery confirmations and outcomes without terminal presentation.

use std::path::Path;

use smith_client::local_result::LocalResult;
use smith_client::recovery_report::{
    RecoveryAction, RecoveryApplied, RecoveryPreview, RecoveryReport, RevertOrigin, RevertPreview,
};
use smith_host::GitChanges;
use smith_runtime::host::HostSession;

use super::diff::patch_lines;

use super::CommandReport;

pub(super) fn undo_command(host: &HostSession) -> CommandReport {
    match undo_preview(host) {
        RecoveryReport::UndoConfirmation(preview) => CommandReport::UndoConfirmation(preview),
        report => CommandReport::Append(LocalResult::Recovery(Box::new(report))),
    }
}

pub(super) fn redo_command(host: &HostSession) -> CommandReport {
    match redo_preview(host) {
        RecoveryReport::RedoConfirmation(preview) => CommandReport::RedoConfirmation(preview),
        report => CommandReport::Show(LocalResult::Recovery(Box::new(report))),
    }
}

pub(super) fn revert_command(
    host: &HostSession,
    project: &Path,
    scope: Option<String>,
) -> CommandReport {
    match revert_preview(host, project, scope) {
        RecoveryReport::RevertConfirmation(preview) => CommandReport::RevertConfirmation(preview),
        report => CommandReport::Append(LocalResult::Recovery(Box::new(report))),
    }
}

pub(super) fn undo_preview(host: &HostSession) -> RecoveryReport {
    match host.changes().undo_preview() {
        Ok(preview) => RecoveryReport::UndoConfirmation(RecoveryPreview {
            patch: patch_lines(&preview),
        }),
        Err(error) => RecoveryReport::PreviewError {
            action: RecoveryAction::Undo,
            message: error.message,
        },
    }
}

pub(super) fn redo_preview(host: &HostSession) -> RecoveryReport {
    match host.changes().redo_preview() {
        Ok(preview) => RecoveryReport::RedoConfirmation(RecoveryPreview {
            patch: patch_lines(&preview),
        }),
        Err(error) => RecoveryReport::PreviewError {
            action: RecoveryAction::Redo,
            message: error.message,
        },
    }
}

pub(super) fn revert_preview(
    host: &HostSession,
    project: &Path,
    scope: Option<String>,
) -> RecoveryReport {
    let Some(scope) = scope else {
        return RecoveryReport::RevertUsage;
    };
    match GitChanges::discover(project).and_then(|git| git.preview_revert(&scope)) {
        Ok(preview) => {
            let path = scope.split('#').next().unwrap_or(scope.as_str());
            let origin = if let Ok(canonical) = project.join(path).canonicalize()
                && host.changes().latest_owns_path(&canonical)
            {
                RevertOrigin::Smith
            } else {
                RevertOrigin::Unknown
            };
            // GitChanges supplies its attribution header alongside the source
            // patch. Interpret that source format here, before the boundary.
            let header = format!("origin: {}\n\n", preview.origin);
            let patch = patch_lines(
                preview
                    .content
                    .strip_prefix(&header)
                    .unwrap_or(&preview.content),
            );
            host.changes()
                .record_revert_event(&preview.scope, &preview.fingerprint, "previewed");
            RecoveryReport::RevertConfirmation(RevertPreview {
                scope: preview.scope,
                fingerprint: preview.fingerprint,
                origin,
                patch,
            })
        }
        Err(error) => RecoveryReport::PreviewError {
            action: RecoveryAction::Revert,
            message: error.message,
        },
    }
}

pub(crate) fn undo(host: &HostSession) -> RecoveryReport {
    match host.changes().undo_latest() {
        // Accurate for a mixed turn: the confirmed preview names the deltas
        // this operation leaves untouched.
        Ok(()) => RecoveryReport::Applied(RecoveryApplied::Undo),
        Err(error) => RecoveryReport::ApplyError {
            action: RecoveryAction::Undo,
            message: error.message,
        },
    }
}

pub(crate) fn redo(host: &HostSession) -> RecoveryReport {
    match host.changes().redo_latest() {
        Ok(()) => RecoveryReport::Applied(RecoveryApplied::Redo),
        Err(error) => RecoveryReport::ApplyError {
            action: RecoveryAction::Redo,
            message: error.message,
        },
    }
}

pub(crate) fn revert(
    host: &HostSession,
    project: &Path,
    scope: String,
    fingerprint: &str,
) -> RecoveryReport {
    let recovery_dir = host.paths().map(|paths| {
        paths
            .directory()
            .join("recovery")
            .join(host.session().id().as_str())
    });
    match GitChanges::discover(project)
        .and_then(|git| git.apply_revert(&scope, fingerprint, recovery_dir.as_deref()))
    {
        Ok(applied) => {
            host.changes()
                .record_revert_event(&scope, fingerprint, "applied");
            host.changes().record_recovery(
                applied.path,
                applied.before,
                applied.after,
                "revert",
                applied.recovery_path,
            );
            RecoveryReport::Applied(RecoveryApplied::Revert { scope })
        }
        Err(error) => {
            host.changes()
                .record_revert_event(&scope, fingerprint, "failed");
            RecoveryReport::ApplyError {
                action: RecoveryAction::Revert,
                message: error.message,
            }
        }
    }
}
