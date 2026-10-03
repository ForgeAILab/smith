use super::*;
use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
use agent_runtime_core::workspace::Workspace;

const COMMAND: &str = "echo shortcut";
const TIMEOUT_MS: u64 = 10_000;

struct Fixture {
    _home: tempfile::TempDir,
    project: tempfile::TempDir,
    host: HostSession,
    provider: Arc<FakeProvider>,
    policy: Arc<InteractiveApproval>,
    requests: ApprovalRequests,
}

impl Fixture {
    async fn new(mode: ApprovalMode, persistent: bool) -> Self {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        std::fs::create_dir_all(project.path().join(".smith")).expect("config directory");
        std::fs::write(
            project.path().join(".smith/config.toml"),
            LOCAL_COMMAND_CONFIG,
        )
        .expect("config");
        let mut config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolution")
            .config;
        config.approval.mode.value = mode;
        config.persistence.enabled.value = persistent;
        let arguments = serde_json::json!({
            "command": COMMAND,
            "cwd": ".",
            "timeout_ms": TIMEOUT_MS,
        });
        let mut call = tool_call_fragments(0, "model-shell", "shell", &arguments.to_string());
        call.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        let provider = Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![
                ScriptedStream::new(call),
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "done".to_owned(),
                    },
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
            ],
        ));
        let (policy, requests) = InteractiveApproval::new(8);
        let policy = Arc::new(policy);
        let runtime = RuntimeRequest {
            provider: Some(provider.clone()),
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("workspace"),
            )),
            approval: (mode == ApprovalMode::Ask)
                .then(|| policy.clone() as Arc<dyn agent_runtime_core::approval::ApprovalPolicy>),
            ..RuntimeRequest::new(config, HostSurface::Terminal)
        };
        let host = Box::pin(smith_runtime::host::start(
            HostSessionRequest::new(runtime, project.path())
                .checkpoint_keys(Arc::new(TestCheckpointKeys)),
        ))
        .await
        .expect("host");
        Self {
            _home: home,
            project,
            host,
            provider,
            policy,
            requests,
        }
    }

    fn app(&self) -> App {
        App::new("example-model", self.project.path().display().to_string())
    }

    async fn start(
        &self,
        approvals: &LocalShellApprovals,
    ) -> tokio::sync::mpsc::UnboundedReceiver<LocalOutcome> {
        let (outcomes, receiver) = tokio::sync::mpsc::unbounded_channel();
        let mut app = self.app();
        app.composer.replace(format!("!{COMMAND}"));
        let Some(Action::RunShell { command }) =
            app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        else {
            panic!("expected a local shell action");
        };
        assert_eq!(command, COMMAND);
        let _ = start_local_shell(
            app.transcript.latest_shell_echo().expect("shell echo"),
            self.host.session().clone(),
            command,
            TIMEOUT_MS,
            approvals.clone(),
            outcomes,
        )
        .await;
        receiver
    }

    async fn prompt(&mut self) -> ApprovalPrompt {
        tokio::time::timeout(Duration::from_secs(10), self.requests.recv())
            .await
            .expect("approval arrived")
            .expect("approval channel open")
    }

    async fn model_asks(&mut self, approvals: &LocalShellApprovals) {
        let turn = self
            .host
            .session()
            .send(UserInput::text("Run echo shortcut in the shell"))
            .expect("model turn");
        let prompt = self.prompt().await;
        assert_eq!(prompt.prepared().arguments()["command"], COMMAND);
        assert_eq!(prompt.origin().turn(), Some(turn.id()));
        let prompt = approvals.resolve(prompt).expect("model call still asks");
        let mut app = self.app();
        app.present_approval(prompt);
        assert_eq!(app.pending_approval_count(), 1);
        drop(app);
        tokio::time::timeout(Duration::from_secs(10), turn.completed())
            .await
            .expect("model turn completed");
        assert!(
            !self
                .policy
                .is_session_allowed(self.host.session().id(), "shell")
        );
    }
}

