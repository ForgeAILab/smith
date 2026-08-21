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
use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::content::ContentPart;
use agent_runtime_core::error::ErrorKind;
use agent_runtime_core::grant::{
    GrantConstraints, SecurityCheck, SecurityCheckId, SecurityCheckMode, SecurityCheckOutcome,
    SecurityCheckRevision,
};
use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
use agent_runtime_core::security::{AuthorizationRequest, PermissionSet, SecurityResource};
use agent_runtime_core::workspace::Workspace;
use agent_runtime_registry::Permission;
use agent_runtime_testkit::scenarios::fake_model_profile;
use async_trait::async_trait;
use smith_host::approval::{HeadlessApproval, InteractiveApproval, PromptScope};
use smith_host::workspace::ProjectWorkspace;

#[derive(Debug)]
struct ProjectToolAuthority {
    id: SecurityCheckId,
    revision: SecurityCheckRevision,
    mount: String,
    coverage: PermissionSet,
}

impl ProjectToolAuthority {
    fn new(mount: impl Into<String>) -> Self {
        Self {
            id: SecurityCheckId::new("smith-tools-integration-authority"),
            revision: SecurityCheckRevision::new("v1"),
            mount: mount.into(),
            coverage: [
                Permission::FsRead,
                Permission::FsWrite,
                Permission::FsCreate,
                Permission::FsDelete,
                Permission::HostFsRead,
                Permission::HostFsWrite,
                Permission::ProcessSpawn,
                Permission::NetHttp,
                Permission::DataEgress,
            ]
            .into_iter()
            .collect(),
        }
    }
}

#[async_trait]
impl SecurityCheck for ProjectToolAuthority {
    fn id(&self) -> &SecurityCheckId {
        &self.id
    }

    fn revision(&self) -> &SecurityCheckRevision {
        &self.revision
    }

    fn declared_coverage(&self) -> Option<PermissionSet> {
        Some(self.coverage.clone())
    }

    async fn evaluate(
        &self,
        request: &AuthorizationRequest,
        _cancel: &Cancellation,
    ) -> SecurityCheckOutcome {
        let filesystem_authority = request.requested.iter().any(|permission| {
            matches!(
                permission,
                Permission::FsRead
                    | Permission::FsWrite
                    | Permission::FsCreate
                    | Permission::FsDelete
            )
        });
        if filesystem_authority {
            match &request.resource {
                SecurityResource::Filesystem { mount, .. } => {
                    if mount != &self.mount {
                        // Outside the project: mirrors Smith's authority —
                        // never unattended, the approval policy decides.
                        return SecurityCheckOutcome::RequireApproval {
                            constraints: GrantConstraints::unconstrained(),
                        };
                    }
                }
                _ => {
                    return SecurityCheckOutcome::Deny {
                        code: agent_runtime_core::grant::DecisionCode::other(
                            "test.workspace_resource_mismatch",
                        ),
                    };
                }
            }
        }

        let host_filesystem_authority = request.requested.iter().any(|permission| {
            matches!(permission, Permission::HostFsRead | Permission::HostFsWrite)
        });
        if host_filesystem_authority {
            return if matches!(
                &request.resource,
                SecurityResource::Other { kind, .. }
                    if kind == smith_tools::HOST_SHELL_RESOURCE_KIND
            ) {
                SecurityCheckOutcome::RequireApproval {
                    constraints: GrantConstraints::unconstrained(),
                }
            } else {
                SecurityCheckOutcome::Deny {
                    code: agent_runtime_core::grant::DecisionCode::other(
                        "test.host_resource_mismatch",
                    ),
                }
            };
        }

        if request.requested.len() == 1 && request.requested.contains(&Permission::FsRead) {
            SecurityCheckOutcome::Allow {
                constraints: GrantConstraints::unconstrained(),
            }
        } else {
            SecurityCheckOutcome::RequireApproval {
                constraints: GrantConstraints::unconstrained(),
            }
        }
    }
}

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
    let workspace = Arc::new(ProjectWorkspace::new(root).expect("a workspace"));
    let authority = Arc::new(ProjectToolAuthority::new(workspace.root()));
    let coverage = authority.coverage.clone();
    RuntimeBuilder::new(ModelId::new("fake"))
        .model_profile(fake_model_profile())
        .provider(provider)
        .security_check(
            authority,
            SecurityCheckMode::Authoritative,
            coverage,
            agent_runtime_core::check_set::ActionClass::new("smith-built-in-tools"),
        )
        .approval(approval)
        .workspace(workspace)
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

    session
        .run(UserInput::text("read the retry policy"))
        .await
        .expect("the turn runs");

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

    session
        .run(UserInput::text("break the retry"))
        .await
        .expect("the turn runs");

    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/retry.rs")).unwrap(),
        "pub fn retry() {\n    backoff();\n}\n",
        "a denied edit must leave the file untouched"
    );
    let results = tool_results(&session);
    assert!(
        results.iter().any(|text| {
            let text = text.to_lowercase();
            text.contains("approval") && text.contains("declined")
        }),
        "the model must receive the runtime's approval denial: {results:?}"
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

    session
        .run(UserInput::text("add jitter"))
        .await
        .expect("the turn runs");

    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/retry.rs")).unwrap(),
        "pub fn retry() {\n    backoff_jittered();\n}\n"
    );
}

