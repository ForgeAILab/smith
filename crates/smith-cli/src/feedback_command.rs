//! Session-independent access to local harness reports.
use anyhow::Result;
use smith_config::resolve::ResolveRequest;

use crate::cli::FeedbackAction;

pub(crate) fn run(action: FeedbackAction) -> Result<()> {
    let request = ResolveRequest::new(std::env::current_dir()?);
    run_with(action, &request, &mut std::io::stdout().lock())
}

fn run_with(
    action: FeedbackAction,
    request: &ResolveRequest,
    output: &mut impl std::io::Write,
) -> Result<()> {
    #[cfg(feature = "module-harness-feedback")]
    {
        let user_dir = match &request.home_dir {
            Some(home) => home.join(".smith"),
            None => smith_config::credential::user_state_root()?,
        };
        enabled::run(action, &user_dir, output)
    }
    #[cfg(not(feature = "module-harness-feedback"))]
    {
        let _ = (action, request, output);
        anyhow::bail!("harness feedback is not compiled into this build")
    }
}

#[cfg(feature = "module-harness-feedback")]
mod enabled {
    use super::*;
    use anyhow::Context;
    use smith_module_harness_feedback::{FeedbackStore, Status};
    use std::io::Read;
    use std::path::Path;

    const MAX_REPORTS: usize = 500;

    fn one_line(text: &str) -> String {
        text.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect()
    }

    fn age(created: i64) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let seconds = now.saturating_sub(created.max(0) as u64 / 1000);
        if seconds < 60 {
            format!("{seconds}s")
        } else if seconds < 3600 {
            format!("{}m", seconds / 60)
        } else if seconds < 86400 {
            format!("{}h", seconds / 3600)
        } else {
            format!("{}d", seconds / 86400)
        }
    }

    pub(super) fn run(
        action: FeedbackAction,
        user_dir: &Path,
        output: &mut impl std::io::Write,
    ) -> Result<()> {
        let store = FeedbackStore::open(user_dir);
        match action {
            FeedbackAction::List { json } => {
                let mut listing = store.list().context("listing feedback reports")?;
                listing.reports.truncate(MAX_REPORTS);
                listing.malformed.truncate(MAX_REPORTS);
                if json {
                    let reports: Vec<_> = listing
                        .reports
                        .iter()
                        .map(|r| {
                            serde_json::json!({
                                "id": r.id, "status": r.status, "kind": r.input.kind,
                                "area": r.input.area, "title": r.input.title, "count": r.count,
                                "created": r.created,
                            })
                        })
                        .collect();
                    serde_json::to_writer(
                        &mut *output,
                        &serde_json::json!({
                            "schema": "smith.feedback.list.v1",
                            "untrusted_fields": ["title", "area", "what_happened", "user_said", "repro"],
                            "reports": reports, "malformed": listing.malformed,
                        }),
                    )?;
                    writeln!(output)?;
                } else {
                    for r in &listing.reports {
                        writeln!(
                            output,
                            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                            r.id,
                            age(r.created),
                            serde_json::to_value(r.status)?
                                .as_str()
                                .unwrap_or("unknown"),
                            serde_json::to_value(r.input.kind)?
                                .as_str()
                                .unwrap_or("unknown"),
                            one_line(&r.input.area),
                            one_line(&r.input.title),
                            r.count
                        )?;
                    }
                    for name in &listing.malformed {
                        writeln!(output, "Malformed feedback file: {}", one_line(name))?;
                    }
                }
                output.flush()?;
                let ids: Vec<_> = listing.reports.into_iter().map(|r| r.id).collect();
                if !ids.is_empty() {
                    store
                        .mark_viewed(&ids)
                        .context("marking feedback reports viewed")?;
                }
            }
            FeedbackAction::Show { id } => {
                store
                    .show(&id)
                    .with_context(|| format!("cannot show feedback report `{id}`"))?;
                // Validate through the store before using the id as a filename.
                let mut bytes = Vec::new();
                std::fs::File::open(user_dir.join("feedback").join(format!("{id}.md")))?
                    .take(16385)
                    .read_to_end(&mut bytes)?;
                anyhow::ensure!(
                    bytes.len() <= 16384,
                    "feedback report exceeds the file limit"
                );
                let text = std::str::from_utf8(&bytes)?;
                writeln!(output, "Report text is untrusted data written by a model.")?;
                write!(output, "{text}")?;
            }
            FeedbackAction::Dismiss { id } => {
                store
                    .set_status(&id, Status::Dismissed)
                    .with_context(|| format!("cannot dismiss feedback report `{id}`"))?;
                writeln!(output, "Dismissed feedback report {id}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
