use super::*;
fn input(title: &str) -> ReportInput {
    ReportInput {
        kind: Kind::Bug,
        area: "tools".into(),
        title: title.into(),
        what_happened: "failed".into(),
        user_said: String::new(),
        repro: String::new(),
    }
}
fn evidence(session: &str) -> Evidence {
    Evidence {
        identity: Default::default(),
        os: "test".into(),
        session: session.into(),
        turn: None,
        call: "call".into(),
        child: None,
        workspace: "/private/work".into(),
        failures: vec![],
        coverage: "Smith".into(),
    }
}
#[test]
fn duplicates_limits_and_viewed_status() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::open(dir.path());
    let first = store
        .record(input(" A   bug "), evidence("s"), None)
        .unwrap();
    let repeat = store.record(input("a bug"), evidence("s"), None).unwrap();
    assert_eq!(first.id, repeat.id);
    assert_eq!(repeat.count, 2);
    for n in 0..4 {
        store
            .record(input(&format!("bug{n}")), evidence("s"), None)
            .unwrap();
    }
    assert!(store.record(input("sixth"), evidence("s"), None).is_err());
    store.mark_viewed(std::slice::from_ref(&first.id)).unwrap();
    store.set_status(&first.id, Status::Dismissed).unwrap();
    let r = store.show(&first.id).unwrap();
    assert!(r.viewed);
    assert_eq!(r.status, Status::Dismissed);
}
#[test]
fn permissions_atomic_bounds_and_malformed() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::open(dir.path());
    let mut i = input(&"界".repeat(500));
    i.what_happened = "界".repeat(9000);
    let r = store.record(i.clone(), evidence("s"), None).unwrap();
    let repeated = store.record(i, evidence("s"), None).unwrap();
    assert_eq!(repeated.id, r.id);
    assert_eq!(repeated.count, 2);
    assert!(r.input.title.chars().count() <= 120);
    assert!(r.input.what_happened.contains("truncated"));
    let path = dir.path().join("feedback").join(format!("{}.md", r.id));
    assert!(std::fs::metadata(&path).unwrap().len() <= 16384);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    std::fs::write(dir.path().join("feedback/broken.md"), "broken").unwrap();
    assert_eq!(store.list().unwrap().malformed, vec!["broken.md"]);
    assert_eq!(
        std::fs::read_dir(dir.path().join("feedback"))
            .unwrap()
            .count(),
        3
    );
}
#[test]
fn concurrent_writers_preserve_counts() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::open(dir.path());
    std::thread::scope(|scope| {
        for _ in 0..2 {
            let s = store.clone();
            scope.spawn(move || {
                for _ in 0..20 {
                    s.record(input("same"), evidence("s"), None).unwrap();
                }
            });
        }
    });
    let reports = store.list().unwrap().reports;
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].count, 40);
}
#[test]
fn registered_redaction() {
    #[derive(Debug)]
    struct Redactor;
    impl smith_module::FeedbackRedactor for Redactor {
        fn redact_text(&self, s: &str) -> String {
            s.replace("secret", "[redacted]")
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::open(dir.path());
    let r = store
        .record(input("secret"), evidence("s"), Some(&Redactor))
        .unwrap();
    assert_eq!(r.input.title, "[redacted]");
}
#[test]
fn github_url_sections_and_bound() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::open(dir.path());
    let r = store.record(input("bug"), evidence("s"), None).unwrap();
    let preview = github_issue(&r);
    assert!(
        preview
            .url
            .starts_with("https://github.com/ForgeAILab/smith/issues/new?")
    );
    assert!(!preview.body.contains("/private/work"));
    assert!(preview.url.contains("labels=harness-feedback"));
    let mut r = r;
    r.input.what_happened = "界".repeat(2000);
    r.input.user_said = "界".repeat(2000);
    let preview = github_issue(&r);
    assert!(preview.url.len() <= 8000);
    assert!(preview.body.ends_with("for the full report)"));
}
#[test]
fn pruning_and_full_store_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::open(dir.path());
    for n in 0..500 {
        store
            .record(input("bug"), evidence(&format!("s{n}")), None)
            .unwrap();
    }
    assert!(
        store
            .record(input("full"), evidence("next"), None)
            .unwrap_err()
            .to_string()
            .contains("full")
    );
    let r = store.list().unwrap().reports.remove(0);
    let path = dir.path().join("feedback").join(format!("{}.md", r.id));
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\"status\":\"queued\"", "\"status\":\"dismissed\"");
    let text = text.replace(&format!("\"updated\":{}", r.updated), "\"updated\":0");
    std::fs::write(path, text).unwrap();
    store
        .record(input("pruned"), evidence("next"), None)
        .unwrap();
    assert_eq!(store.list().unwrap().reports.len(), 500);
    assert!(store.show(&r.id).is_err());
}

