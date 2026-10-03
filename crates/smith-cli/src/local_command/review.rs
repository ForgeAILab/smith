//! Captures Git inspection data for a provider-backed read-only review.

use std::path::Path;

use smith_client::review_report::{ReviewPreview, ReviewReport};
use smith_host::GitChanges;

use super::diff::patch_lines;

pub(super) fn report(project: &Path, scope: Option<String>) -> ReviewReport {
    let scope = scope.unwrap_or_else(|| "all".to_owned());
    match GitChanges::discover(project).and_then(|git| git.inspect(Some(scope.as_str()))) {
        Ok(view) if view.content == ReviewReport::EMPTY_MESSAGE => ReviewReport::Empty,
        Ok(view) => ReviewReport::Confirmation(ReviewPreview {
            scope,
            title: view.title,
            patch: patch_lines(&view.content),
        }),
        Err(error) => ReviewReport::Error(error.message),
    }
}
