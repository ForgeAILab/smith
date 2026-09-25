//! Turn submission, event-stream, interaction, and recovery tests.

use super::*;

#[test]
fn a_failed_parent_does_not_wait_for_pending_child_delivery() {
    assert!(finish_waits_for_required_follow_up(&TurnFinish::Completed));
    assert!(!finish_waits_for_required_follow_up(&TurnFinish::Failed));
}

#[tokio::test]
async fn rejected_headless_submission_is_a_structured_machine_result() {
    const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;
    let home = tempfile::tempdir().expect("a home");
    let project = tempfile::tempdir().expect("a project");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a config directory");
    std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");
    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved config")
        .config;
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a workspace"),
        )),
        approval: Some(Arc::new(HeadlessApproval::new())),
        provider: Some(Arc::new(FakeProvider::text_reply("unused")) as Arc<dyn Provider>),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = smith_runtime::host::start(host_request(runtime, project.path()))
        .await
        .expect("a host");
    let initial_activation = host
        .session()
        .activation_epoch()
        .expect("the protected bootstrap activation");
    host.session().cancel_session(CancelReason::Shutdown);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let outcome = run_with_io(
        &host,
        "must be rejected".into(),
        OutputFormat::StreamJson,
        HeadlessBrokers::default(),
        BackgroundExit::Error,
        &mut stdout,
        &mut stderr,
    )
    .await
    .expect("a structured result");

    assert_eq!(outcome.exit_code, 1);
    assert!(stderr.is_empty());
    let lines = String::from_utf8(stdout)
        .expect("UTF-8 JSONL")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("one JSON value"))
        .collect::<Vec<_>>();
    let result = terminal_stream_result(&lines);
    assert_eq!(result["status"], "failed");
    assert_eq!(result["turn_id"], "");
    assert_eq!(
        result["lifecycle"]["activation"]["epoch"],
        initial_activation.index()
    );
    assert_eq!(
        result["lifecycle"]["activation"]["capabilities"],
        serde_json::Value::Array(
            initial_activation
                .activated()
                .iter()
                .map(|(id, _)| serde_json::Value::String(id.to_string()))
                .collect()
        )
    );
    assert!(
        result["error"]
            .as_str()
            .is_some_and(|error| error.contains("no longer accepts turns"))
    );
}

