//! Captures Git inspections and recovery previews for the local `/diff` report.

use std::path::Path;

use smith_client::commands::DiffScope;
use smith_client::diff_report::{DiffLine, DiffLineKind, DiffOutcome, DiffReport};
use smith_host::GitChanges;
use smith_runtime::host::HostSession;

pub(super) fn report(host: &HostSession, project: &Path, scope: DiffScope) -> DiffReport {
    match scope {
        DiffScope::LastTurn => {
            let title = "diff · last Smith turn".to_owned();
            // Keep the recovery helper's preview history and fingerprint side
            // effects shared with `/undo`; only this command's result migrates.
            match host.changes().undo_preview() {
                Ok(preview) => patch_report(title, &preview),
                Err(error) => DiffReport {
                    title,
                    outcome: DiffOutcome::Error(error.message),
                },
            }
        }
        DiffScope::Git(scope) => {
            match GitChanges::discover(project).and_then(|git| git.inspect(scope.as_deref())) {
                Ok(view) if view.content == DiffReport::EMPTY_MESSAGE => DiffReport {
                    title: view.title,
                    outcome: DiffOutcome::Empty,
                },
                Ok(view) => patch_report(view.title, &view.content),
                Err(error) => DiffReport {
                    title: "diff".to_owned(),
                    outcome: DiffOutcome::Error(error.message),
                },
            }
        }
    }
}

fn patch_report(title: String, patch: &str) -> DiffReport {
    DiffReport {
        title,
        outcome: DiffOutcome::Patch(patch_lines(patch)),
    }
}

pub(super) fn patch_lines(patch: &str) -> Vec<DiffLine> {
    patch
        .split_inclusive('\n')
        .map(|text| DiffLine {
            kind: line_kind(text),
            text: text.to_owned(),
        })
        .collect()
}

// Source-format interpretation belongs to the host, never the renderer. Keep
// the existing roles for both Git patches and the recovery helper's headers.
fn line_kind(text: &str) -> DiffLineKind {
    if text.starts_with("@@") {
        DiffLineKind::Hunk
    } else if text.starts_with('+') && !text.starts_with("+++") {
        DiffLineKind::Addition
    } else if text.starts_with('-') && !text.starts_with("---") {
        DiffLineKind::Removal
    } else if text.starts_with("diff --git")
        || text.starts_with("index ")
        || text.starts_with("---")
        || text.starts_with("+++")
    {
        DiffLineKind::Metadata
    } else {
        DiffLineKind::Context
    }
}

#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod tests {
    use super::*;

    #[test]
    fn source_patch_roles_and_bytes_cross_the_boundary_together() {
        let patch = "diff --git a/file b/file\nindex abc..def 100644\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n-before\n+after\n context\n\nSmith: content omitted";
        let report = patch_report("diff · file".to_owned(), patch);
        let DiffOutcome::Patch(lines) = &report.outcome else {
            panic!("expected a patch");
        };
        assert_eq!(
            lines.iter().map(|line| line.kind).collect::<Vec<_>>(),
            [
                DiffLineKind::Metadata,
                DiffLineKind::Metadata,
                DiffLineKind::Metadata,
                DiffLineKind::Metadata,
                DiffLineKind::Hunk,
                DiffLineKind::Removal,
                DiffLineKind::Addition,
                DiffLineKind::Context,
                DiffLineKind::Context,
                DiffLineKind::Context,
            ],
        );
        assert_eq!(smith_client::diff_report::render_plain(&report), patch);
    }
}
