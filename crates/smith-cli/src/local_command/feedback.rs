//! Store-to-client conversion and explicitly confirmed browser hand-off.
use super::CommandReport;
use smith_client::commands::FeedbackAction;
use smith_client::feedback_report::FeedbackReport;
use smith_client::local_result::LocalResult;
use smith_runtime::host::HostSession;

fn show(report: FeedbackReport) -> CommandReport {
    CommandReport::Show(LocalResult::Feedback(Box::new(report)))
}

pub(super) fn command(host: &HostSession, action: FeedbackAction) -> CommandReport {
    #[cfg(not(feature = "module-harness-feedback"))]
    {
        let _ = (host, action);
        show(FeedbackReport::Error(
            "harness feedback is not compiled into this build".into(),
        ))
    }
    #[cfg(feature = "module-harness-feedback")]
    {
        let store =
            smith_module_harness_feedback::FeedbackStore::open(&host.runtime().policy().user_dir);
        execute(&store, action, crate::browser::open_browser)
            .unwrap_or_else(|error| show(FeedbackReport::Error(error.to_string())))
    }
}

#[cfg(feature = "module-harness-feedback")]
fn execute(
    store: &smith_module_harness_feedback::FeedbackStore,
    action: FeedbackAction,
    open: impl FnOnce(&str) -> bool,
) -> std::io::Result<CommandReport> {
    use smith_client::feedback_report::FeedbackRow;
    use smith_module_harness_feedback::{Status, github_issue};
    Ok(match action {
        FeedbackAction::Show(None) => {
            let mut listing = store.list()?;
            listing
                .reports
                .sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.id.cmp(&a.id)));
            if !listing.reports.is_empty() {
                store.mark_viewed(
                    &listing
                        .reports
                        .iter()
                        .map(|r| r.id.clone())
                        .collect::<Vec<_>>(),
                )?;
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            let rows = listing
                .reports
                .iter()
                .map(|r| FeedbackRow {
                    id: r.id.clone(),
                    age: age(now.saturating_sub(r.created)),
                    status: word(&r.status),
                    kind: word(&r.input.kind),
                    area: r.input.area.clone(),
                    title: r.input.title.clone(),
                    count: r.count,
                })
                .collect::<Vec<_>>();
            show(if rows.is_empty() && listing.malformed.is_empty() {
                FeedbackReport::Empty
            } else {
                FeedbackReport::List {
                    rows,
                    malformed: listing.malformed,
                }
            })
        }
        FeedbackAction::Show(Some(id)) => {
            let report = store.show(&id)?;
            let content = format!(
                "created: {}\nupdated: {}\nstatus: {}\nviewed: {}\nkind: {}\narea: {}\ntitle: {}\ncount: {}\n\nWhat happened\n{}\n\nUser said\n{}\n\nRepro\n{}\n\nEvidence (local, untrusted report text)\n{}",
                report.created,
                report.updated,
                word(&report.status),
                report.viewed,
                word(&report.input.kind),
                report.input.area,
                report.input.title,
                report.count,
                report.input.what_happened,
                report.input.user_said,
                report.input.repro,
                serde_json::to_string_pretty(&report.evidence)?
            );
            CommandReport::FeedbackDetail {
                id: id.clone(),
                report: FeedbackReport::Detail { id, content },
            }
        }
        FeedbackAction::Preview(id) => {
            let preview = github_issue(&store.show(&id)?);
            CommandReport::FeedbackPreview {
                id,
                url: preview.url,
                title: preview.title,
                body: preview.body,
            }
        }
        FeedbackAction::Dismiss(id) => {
            store.set_status(&id, Status::Dismissed)?;
            show(FeedbackReport::Outcome(format!("{id} dismissed.")))
        }
        FeedbackAction::Open { id, url } => {
            // The action carries the URL whose exact title and body were reviewed.
            store.show(&id)?;
            if open(&url) {
                store.set_status(&id, Status::Filed)?;
                show(FeedbackReport::Outcome(format!(
                    "{id} handed off to GitHub; submit the issue in your browser."
                )))
            } else {
                show(FeedbackReport::Outcome(format!(
                    "Browser hand-off failed; report unchanged. Copy this URL:\n{url}"
                )))
            }
        }
    })
}

#[cfg(feature = "module-harness-feedback")]
fn word(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}
#[cfg(feature = "module-harness-feedback")]
fn age(ms: i64) -> String {
    let seconds = ms.max(0) / 1000;
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86400)
    }
}

pub(crate) fn startup_notice(host: &HostSession) -> Option<String> {
    // A new session can create another App in the same launch.
    static CHECKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if CHECKED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    #[cfg(not(feature = "module-harness-feedback"))]
    {
        let _ = host;
        None
    }
    #[cfg(feature = "module-harness-feedback")]
    {
        notice_for(
            &smith_module_harness_feedback::FeedbackStore::open(&host.runtime().policy().user_dir),
            host.runtime().module_report().iter().any(|r| {
                r.descriptor.id == "harness-feedback"
                    && matches!(r.current_state(), smith_module::ModuleState::Mounted)
            }),
        )
    }
}