async fn outcome(
    receiver: &mut tokio::sync::mpsc::UnboundedReceiver<LocalOutcome>,
) -> LocalOutcome {
    tokio::time::timeout(Duration::from_secs(10), receiver.recv())
        .await
        .expect("local action completed")
        .expect("local outcome")
}

fn assert_success(result: LocalOutcome) {
    match result {
        LocalOutcome::Shell {
            content, is_error, ..
        } => {
            assert!(!is_error, "{content}");
            assert!(content.contains("shortcut"), "{content}");
        }
        LocalOutcome::Error(error) => panic!("local shell failed: {error}"),
        LocalOutcome::Notice { .. } | LocalOutcome::Agent(_) | LocalOutcome::Review(_) => {
            panic!("expected a shell result")
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shortcut_under_ask_runs_without_presenting_approval() {
    // Ephemeral execution can publish its approval in the initial poll,
    // racing the receiving task. The identity must already be bound when
    // that task resolves the prompt.
    let mut fixture = Fixture::new(ApprovalMode::Ask, false).await;
    let approvals = LocalShellApprovals::default();
    let mut app = fixture.app();
    let mut receiver = fixture.start(&approvals).await;
    let prompt = fixture.prompt().await;
    assert_eq!(prompt.prepared().arguments()["command"], COMMAND);
    assert_eq!(prompt.prepared().arguments()["timeout_ms"], TIMEOUT_MS);
    let workspace = ProjectWorkspace::new(fixture.project.path()).expect("workspace");
    assert_eq!(prompt.prepared().arguments()["cwd"], workspace.root());
    if let Some(prompt) = approvals.resolve(prompt) {
        app.present_approval(prompt);
    }
    assert_eq!(app.pending_approval_count(), 0);
    assert_success(outcome(&mut receiver).await);
    assert!(fixture.provider.requests().is_empty());
    assert!(
        !fixture
            .policy
            .is_session_allowed(fixture.host.session().id(), "shell")
    );
    fixture.host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn matching_model_call_after_shortcut_still_presents_approval() {
    let mut fixture = Fixture::new(ApprovalMode::Ask, true).await;
    let approvals = LocalShellApprovals::default();
    let mut receiver = fixture.start(&approvals).await;
    let prompt = fixture.prompt().await;
    assert!(approvals.resolve(prompt).is_none());
    assert_success(outcome(&mut receiver).await);
    fixture.model_asks(&approvals).await;
    fixture.host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn submission_authorization_is_consumed_only_once() {
    use agent_runtime_core::approval::{ApprovalPolicy, ApprovalRequest};

    let mut fixture = Fixture::new(ApprovalMode::Ask, true).await;
    let approvals = LocalShellApprovals::default();
    let mut receiver = fixture.start(&approvals).await;
    let prompt = fixture.prompt().await;
    let duplicate = ApprovalRequest::new(
        prompt.prepared().clone(),
        prompt.deadline(),
        prompt.origin().clone(),
    );
    assert!(approvals.resolve(prompt).is_none());
    let policy = fixture.policy.clone();
    let decision = tokio::spawn(async move { policy.decide(&duplicate).await });
    let duplicate_prompt = fixture.prompt().await;
    let duplicate_prompt = approvals
        .resolve(duplicate_prompt)
        .expect("one-shot was consumed");
    duplicate_prompt.deny("test denial");
    assert!(!decision.await.expect("approval task").is_allowed());
    assert_success(outcome(&mut receiver).await);
    fixture.host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn matching_model_call_queued_during_shortcut_cannot_consume_authorization() {
    let mut fixture = Fixture::new(ApprovalMode::Ask, true).await;
    let approvals = LocalShellApprovals::default();
    let mut receiver = fixture.start(&approvals).await;
    let local_prompt = fixture.prompt().await;
    let local_turn = local_prompt.origin().turn().cloned().expect("local turn");
    let model = fixture
        .host
        .session()
        .send(UserInput::text("Run echo shortcut in the shell"))
        .expect("queued model turn");
    assert_ne!(&local_turn, model.id());
    assert!(fixture.provider.requests().is_empty());
    assert!(approvals.resolve(local_prompt).is_none());
    let model_prompt = fixture.prompt().await;
    assert_eq!(model_prompt.origin().turn(), Some(model.id()));
    let model_prompt = approvals
        .resolve(model_prompt)
        .expect("queued model call still asks");
    model_prompt.deny("test denial");
    assert_success(outcome(&mut receiver).await);
    tokio::time::timeout(Duration::from_secs(10), model.completed())
        .await
        .expect("model turn completed");
    assert!(
        !fixture
            .policy
            .is_session_allowed(fixture.host.session().id(), "shell")
    );
    fixture.host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn rejected_shortcut_cannot_approve_an_existing_identical_model_call() {
    let mut fixture = Fixture::new(ApprovalMode::Ask, true).await;
    let approvals = LocalShellApprovals::default();
    let model = fixture
        .host
        .session()
        .send(UserInput::text("Run echo shortcut in the shell"))
        .expect("model turn");
    let model_prompt = fixture.prompt().await;
    let mut receiver = fixture.start(&approvals).await;
    match outcome(&mut receiver).await {
        LocalOutcome::Shell {
            content,
            is_error,
            call,
            ..
        } => {
            assert!(is_error && call.is_none());
            assert!(content.contains("idle session"), "{content}");
        }
        _ => panic!("busy local action must fail"),
    }
    let model_prompt = approvals
        .resolve(model_prompt)
        .expect("existing model call still asks");
    model_prompt.deny("test denial");
    tokio::time::timeout(Duration::from_secs(10), model.completed())
        .await
        .expect("model turn completed");
    fixture.host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn unconsumed_authorization_is_discarded_when_local_call_is_cancelled() {
    let mut fixture = Fixture::new(ApprovalMode::Ask, true).await;
    let approvals = LocalShellApprovals::default();
    let mut receiver = fixture.start(&approvals).await;
    let held_prompt = fixture.prompt().await;
    // Keep the prompt unanswered so the token is still unconsumed when
    // the runtime's cancellation ends this local action.
    fixture
        .host
        .session()
        .interrupt_current_turn(CancelReason::UserRequested)
        .expect("interrupt local action");
    match outcome(&mut receiver).await {
        LocalOutcome::Shell {
            content, is_error, ..
        } => {
            assert!(is_error, "{content}");
            assert!(content.contains("cancel"), "{content}");
        }
        LocalOutcome::Error(error) => assert!(error.contains("cancel"), "{error}"),
        LocalOutcome::Notice { .. } | LocalOutcome::Agent(_) | LocalOutcome::Review(_) => {
            panic!("expected cancellation")
        }
    }
    // Even the original prepared request must no longer match: checking
    // only a new model turn would not prove the old token was discarded.
    let held_prompt = approvals
        .resolve(held_prompt)
        .expect("unused token was discarded");
    held_prompt.cancel();
    assert!(fixture.provider.requests().is_empty());
    fixture.model_asks(&approvals).await;
    fixture.host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn deny_policy_still_refuses_the_shortcut() {
    let fixture = Fixture::new(ApprovalMode::Deny, true).await;
    let approvals = LocalShellApprovals::default();
    let mut receiver = fixture.start(&approvals).await;
    match outcome(&mut receiver).await {
        LocalOutcome::Shell {
            content, is_error, ..
        } => {
            assert!(is_error, "{content}");
            assert!(content.contains("approval declined"), "{content}");
        }
        _ => panic!("deny policy must return a denied shell result"),
    }
    assert!(fixture.provider.requests().is_empty());
    assert!(
        !fixture
            .policy
            .is_session_allowed(fixture.host.session().id(), "shell")
    );
    fixture.host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn finished_shortcut_is_visible_after_resuming_the_host() {
    let mut fixture = Box::pin(Fixture::new(ApprovalMode::Ask, true)).await;
    let approvals = LocalShellApprovals::default();
    Box::pin(fixture.model_asks(&approvals)).await;
    let provider_requests = fixture.provider.requests().len();
    let mut app = fixture.app();
    crate::tui_driver::restore_transcript(
        &fixture.host,
        &mut app,
        &fixture.host.snapshot().history,
    );
    app.composer.replace(format!("!{COMMAND}"));
    let Some(Action::RunShell { command }) =
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("expected a shell action");
    };
    let echo = app.transcript.latest_shell_echo().expect("shell echo");
    let anchor = fixture.host.session().with_history(|history| history.len());
    assert!(anchor > 0);
    let mut shortcuts = crate::tui_driver::ShellShortcuts::default();
    shortcuts.dispatched(&fixture.host, echo);
    assert!(fixture.host.saved_shell_shortcuts().is_empty());
    let (outcomes, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let _ = Box::pin(start_local_shell(
        echo,
        fixture.host.session().clone(),
        command,
        TIMEOUT_MS,
        approvals.clone(),
        outcomes,
    ))
    .await;
    assert!(fixture.host.saved_shell_shortcuts().is_empty());
    assert!(approvals.resolve(fixture.prompt().await).is_none());
    let LocalOutcome::Shell {
        echo,
        call,
        content,
        is_error,
    } = outcome(&mut receiver).await
    else {
        panic!("expected a shell result");
    };
    assert!(!is_error, "{content}");
    shortcuts.finish(
        &fixture.host,
        &mut app,
        echo,
        call.as_ref().map(|call| call.as_str()),
        &content,
        is_error,
    );
    let saved = fixture.host.saved_shell_shortcuts();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].anchor, anchor);
    assert_eq!(saved[0].command, COMMAND);
    assert_eq!(
        fixture.host.session().with_history(|history| history.len()),
        anchor
    );
    assert_eq!(fixture.provider.requests().len(), provider_requests);
    let session_id = fixture.host.session().id().clone();
    Box::pin(fixture.host.shutdown()).await.expect("shutdown");

    let config =
        resolve(&ResolveRequest::new(fixture.project.path()).with_home_dir(fixture._home.path()))
            .expect("resume resolution")
            .config;
    let runtime = RuntimeRequest {
        provider: Some(fixture.provider.clone()),
        workspace: Some(Arc::new(
            ProjectWorkspace::new(fixture.project.path()).expect("workspace"),
        )),
        approval: Some(fixture.policy.clone()),
        ..RuntimeRequest::new(config, HostSurface::Terminal)
    };
    let resumed = Box::pin(smith_runtime::host::start(
        HostSessionRequest::new(runtime, fixture.project.path())
            .resume(session_id)
            .checkpoint_keys(Arc::new(TestCheckpointKeys)),
    ))
    .await
    .expect("resumed host");
    assert_eq!(resumed.saved_shell_shortcuts(), saved);
    let mut resumed_app = fixture.app();
    crate::tui_driver::restore_transcript(&resumed, &mut resumed_app, &resumed.snapshot().history);
    assert_eq!(app.transcript.blocks(), resumed_app.transcript.blocks());
    assert!(matches!(
        resumed_app.transcript.blocks().last().expect("restored shortcut"),
        Block::Tool {
            user_command: Some(command),
            result_preview: Some(result),
            status: smith_tui::transcript::ToolStatus::Ok,
            started_at: None,
            ..
        } if command == COMMAND && result.contains("shortcut")
    ));
    Box::pin(resumed.shutdown())
        .await
        .expect("resumed shutdown");
}

#[test]
fn double_exclamation_still_submits_a_literal_prompt() {
    let mut app = App::new("example-model", "/repo");
    app.composer.replace("!!echo shortcut");
    let Some(Action::Submit { submission, .. }) =
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("expected a literal prompt rather than a shell action");
    };
    let input = submission.input_without_files();
    assert_eq!(input.parts[0].as_text(), Some("!echo shortcut"));
}