#[tokio::test]
async fn headless_result_never_reuses_an_older_session_answer() {
    const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;
    let home = tempfile::tempdir().expect("a home");
    let project = tempfile::tempdir().expect("a project");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a config directory");
    std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");
    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved config")
        .config;
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "answer from an older turn".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
            ScriptedStream::new(vec![
                ProviderStreamEvent::ReasoningDelta {
                    text: "the current turn has no visible answer".into(),
                    redacted: false,
                    signature: None,
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a workspace"),
        )),
        approval: Some(Arc::new(HeadlessApproval::new())),
        provider: Some(provider as Arc<dyn Provider>),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = smith_runtime::host::start(host_request(runtime, project.path()))
        .await
        .expect("a host");
    host.session()
        .run(UserInput::text("first turn"))
        .await
        .expect("the older turn runs");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let outcome = run_with_io(
        &host,
        "current turn".into(),
        OutputFormat::Json,
        HeadlessBrokers::default(),
        BackgroundExit::Error,
        &mut stdout,
        &mut stderr,
    )
    .await
    .expect("a structured result");

    assert_eq!(outcome.exit_code, 0);
    assert!(stderr.is_empty());
    let result: serde_json::Value = serde_json::from_slice(&stdout).expect("a result envelope");
    assert_eq!(result["status"], "ok");
    assert_eq!(result["output"], "");
    assert_ne!(result["turn_id"], "");
    assert!(
        !String::from_utf8(stdout)
            .expect("UTF-8 output")
            .contains("answer from an older turn")
    );
}

#[tokio::test]
async fn headless_follows_an_explicit_goal_until_its_internal_turn_completes() {
    const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;
    let home = tempfile::tempdir().expect("a home");
    let project = tempfile::tempdir().expect("a project");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a config directory");
    std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");

    let mut create = tool_call_fragments(
        0,
        "call-create",
        "create_goal",
        r#"{"objective":"finish the explicit goal"}"#,
    );
    create.push(usage_event(10, 2));
    create.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let mut complete = tool_call_fragments(
        0,
        "call-complete",
        "update_goal",
        r#"{"id":"goal-call-create","generation":2,"status":"complete"}"#,
    );
    complete.push(usage_event(8, 2));
    complete.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(create),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "goal accepted".into(),
                },
                usage_event(4, 1),
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
            ScriptedStream::new(complete),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "goal complete".into(),
                },
                usage_event(3, 1),
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved config")
        .config;
    assert!(config.persistence.enabled.value);
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a workspace"),
        )),
        approval: Some(Arc::new(HeadlessApproval::new())),
        provider: Some(provider.clone() as Arc<dyn Provider>),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = smith_runtime::host::start(host_request(runtime, project.path()))
        .await
        .expect("a host");
    assert!(host.runtime().goal_component().is_some());
    let session = host.session().clone();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let execution = tokio::time::timeout(
            HEADLESS_TEST_WATCHDOG,
            run_with_io(
                &host,
                "Use create_goal to create an explicit persistent multi-turn goal, then continue it until complete".into(),
                OutputFormat::Json,
                HeadlessBrokers::default(),
                BackgroundExit::Error,
                &mut stdout,
                &mut stderr,
            ),
        )
        .await;
    let outcome = match execution {
        Ok(result) => result.expect("goal result"),
        Err(_) => {
            let goal = host.goal();
            let requests = provider.requests();
            let request_tools = requests
                .iter()
                .take(4)
                .map(|request| {
                    request
                        .tools
                        .iter()
                        .map(|tool| tool.name.clone())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let complete_result =
                host.tool_result_text(&agent_runtime_core::ids::ToolCallId::new("call-complete"));
            let timeline = host.client_timeline_events().await.unwrap_or_default();
            let errors = timeline
                .iter()
                .filter_map(|event| match &event.payload {
                    RuntimeEvent::Error { error } => Some(error.to_string()),
                    _ => None,
                })
                .take(3)
                .collect::<Vec<_>>();
            let _ = host.shutdown().await;
            panic!(
                "goal execution did not stop; goal={goal:?}; requests={}; request_tools={request_tools:?}; complete_result={complete_result:?}; errors={errors:?}",
                requests.len(),
            );
        }
    };

    assert_eq!(outcome.exit_code, 0);
    assert!(stderr.is_empty());
    let result: serde_json::Value = serde_json::from_slice(&stdout).expect("goal result JSON");
    assert_eq!(result["goal"]["status"], "complete", "{result:#}");
    assert_eq!(result["goal"]["usage"]["charged_tokens"], 19);
    assert_eq!(result["goal"]["usage"]["provenance"], "provider_reported");
    assert_eq!(result["goal_continuation_turns"], 1);
    assert_eq!(result["output"], "goal complete");
    assert_eq!(provider.requests().len(), 4);
    assert_eq!(
        session
            .history()
            .iter()
            .filter(|message| message.role == Role::User)
            .count(),
        1,
        "the internal continuation created a synthetic user message"
    );
}

#[tokio::test]
async fn a_headless_edit_without_authority_is_structured_denied_and_redacted() {
    const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

# A turn carries no wall-clock ceiling by default. This test covers the
# deadline reaching the approval envelope end to end, so it opts back in to
# one; the absent case is covered where the envelope is built directly.
[limits]
turn_time_limit_ms = 600000
"#;
    const PROTECTED: &str = "TOP-SECRET-REPLACEMENT";

    let home = tempfile::tempdir().expect("a home");
    let project = tempfile::tempdir().expect("a project");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a config directory");
    std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");
    let target = project.path().join("target.txt");
    std::fs::write(&target, "safe\n").expect("a target");

    let mut tool = tool_call_fragments(
        0,
        "call-edit",
        "edit",
        &serde_json::json!({
            "path": "target.txt",
            "old_string": "safe",
            "new_string": PROTECTED,
        })
        .to_string(),
    );
    tool.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let final_answer = vec![
        ProviderStreamEvent::TextDelta {
            text: "The edit was not authorized.".into(),
        },
        ProviderStreamEvent::Finish {
            reason: FinishReason::Stop,
        },
    ];
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![ScriptedStream::new(tool), ScriptedStream::new(final_answer)],
    ));

    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved config")
        .config;
    let approval = Arc::new(HeadlessApproval::new());
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a workspace"),
        )),
        approval: Some(approval.clone()),
        provider: Some(provider as Arc<dyn Provider>),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = smith_runtime::host::start(host_request(runtime, project.path()))
        .await
        .expect("a host");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let outcome = tokio::time::timeout(
        HEADLESS_TEST_WATCHDOG,
        run_with_io(
            &host,
            "edit the file".into(),
            OutputFormat::Json,
            HeadlessBrokers {
                approval: Some(approval.as_ref()),
                ..HeadlessBrokers::default()
            },
            BackgroundExit::Error,
            &mut stdout,
            &mut stderr,
        ),
    )
    .await
    .expect("headless approval must not wait for stdin")
    .expect("a presented result");

    assert_eq!(outcome.exit_code, APPROVAL_REQUIRED_EXIT);
    assert_eq!(
        std::fs::read_to_string(target).expect("target contents"),
        "safe\n"
    );
    assert!(stderr.is_empty(), "JSON diagnostics leaked to stderr");
    let rendered = String::from_utf8(stdout).expect("UTF-8 JSON");
    assert!(!rendered.contains(PROTECTED), "{rendered}");
    let result: serde_json::Value =
        serde_json::from_str(rendered.trim()).expect("a result envelope");
    assert_eq!(result["status"], "approval_required");
    assert_eq!(result["approval_required"]["tool"], "edit");
    assert_eq!(
        result["approval_required"]["argument_keys"],
        // `operation` is normalized in by `edit::prepare`, so the key the
        // permission set was derived from is visible to the approver.
        serde_json::json!([
            "new_string",
            "old_string",
            "operation",
            "path",
            "replace_all"
        ])
    );
    assert_eq!(
        result["approval_required"]["permissions"],
        serde_json::json!(["fs.read", "fs.write"])
    );
    assert_eq!(
        result["approval_required"]["resource"]["resource_kind"],
        "filesystem"
    );
    assert_eq!(
        result["approval_required"]["resource"]["segments"],
        serde_json::json!(["target.txt"])
    );
    assert_eq!(
        result["approval_required"]["preparation_fingerprint"]
            .as_str()
            .map(str::len),
        Some(32)
    );
    // The configured ceiling above must survive the whole path into the
    // approval envelope: an approver that cannot see when its window
    // closes has to guess.
    assert!(
        result["approval_required"]["deadline_at_ms"]
            .as_u64()
            .is_some(),
        "{result}"
    );
}