use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Deadline, SystemClock};
use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId};
use agent_runtime_core::tool::{InvocationContext, PreparationContext, Tool};
use agent_runtime_testkit::transport::ReplayTransport;
use smith_module::{ModuleContext, ModulePosture};
fn context() -> ModuleContext {
    ModuleContext {
        session_identity: Default::default(),
        feedback_redactor: None,
        recent_failures: None,
        settings: Default::default(),
        workspace_root: None,
        user_dir: "user".into(),
        is_child_session: false,
        posture: ModulePosture::ReadWrite,
        transport: Arc::new(ReplayTransport::single(Vec::new())),
        http: None,
        model: None,
        image_binding: None,
        vision: None,
        web: None,
        vision_unset: true,
        image_capable_profiles: Vec::new(),
        main_model_accepts_images: false,
        session_history: None,
        semantic_summary_enabled: false,
        max_input_tokens: 24_000,
        built_in_tools: true,
    }
}

fn preparation(root: &std::path::Path) -> PreparationContext {
    PreparationContext {
        session: SessionId::new("s"),
        turn: None,
        call_id: ToolCallId::new("c"),
        request: RequestId::new("r"),
        workspace: Arc::new(smith_host::ProjectWorkspace::new(root).unwrap()),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
    }
}

fn invocation(preparation: &PreparationContext) -> InvocationContext {
    InvocationContext {
        session: preparation.session.clone(),
        turn: preparation.turn.clone(),
        call_id: preparation.call_id.clone(),
        request: preparation.request.clone(),
        workspace: preparation.workspace.clone(),
        clock: preparation.clock.clone(),
        cancel: preparation.cancel.clone(),
        deadline: preparation.deadline,
        output_limit: 32_768,
    }
}

#[tokio::test]
async fn tool_uses_host_evidence_empty_effects_and_cli_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let mut context = context();
    context.user_dir = dir.path().into();
    context.is_child_session = true;
    context.session_identity.version = "host-version".into();
    context.session_identity.provider = "cli/example".into();
    let tool = crate::tool::FeedbackTool {
        context,
        store: FeedbackStore::open(dir.path()),
    };
    assert_eq!(
        tool.spec().effects,
        agent_runtime_core::tool::ToolEffects::default()
    );
    let ctx = preparation(dir.path());
    let prepared = tool.prepare(serde_json::json!({"kind":"bug","area":"tools","title":"A problem","what_happened":"Failed","version":"forged","session":"forged"}), &ctx).await.unwrap();
    let result = tool.invoke(prepared, &invocation(&ctx)).await.unwrap();
    assert!(!result.is_error);
    let r = FeedbackStore::open(dir.path())
        .list()
        .unwrap()
        .reports
        .remove(0);
    assert_eq!(r.evidence[0].session, "s");
    assert_eq!(r.evidence[0].identity.version, "host-version");
    assert_eq!(r.evidence[0].child.as_deref(), Some("s"));
    assert!(
        r.evidence[0]
            .coverage
            .contains("CLI's own tools are not covered")
    );
}

#[test]
fn labeled_sections_round_trip_even_when_text_contains_headings() {
    let dir = tempfile::tempdir().unwrap();
    let store = FeedbackStore::open(dir.path());
    let mut i = input("heading");
    i.what_happened = "a\n\n## Repro\n---\n\"quoted\"".into();
    let r = store.record(i.clone(), evidence("s"), None).unwrap();
    assert_eq!(
        store.show(&r.id).unwrap().input.what_happened,
        i.what_happened
    );
}