#[tokio::test]
async fn a_create_invocation_runs_through_preparation_authorization_and_execution() {
    let dir = project();
    let runtime = build(
        dir.path(),
        Arc::new(provider(
            "edit",
            r#"{"path":"src/generated.rs","old_string":"","new_string":"pub fn generated() {}\\n"}"#,
        )),
        Arc::new(AllowAll),
    );
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");

    session
        .run(UserInput::text("create the generated module"))
        .await
        .expect("the turn runs");

    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/generated.rs")).unwrap(),
        "pub fn generated() {}\\n"
    );
}

#[tokio::test]
async fn the_interactive_gate_carries_the_users_answer_to_the_tool() {
    let dir = project();
    let (approval, mut requests) = InteractiveApproval::new(4);
    let root = std::fs::canonicalize(dir.path()).expect("a canonical project root");
    let expected_path = root.join("src/retry.rs").to_string_lossy().into_owned();
    let expected_resource = SecurityResource::filesystem(
        root.to_string_lossy(),
        vec!["src".into(), "retry.rs".into()],
    );

    // Stand in for the TUI: answer the prompt the way a user pressing `y` would.
    let surface = tokio::spawn(async move {
        let prompt = tokio::time::timeout(Duration::from_secs(5), requests.recv())
            .await
            .expect("a prompt arrived")
            .expect("a prompt");
        assert_eq!(prompt.tool(), "edit");
        assert_eq!(prompt.prepared().arguments()["path"], expected_path);
        assert_eq!(prompt.prepared().resource(), &expected_resource);
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

    session
        .run(UserInput::text("rename the call"))
        .await
        .expect("the turn runs");
    surface.await.unwrap();

    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/retry.rs")).unwrap(),
        "pub fn retry() {\n    retry_now();\n}\n"
    );
}

#[tokio::test]
async fn shell_reaches_approval_with_host_authority_before_execution() {
    let dir = project();
    let (approval, mut requests) = InteractiveApproval::new(4);
    let surface = tokio::spawn(async move {
        let prompt = tokio::time::timeout(Duration::from_secs(5), requests.recv())
            .await
            .expect("a shell prompt arrived")
            .expect("a shell prompt");
        assert_eq!(prompt.tool(), "shell");
        assert!(matches!(
            prompt.prepared().resource(),
            SecurityResource::Other { kind, id }
                if kind == smith_tools::HOST_SHELL_RESOURCE_KIND && id.starts_with("sha256:")
        ));
        assert_eq!(
            prompt.prepared().required_permissions(),
            &[
                Permission::HostFsRead,
                Permission::HostFsWrite,
                Permission::ProcessSpawn,
                Permission::NetHttp,
                Permission::DataEgress,
            ]
            .into_iter()
            .collect::<PermissionSet>()
        );
        assert_eq!(
            prompt
                .prepared()
                .effects()
                .mutation_scopes()
                .map(|scope| scope.as_str())
                .collect::<Vec<_>>(),
            ["host:filesystem"]
        );
        prompt.deny("shell authority was reviewed and declined");
    });
    let runtime = build(
        dir.path(),
        Arc::new(provider(
            "shell",
            r#"{"command":"printf unauthorized > shell-ran.txt","cwd":"src"}"#,
        )),
        Arc::new(approval),
    );
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");

    session
        .run(UserInput::text("run a command"))
        .await
        .expect("the turn runs");
    surface.await.expect("the approval surface completes");

    assert!(
        !dir.path().join("src/shell-ran.txt").exists(),
        "the shell ran before its broad authority was approved"
    );
}

#[tokio::test]
async fn headless_shell_refuses_before_spawning() {
    let dir = project();
    let approval = Arc::new(HeadlessApproval::new());
    let runtime = build(
        dir.path(),
        Arc::new(provider(
            "shell",
            r#"{"command":"printf unauthorized > shell-ran.txt","cwd":"src"}"#,
        )),
        approval.clone(),
    );
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");

    session
        .run(UserInput::text("run a command"))
        .await
        .expect("the denial is a tool outcome");

    assert_eq!(
        approval
            .required()
            .expect("host approval was required")
            .tool,
        "shell"
    );
    assert!(!dir.path().join("src/shell-ran.txt").exists());
}

#[tokio::test]
async fn explicit_allow_all_host_shell_can_read_outside_the_project() {
    let dir = project();
    let outside = tempfile::tempdir().expect("an outside dir");
    let secret = outside.path().join("host-secret.txt");
    std::fs::write(&secret, "host-shell-crossed-the-project-boundary\n").unwrap();
    let arguments = serde_json::json!({
        "command": format!("cat {}", secret.display()),
    })
    .to_string();
    let runtime = build(
        dir.path(),
        Arc::new(provider("shell", &arguments)),
        Arc::new(AllowAll),
    );
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");

    session
        .run(UserInput::text("read an explicitly approved host file"))
        .await
        .expect("the host shell runs");

    let results = tool_results(&session);
    assert!(
        results
            .iter()
            .any(|text| text.contains("host-shell-crossed-the-project-boundary")),
        "the approved host shell did not reach the host file: {results:?}"
    );
}

#[tokio::test]
async fn a_path_outside_the_project_is_blocked_when_approval_denies() {
    let dir = project();
    let outside = tempfile::tempdir().expect("an outside dir");
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, "outside-content\n").unwrap();

    let runtime = build(
        dir.path(),
        Arc::new(provider(
            "read",
            &format!(r#"{{"path":"{}"}}"#, secret.display()),
        )),
        // The capability-rooted filesystem refuses the path before approval;
        // a denying policy cannot weaken that boundary.
        Arc::new(DenyAll),
    );
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");

    session
        .run(UserInput::text("read the secret"))
        .await
        .expect("the turn runs");

    let results = tool_results(&session);
    assert!(
        !results.iter().any(|text| text.contains("outside-content")),
        "no content from outside the project may be returned without approval: {results:?}"
    );
}

#[tokio::test]
async fn approval_cannot_widen_a_read_beyond_the_project_capability() {
    let dir = project();
    let outside = tempfile::tempdir().expect("an outside dir");
    let notes = outside.path().join("notes.txt");
    std::fs::write(&notes, "carried across the boundary\n").unwrap();

    let runtime = build(
        dir.path(),
        Arc::new(provider(
            "read",
            &format!(r#"{{"path":"{}"}}"#, notes.display()),
        )),
        // Allow-all governs actions inside the granted capability. It cannot
        // turn an ambient absolute host path into a project file handle.
        Arc::new(AllowAll),
    );
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");

    session
        .run(UserInput::text("read the notes"))
        .await
        .expect("the turn runs");

    let results = tool_results(&session);
    assert!(
        !results
            .iter()
            .any(|text| text.contains("carried across the boundary")),
        "approval must not widen the project filesystem capability: {results:?}"
    );
    assert!(
        results
            .iter()
            .any(|text| text.contains("outside the project")),
        "the refusal should identify the capability boundary: {results:?}"
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

    session
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");

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