#[tokio::test]
async fn stream_json_projects_attempts_activation_todos_and_recoverable_artifacts() {
    const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;
    const DISCARDED: &str = "FAILED ATTEMPT MUST NOT BECOME FINAL OUTPUT";
    let home = tempfile::tempdir().expect("a home");
    let project = tempfile::tempdir().expect("a project");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a config directory");
    std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");

    let mut todos = tool_call_fragments(
        0,
        "call-plan",
        "write_todos",
        &serde_json::json!({
            "items": [
                {
                    "id": "inspect",
                    "text": "Inspect the retry evidence",
                    "status": "completed"
                },
                {
                    "id": "capture",
                    "text": "Capture the large diagnostic",
                    "status": "in_progress"
                }
            ]
        })
        .to_string(),
    );
    todos.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let command = "yes 'headless artifact line' | head -c 262144";
    let mut shell = tool_call_fragments(
        0,
        "call-shell",
        "shell",
        &serde_json::json!({ "command": command }).to_string(),
    );
    shell.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: DISCARDED.into(),
                },
                ProviderStreamEvent::Error {
                    error: ProviderError::new(
                        ProviderErrorKind::Server,
                        "retry the deterministic fixture",
                    )
                    .retryable(),
                },
            ]),
            ScriptedStream::new(todos),
            ScriptedStream::new(shell),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "final committed answer".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved config")
        .config;
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a workspace"),
        )),
        approval: Some(Arc::new(AllowAll)),
        provider: Some(provider as Arc<dyn Provider>),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = smith_runtime::host::start(host_request(runtime, project.path()))
        .await
        .expect("a host");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let outcome = run_with_io(
        &host,
        "Use write_todos, then shell, for this multi-step diagnostic.".into(),
        OutputFormat::StreamJson,
        HeadlessBrokers::default(),
        BackgroundExit::Error,
        &mut stdout,
        &mut stderr,
    )
    .await
    .expect("a stream result");

    assert_eq!(outcome.exit_code, 0);
    assert!(stderr.is_empty());
    let lines = String::from_utf8(stdout)
        .expect("UTF-8 JSONL")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("one JSON value"))
        .collect::<Vec<_>>();
    let events = &lines[..lines.len() - 1];
    assert!(
        events.iter().any(|line| {
            line["event"]["payload"]["event"] == "provider_attempt_output_discarded"
        })
    );
    assert!(
        events.iter().any(|line| {
            line["event"]["payload"]["event"] == "provider_attempt_output_committed"
        })
    );
    assert!(
        events
            .iter()
            .any(|line| line["event"]["payload"]["event"] == "capabilities_activated")
    );
    assert!(
        events
            .iter()
            .any(|line| line["event"]["payload"]["event"] == "plan_updated")
    );

    let result = lines.last().expect("a terminal result");
    assert_eq!(result["status"], "ok");
    assert_eq!(result["output"], "final committed answer");
    assert_eq!(result["lifecycle"]["attempts_discarded"], 1);
    assert_eq!(result["lifecycle"]["attempts_committed"], 3);
    assert_eq!(result["lifecycle"]["plan"]["revision"], 2);
    assert_eq!(result["lifecycle"]["plan"]["counts"]["in_progress"], 0);
    assert_eq!(result["lifecycle"]["plan"]["counts"]["cancelled"], 1);
    assert!(
        result["lifecycle"]["activation"]["capabilities"]
            .as_array()
            .is_some_and(|capabilities| capabilities.iter().any(|id| {
                id.as_str()
                    .is_some_and(|id| id.contains("write_todos") || id.contains("shell"))
            }))
    );
    assert_eq!(result["artifacts"].as_array().map(Vec::len), Some(1));
    assert!(!result.to_string().contains(DISCARDED));

    let reference: ArtifactRef =
        serde_json::from_value(result["artifacts"][0].clone()).expect("typed artifact");
    let page = host
        .runtime()
        .artifact_store()
        .expect("protected artifact store")
        .read(ArtifactRead {
            session: host.session().id().clone(),
            id: reference.id,
            offset: 0,
            limit: MAX_ARTIFACT_READ_BYTES,
        })
        .await
        .expect("the reported artifact is readable by its session");
    assert!(String::from_utf8_lossy(&page.bytes).contains("headless artifact line"));
}

