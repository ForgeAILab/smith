use super::*;
use crate::testing::{call, context, text_of};
use serde_json::json;

/// A composed session: the real wrapper, the real recorders.
fn session() -> (tempfile::TempDir, Vec<Arc<dyn Tool>>, InvocationContext) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let ctx = context(dir.path());
    let tools = observe(
        Some(Arc::new(ChangeRecorder::new(None))),
        ReadRecorder::new(),
    );
    (dir, tools, ctx)
}

async fn read_fully(tools: &[Arc<dyn Tool>], ctx: &InvocationContext, path: &str) {
    call(tools, "read", json!({ "path": path }), ctx)
        .await
        .expect("the read succeeds");
}

#[tokio::test]
async fn overwriting_an_unread_file_is_refused() {
    let (dir, tools, ctx) = session();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").expect("seed");

    let error = call(
        &tools,
        "edit",
        json!({"path": "a.rs", "operation": "overwrite", "new_string": "gone\n"}),
        &ctx,
    )
    .await
    .expect_err("an unread file cannot be overwritten");

    assert!(error.to_string().contains("has not been read"), "{error}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.rs")).expect("still there"),
        "fn a() {}\n"
    );
}

#[tokio::test]
async fn a_partial_read_does_not_authorize_an_overwrite() {
    let (dir, tools, ctx) = session();
    let body: String = (1..=50).map(|n| format!("line {n}\n")).collect();
    std::fs::write(dir.path().join("long.txt"), &body).expect("seed");

    call(
        &tools,
        "read",
        json!({"path": "long.txt", "offset": 1, "limit": 5}),
        &ctx,
    )
    .await
    .expect("the partial read succeeds");

    let error = call(
        &tools,
        "edit",
        json!({"path": "long.txt", "operation": "overwrite", "new_string": "short\n"}),
        &ctx,
    )
    .await
    .expect_err("a window does not authorize replacing the whole file");

    assert!(error.to_string().contains("only read in part"), "{error}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("long.txt")).expect("still there"),
        body
    );
}

#[tokio::test]
async fn an_external_change_invalidates_the_read() {
    let (dir, tools, ctx) = session();
    let path = dir.path().join("a.rs");
    std::fs::write(&path, "fn a() {}\n").expect("seed");
    read_fully(&tools, &ctx, "a.rs").await;

    // The user edits the file in their own editor.
    std::fs::write(&path, "fn a() { work_in_progress(); }\n").expect("user edit");
    std::fs::File::options()
        .write(true)
        .open(&path)
        .and_then(|file| {
            file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(1))
        })
        .expect("retouch");

    let error = call(
        &tools,
        "edit",
        json!({"path": "a.rs", "operation": "overwrite", "new_string": "clobbered\n"}),
        &ctx,
    )
    .await
    .expect_err("a file changed since the read cannot be overwritten");

    assert!(
        error.to_string().contains("changed since it was read"),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("still there"),
        "fn a() { work_in_progress(); }\n",
        "the user's work must survive"
    );
}

#[tokio::test]
async fn an_edit_during_approval_is_rejected_at_commit() {
    let (dir, tools, ctx) = session();
    let path = dir.path().join("approval.rs");
    std::fs::write(&path, "fn approved() {}\n").expect("seed");
    read_fully(&tools, &ctx, "approval.rs").await;
    let edit = tools
        .iter()
        .find(|tool| tool.spec().name == "edit")
        .expect("edit tool");
    let prepared = edit
        .prepare(
            json!({
                "path": "approval.rs",
                "operation": "overwrite",
                "new_string": "clobbered\n"
            }),
            &crate::testing::preparation_context(&ctx),
        )
        .await
        .expect("prepared before approval");

    // Represents time spent at an approval prompt while the user edits in
    // another application.
    std::fs::write(&path, "user change during approval\n").expect("user edit");
    let error = edit
        .invoke(prepared, &ctx)
        .await
        .expect_err("commit must revalidate after approval");

    assert!(
        error.message.contains("changed since it was read"),
        "{error:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "user change during approval\n"
    );
}

#[tokio::test]
async fn a_full_read_authorizes_overwrite_and_delete() {
    let (dir, tools, ctx) = session();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").expect("seed");
    read_fully(&tools, &ctx, "a.rs").await;

    let outcome = call(
        &tools,
        "edit",
        json!({"path": "a.rs", "operation": "overwrite", "new_string": "fn b() {}\n"}),
        &ctx,
    )
    .await
    .expect("the overwrite runs");
    assert!(!outcome.is_error, "{}", text_of(&outcome));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.rs")).expect("rewritten"),
        "fn b() {}\n"
    );

    // The overwrite changed the file, so the earlier read no longer proves
    // anything about it — exactly the staleness rule, applied to ourselves.
    read_fully(&tools, &ctx, "a.rs").await;
    let outcome = call(
        &tools,
        "edit",
        json!({"path": "a.rs", "operation": "delete"}),
        &ctx,
    )
    .await
    .expect("the delete runs");
    assert!(!outcome.is_error, "{}", text_of(&outcome));
    assert!(!dir.path().join("a.rs").exists());
}

#[tokio::test]
async fn exact_replacement_needs_no_prior_read() {
    let (dir, tools, ctx) = session();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").expect("seed");

    let outcome = call(
        &tools,
        "edit",
        json!({"path": "a.rs", "old_string": "fn a", "new_string": "fn b"}),
        &ctx,
    )
    .await
    .expect("an exact replacement proves its own currency");
    assert!(!outcome.is_error, "{}", text_of(&outcome));
}

#[tokio::test]
async fn a_deleted_file_is_recorded_with_its_pre_image() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let ctx = context(dir.path());
    let recorder = Arc::new(ChangeRecorder::new(None));
    let tools = observe(Some(recorder.clone()), ReadRecorder::new());
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").expect("seed");

    recorder.start_turn();
    read_fully(&tools, &ctx, "a.rs").await;
    call(
        &tools,
        "edit",
        json!({"path": "a.rs", "operation": "delete"}),
        &ctx,
    )
    .await
    .expect("the delete runs");
    let set = recorder.finish_turn().expect("a change set");

    assert!(
        set.is_fully_attributable(),
        "a delete must be exactly attributed so it can be undone: {set:?}"
    );
    let [ToolMutation::Exact(edit)] = set.mutations.as_slice() else {
        panic!("expected one exact mutation: {set:?}");
    };
    assert_eq!(edit.before.as_deref(), Some(b"fn a() {}\n".as_slice()));
    assert_eq!(edit.after, None, "a removed file has no post-image");
}
