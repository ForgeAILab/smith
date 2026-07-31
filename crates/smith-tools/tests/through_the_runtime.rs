//! The tools driven the way a model drives them: through the runtime.
//!
//! The unit tests invoke each tool directly, which proves its behavior but not
//! that it is reachable. These tests script a provider into requesting a tool
//! call and assert on what actually happens — including that a mutating tool
//! stops at the approval gate and a read-only one does not.

use std::sync::Arc;
use std::time::Duration;

use agent_runtime::prelude::*;
use agent_runtime::provider::fake::{ScriptedStream, tool_call_fragments, usage_event};
use agent_runtime::runtime::{RuntimeBuilder, StartSession};
use agent_runtime_core::approval::{AllowAll, DenyAll};
use agent_runtime_core::content::ContentPart;
use agent_runtime_core::error::ErrorKind;
use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
use agent_runtime_testkit::scenarios::fake_model_profile;
use smith_host::approval::{InteractiveApproval, PromptScope};
use smith_host::workspace::ProjectWorkspace;

/// A provider that requests one tool call, then replies with text.
fn provider(tool: &str, arguments: &str) -> FakeProvider {
    let mut first = tool_call_fragments(0, "call-1", tool, arguments);
    first.push(usage_event(10, 4));
    first.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });

    FakeProvider::new(
        "fake",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(first),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "done".into(),
                },
                usage_event(12, 2),
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    )
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/retry.rs"),
        "pub fn retry() {\n    backoff();\n}\n",
    )
    .unwrap();
    dir
}

/// Composes every runtime in this file.
///
/// The model profile comes from the shared testkit rather than a local
/// literal, so Smith's fixtures cannot drift from the limits, context policy,
/// or revision identity that the upstream Smith conformance fixture plans
/// against.
fn build(
    root: &std::path::Path,
    provider: Arc<FakeProvider>,
    approval: Arc<dyn ApprovalPolicy>,
) -> Runtime {
    RuntimeBuilder::new(ModelId::new("fake"))
        .model_profile(fake_model_profile())
        .provider(provider)
        .legacy_approval_authority()
        .approval(approval)
        .workspace(Arc::new(ProjectWorkspace::new(root).expect("a workspace")))
        .tools(smith_tools::all())
        .build()
        .expect("a runtime")
}