#[tokio::test]
async fn a_forced_headless_question_is_structured_and_never_reads_stdin() {
    const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;
    const SENSITIVE_PROMPT: &str = "Which unreleased codename?";
    let home = tempfile::tempdir().expect("a home");
    let project = tempfile::tempdir().expect("a project");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a config directory");
    std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");

    let arguments = serde_json::json!({
        "questions": [{
            "id": "codename",
            "header": "Codename",
            "prompt": SENSITIVE_PROMPT,
            "choices": [{"id": "alpha", "label": "Alpha"}],
            "allow_free_form": true
        }],
        "sensitivity": "sensitive"
    })
    .to_string();
    let mut question = tool_call_fragments(0, "call-question", "ask_user", &arguments);
    question.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(question),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "Input was unavailable.".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved config")
        .config;
    let interaction = Arc::new(HeadlessInteraction::new());
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a workspace"),
        )),
        approval: Some(Arc::new(HeadlessApproval::new())),
        interaction: Some(interaction.clone()),
        provider: Some(provider.clone() as Arc<dyn Provider>),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = smith_runtime::host::start(host_request(runtime, project.path()))
        .await
        .expect("a host");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    // Runtime construction and the scripted follow-up can be delayed when
    // the full binary test suite contends for CI workers. The broker is
    // still required to resolve without stdin; this bound only avoids
    // treating scheduler contention as an interaction regression.
    let outcome = tokio::time::timeout(
        HEADLESS_TEST_WATCHDOG,
        run_with_io(
            &host,
            "ask me for the codename".into(),
            OutputFormat::Json,
            HeadlessBrokers {
                interaction: Some(interaction.as_ref()),
                ..HeadlessBrokers::default()
            },
            BackgroundExit::Error,
            &mut stdout,
            &mut stderr,
        ),
    )
    .await
    .expect("headless interaction must not wait for stdin")
    .expect("a structured outcome");

    assert_eq!(outcome.exit_code, INTERACTION_REQUIRED_EXIT);
    assert!(stderr.is_empty(), "JSON diagnostics leaked to stderr");
    let rendered = String::from_utf8(stdout).expect("UTF-8 result");
    assert!(!rendered.contains(SENSITIVE_PROMPT), "{rendered}");
    let result: serde_json::Value = serde_json::from_str(rendered.trim()).expect("result JSON");
    assert_eq!(result["schema_version"], OUTPUT_SCHEMA_VERSION);
    assert_eq!(result["status"], "interaction_required");
    assert_eq!(result["interaction_required"]["question_count"], 1);
    assert!(
        provider.requests()[0]
            .tools
            .iter()
            .all(|tool| tool.name != "ask_user"),
        "ordinary headless planning advertised the questionnaire ability"
    );
}