#[cfg(feature = "module-harness-feedback")]
fn notice_for(
    store: &smith_module_harness_feedback::FeedbackStore,
    mounted: bool,
) -> Option<String> {
    if !mounted {
        return None;
    }
    let listing = store.list().ok()?;
    let count = listing
        .reports
        .iter()
        .filter(|r| !r.viewed && r.status == smith_module_harness_feedback::Status::Queued)
        .count();
    (count > 0).then(|| {
        format!(
            "{count} harness feedback report{} queued — /feedback",
            if count == 1 { "" } else { "s" }
        )
    })
}

#[cfg(all(test, feature = "module-harness-feedback"))]
mod tests {
    use super::*;
    use smith_module_harness_feedback::{Evidence, FeedbackStore, Kind, ReportInput, Status};
    fn seed(store: &FeedbackStore) -> String {
        store
            .record(
                ReportInput {
                    kind: Kind::Bug,
                    area: "tools".into(),
                    title: "Edit failed".into(),
                    what_happened: "Failed edit".into(),
                    user_said: "".into(),
                    repro: "Retry".into(),
                },
                Evidence {
                    identity: Default::default(),
                    os: "test".into(),
                    session: "session".into(),
                    turn: None,
                    call: "call".into(),
                    child: None,
                    workspace: "/private/work".into(),
                    failures: vec![],
                    coverage: "Smith".into(),
                },
                None,
            )
            .unwrap()
            .id
    }
    #[test]
    fn feedback_notice_respects_mount_viewed_and_status() {
        let dir = tempfile::tempdir().unwrap();
        let store = FeedbackStore::open(dir.path());
        let id = seed(&store);
        assert_eq!(
            notice_for(&store, true).as_deref(),
            Some("1 harness feedback report queued — /feedback")
        );
        assert_eq!(notice_for(&store, false), None);
        store.set_status(&id, Status::Dismissed).unwrap();
        assert_eq!(notice_for(&store, true), None);
        store.set_status(&id, Status::Queued).unwrap();
        // Reviewing the store remains available independently of mounting.
        execute(&store, FeedbackAction::Show(None), |_| false).unwrap();
        assert_eq!(notice_for(&store, true), None);
        assert!(store.show(&id).unwrap().viewed);
    }
    #[test]
    fn feedback_listing_marks_viewed_and_names_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let store = FeedbackStore::open(dir.path());
        let id = seed(&store);
        std::fs::write(dir.path().join("feedback/broken.md"), "bad").unwrap();
        let result = execute(&store, FeedbackAction::Show(None), |_| {
            panic!("listing opened browser")
        })
        .unwrap();
        let CommandReport::Show(LocalResult::Feedback(report)) = result else {
            panic!("not a local result")
        };
        let FeedbackReport::List { rows, malformed } = *report else {
            panic!("not a list")
        };
        assert_eq!(rows[0].id, id);
        assert_eq!(malformed, ["broken.md"]);
        assert!(store.show(&id).unwrap().viewed);
    }
    #[test]
    fn feedback_preview_and_handoff_preserve_review_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let store = FeedbackStore::open(dir.path());
        let id = seed(&store);
        let CommandReport::FeedbackPreview {
            title, body, url, ..
        } = execute(&store, FeedbackAction::Preview(id.clone()), |_| {
            panic!("preview opened browser")
        })
        .unwrap()
        else {
            panic!("not a preview")
        };
        let exact = smith_module_harness_feedback::github_issue(&store.show(&id).unwrap());
        assert_eq!(
            (title, body, url.clone()),
            (exact.title, exact.body, exact.url)
        );
        assert_eq!(store.show(&id).unwrap().status, Status::Queued);
        let result = execute(
            &store,
            FeedbackAction::Open {
                id: id.clone(),
                url: url.clone(),
            },
            |_| false,
        )
        .unwrap();
        let CommandReport::Show(LocalResult::Feedback(report)) = result else {
            panic!("not a result")
        };
        assert!(smith_client::feedback_report::render_plain(&report).contains(&url));
        assert_eq!(store.show(&id).unwrap().status, Status::Queued);
        execute(
            &store,
            FeedbackAction::Open {
                id: id.clone(),
                url: url.clone(),
            },
            |opened| {
                assert_eq!(opened, url);
                true
            },
        )
        .unwrap();
        assert_eq!(store.show(&id).unwrap().status, Status::Filed);
    }
    #[test]
    fn feedback_dismiss_persists_locally() {
        let dir = tempfile::tempdir().unwrap();
        let store = FeedbackStore::open(dir.path());
        let id = seed(&store);
        execute(&store, FeedbackAction::Dismiss(id.clone()), |_| {
            panic!("dismiss opened browser")
        })
        .unwrap();
        assert_eq!(store.show(&id).unwrap().status, Status::Dismissed);
    }
}