/// The conversation history after the turn, as rendered tool-result text.
fn tool_results(session: &SessionHandle) -> Vec<String> {
    session
        .history()
        .iter()
        .flat_map(|message| message.content.clone())
        .filter_map(|part| match part {
            ContentPart::ToolResult(result) => Some(
                result
                    .content
                    .iter()
                    .filter_map(|part| part.as_text())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_read_only_tool_runs_without_asking_for_approval() {
    let dir = project();
    let runtime = build(
        dir.path(),
        Arc::new(provider("read", r#"{"path":"src/retry.rs"}"#)),
        // Fail-closed: if `read` were treated as mutating, this would deny it.
        Arc::new(DenyAll),
    );
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");

    session.run(UserInput::text("read the retry policy")).await;

    let results = tool_results(&session);
    assert_eq!(results.len(), 1, "expected one tool result");
    assert!(
        results[0].contains("backoff();"),
        "the file content is missing: {}",
        results[0]
    );
}

#[tokio::test]
async fn a_mutating_tool_is_blocked_when_approval_denies() {
    let dir = project();
    let runtime = build(
        dir.path(),
        Arc::new(provider(
            "edit",
            r#"{"path":"src/retry.rs","old_string":"backoff();","new_string":"give_up();"}"#,
        )),
        Arc::new(DenyAll),
    );
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");

    session.run(UserInput::text("break the retry")).await;

    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/retry.rs")).unwrap(),
        "pub fn retry() {\n    backoff();\n}\n",
        "a denied edit must leave the file untouched"
    );
    let results = tool_results(&session);
    assert!(
        results
            .iter()
            .any(|text| text.to_lowercase().contains("den")),
        "the model must be told it was denied: {results:?}"
    );
}

#[tokio::test]
async fn a_mutating_tool_runs_once_the_user_allows_it() {
    let dir = project();
    let runtime = build(
        dir.path(),
        Arc::new(provider(
            "edit",
            r#"{"path":"src/retry.rs","old_string":"backoff();","new_string":"backoff_jittered();"}"#,
        )),
        Arc::new(AllowAll),
    );
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");

    session.run(UserInput::text("add jitter")).await;

    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/retry.rs")).unwrap(),
        "pub fn retry() {\n    backoff_jittered();\n}\n"
    );
}

#[tokio::test]
async fn the_interactive_gate_carries_the_users_answer_to_the_tool() {
    let dir = project();
    let (approval, mut requests) = InteractiveApproval::new(4);

    // Stand in for the TUI: answer the prompt the way a user pressing `y` would.
    let surface = tokio::spawn(async move {
        let prompt = tokio::time::timeout(Duration::from_secs(5), requests.recv())
            .await
            .expect("a prompt arrived")
            .expect("a prompt");
        assert_eq!(prompt.tool(), "edit");
        assert_eq!(prompt.request.arguments["path"], "src/retry.rs");
        prompt.allow(PromptScope::Once);
    });

    let runtime = build(
        dir.path(),
        Arc::new(provider(
            "edit",
            r#"{"path":"src/retry.rs","old_string":"backoff();","new_string":"retry_now();"}"#,
        )),
        Arc::new(approval),
    );
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");

    session.run(UserInput::text("rename the call")).await;
    surface.await.unwrap();

    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/retry.rs")).unwrap(),
        "pub fn retry() {\n    retry_now();\n}\n"
    );
}

#[tokio::test]
async fn a_path_outside_the_project_fails_even_when_approved() {
    let dir = project();
    let runtime = build(
        dir.path(),
        Arc::new(provider("read", r#"{"path":"../../../../etc/passwd"}"#)),
        // Approval is not the boundary; the workspace is. Allowing everything
        // must still not let a tool read outside the project.
        Arc::new(AllowAll),
    );
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");

    session.run(UserInput::text("read the password file")).await;

    let results = tool_results(&session);
    assert!(
        results
            .iter()
            .any(|text| text.contains("outside the project")),
        "the escape must be refused: {results:?}"
    );
    assert!(
        !results.iter().any(|text| text.contains("root:")),
        "no content from outside the project may be returned"
    );
}

#[tokio::test]
async fn every_built_in_tool_is_advertised_to_the_model() {
    let dir = project();
    let fake = Arc::new(FakeProvider::text_reply("hi"));
    let runtime = build(dir.path(), fake.clone(), Arc::new(AllowAll));
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");

    session.run(UserInput::text("hello")).await;

    let request = fake.requests().pop().expect("a provider request");
    let advertised: Vec<&str> = request.tools.iter().map(|t| t.name.as_str()).collect();
    for expected in ["read", "list", "search", "edit", "shell"] {
        assert!(
            advertised.contains(&expected),
            "`{expected}` was not advertised: {advertised:?}"
        );
    }
}

#[tokio::test]
async fn a_runtime_without_model_limits_is_refused_before_the_provider_is_reached() {
    let dir = project();
    let fake = Arc::new(FakeProvider::text_reply(
        "this reply must never be requested",
    ));

    // The one runtime here that deliberately bypasses `build`: omitting the
    // profile is the thing under test, so it cannot come from the fixture.
    let err = RuntimeBuilder::new(ModelId::new("fake"))
        .provider(fake.clone())
        .legacy_approval_authority()
        .approval(Arc::new(AllowAll))
        .workspace(Arc::new(
            ProjectWorkspace::new(dir.path()).expect("a workspace"),
        ))
        .tools(smith_tools::all())
        .build()
        .expect_err("a runtime that cannot enforce a context budget must not build");

    assert_eq!(err.kind, ErrorKind::Config, "{err:?}");
    // The fake records every request it is handed, so an empty log is proof
    // that the failure happened before any provider I/O — a startup error must
    // never cost a request or leave a half-configured session behind.
    assert!(
        fake.requests().is_empty(),
        "the provider was called despite an unbuildable runtime"
    );
}