#[tokio::test]
async fn a_restored_question_returns_the_same_request_without_submitting_a_new_turn() {
    const CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;
    const SENSITIVE_PROMPT: &str = "Which private recovery branch?";
    const NEW_PROMPT: &str = "THIS MUST NOT BECOME A SECOND USER TURN";
    let home = tempfile::tempdir().expect("a home");
    let project = tempfile::tempdir().expect("a project");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a config directory");
    std::fs::write(config_dir.join("config.toml"), CONFIG).expect("a config");

    let arguments = serde_json::json!({
        "questions": [{
            "id": "branch",
            "header": "Branch",
            "prompt": SENSITIVE_PROMPT,
            "choices": [{"id": "safe", "label": "Safe"}]
        }],
        "sensitivity": "sensitive"
    })
    .to_string();
    let mut question = tool_call_fragments(0, "recovery-question", "ask_user", &arguments);
    question.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let first_provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(question),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "first host closed the question".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let (interactive, mut requests) = InteractiveInteraction::new();
    let first_config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved config")
        .config;
    let first_runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a workspace"),
        )),
        approval: Some(Arc::new(DenyAll)),
        interaction: Some(Arc::new(interactive)),
        provider: Some(first_provider as Arc<dyn Provider>),
        ..RuntimeRequest::new(first_config, HostSurface::Terminal)
    };
    let first = smith_runtime::host::start(host_request(first_runtime, project.path()))
        .await
        .expect("interactive host");
    let session_id = first.session().id().clone();
    let checkpoint_path = first
        .paths()
        .expect("persistent paths")
        .checkpoint(&session_id)
        .expect("checkpoint path");
    let turn = first
        .session()
        .send(UserInput::text("ask the recovery question"))
        .expect("accepted first turn");
    let turn_id = turn.id().clone();
    let InteractionNotice::Present(prompt) =
        requests.recv().await.expect("questionnaire presentation")
    else {
        panic!("expected a questionnaire presentation");
    };
    let request_id = prompt.request().id().clone();
    let pending_checkpoint = std::fs::read(&checkpoint_path).expect("protected pending checkpoint");
    assert!(
        !pending_checkpoint
            .windows(SENSITIVE_PROMPT.len())
            .any(|window| window == SENSITIVE_PROMPT.as_bytes()),
        "the protected envelope exposed plaintext questionnaire content"
    );

    prompt.cancel().expect("close the first presentation");
    turn.completed().await;
    first.shutdown().await.expect("first host shutdown");

    // Model an abrupt process loss at the captured AwaitingInteraction
    // boundary after the orderly test owner has released the lifecycle
    // lease. The later journal tail is intentionally left in place so
    // startup also exercises checkpoint-watermark reconciliation.
    std::fs::write(&checkpoint_path, &pending_checkpoint)
        .expect("restore the pending crash boundary");

    let recovery_provider = Arc::new(FakeProvider::text_reply(
        "recovered with unavailable interaction",
    ));
    let headless_interaction = Arc::new(HeadlessInteraction::new());
    let recovery_config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved recovery config")
        .config;
    let recovery_runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a workspace"),
        )),
        approval: Some(Arc::new(DenyAll)),
        interaction: Some(headless_interaction.clone()),
        provider: Some(recovery_provider.clone() as Arc<dyn Provider>),
        ..RuntimeRequest::new(recovery_config, HostSurface::Headless)
    };
    let recovered = smith_runtime::host::start(
        host_request(recovery_runtime, project.path()).resume(session_id.clone()),
    )
    .await
    .expect("headless recovery host");
    let restored = recovered
        .restored_interaction()
        .expect("pending interaction metadata");
    assert_eq!(restored.request_id(), &request_id);
    assert_eq!(restored.turn_id(), &turn_id);
    assert_eq!(restored.question_count(), 1);

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let outcome = tokio::time::timeout(
        HEADLESS_TEST_WATCHDOG,
        run_with_io(
            &recovered,
            NEW_PROMPT.into(),
            OutputFormat::StreamJson,
            HeadlessBrokers {
                interaction: Some(headless_interaction.as_ref()),
                ..HeadlessBrokers::default()
            },
            BackgroundExit::Error,
            &mut stdout,
            &mut stderr,
        ),
    )
    .await
    .expect("restored headless interaction never waits for stdin")
    .expect("structured restored result");

    assert_eq!(outcome.exit_code, INTERACTION_REQUIRED_EXIT);
    assert!(stderr.is_empty(), "JSON diagnostics leaked to stderr");
    let rendered = String::from_utf8(stdout).expect("UTF-8 result");
    assert!(!rendered.contains(SENSITIVE_PROMPT), "{rendered}");
    assert!(!rendered.contains(NEW_PROMPT), "{rendered}");
    let lines = rendered
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("one JSON value"))
        .collect::<Vec<_>>();
    let result = terminal_stream_result(&lines);
    assert_eq!(result["schema_version"], OUTPUT_SCHEMA_VERSION);
    assert_eq!(result["status"], "interaction_required");
    assert_eq!(result["session_id"], session_id.as_str());
    assert_eq!(result["turn_id"], turn_id.as_str());
    assert_eq!(
        result["interaction_required"]["request_id"],
        request_id.as_str()
    );
    assert_eq!(result["interaction_required"]["question_count"], 1);
    assert!(
        headless_interaction.required().is_none(),
        "headless inspection consumed the restored interaction through its broker"
    );
    assert_eq!(
        std::fs::read(&checkpoint_path).expect("preserved pending checkpoint"),
        pending_checkpoint,
        "reporting interaction_required advanced the exact pending checkpoint"
    );
    assert!(
        recovery_provider.requests().iter().all(|request| {
            !serde_json::to_string(&request.messages)
                .expect("serializable provider messages")
                .contains(NEW_PROMPT)
        }),
        "the command-line prompt was submitted while an older interaction was being recovered"
    );

    let interactive_provider = Arc::new(FakeProvider::text_reply(
        "resumed after the exact restored answer",
    ));
    let (interactive, mut requests) = InteractiveInteraction::new();
    let interactive_config =
        resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved interactive config")
            .config;
    let interactive_runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a workspace"),
        )),
        approval: Some(Arc::new(DenyAll)),
        interaction: Some(Arc::new(interactive)),
        provider: Some(interactive_provider.clone() as Arc<dyn Provider>),
        ..RuntimeRequest::new(interactive_config, HostSurface::Terminal)
    };
    let interactive_host = smith_runtime::host::start(
        host_request(interactive_runtime, project.path()).resume(session_id),
    )
    .await
    .expect("interactive recovery host");
    let InteractionNotice::Present(prompt) =
        tokio::time::timeout(HEADLESS_TEST_WATCHDOG, requests.recv())
            .await
            .expect("interactive recovery presents without hanging")
            .expect("restored questionnaire presentation")
    else {
        panic!("expected the restored questionnaire presentation");
    };
    assert_eq!(prompt.request().id(), &request_id);
    assert_eq!(prompt.request().origin().turn(), &turn_id);
    prompt
        .answer(vec![
            agent_runtime_core::interaction::QuestionAnswer::choice(
                agent_runtime_core::ids::QuestionId::new("branch"),
                agent_runtime_core::ids::ChoiceId::new("safe"),
            ),
        ])
        .expect("the exact restored request accepts one answer");

    tokio::time::timeout(HEADLESS_TEST_WATCHDOG, async {
        loop {
            if interactive_host
                .session()
                .history()
                .iter()
                .any(|message| message.joined_text() == "resumed after the exact restored answer")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the original turn resumes after its restored answer");
    assert_eq!(
        interactive_provider.requests().len(),
        1,
        "restored answer repeated provider work or created another user turn"
    );
    interactive_host
        .shutdown()
        .await
        .expect("interactive recovery shutdown");
}
