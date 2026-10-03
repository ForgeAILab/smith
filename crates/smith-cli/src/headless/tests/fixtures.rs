//! Replaceable before-refactor recordings of the existing flow and output scenarios.

use std::io::Write;

use super::*;
use crate::tests::fixture_support::{FixedClock, Normalizer, compare_or_update};

const FORMATS: [OutputFormat; 3] = [
    OutputFormat::Text,
    OutputFormat::Json,
    OutputFormat::StreamJson,
];

fn record_io(
    name: &str,
    format: OutputFormat,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    normalizer: &mut Normalizer,
) {
    let stdout = String::from_utf8(stdout).expect("UTF-8 stdout");
    let stderr = String::from_utf8(stderr).expect("UTF-8 stderr");
    let (extension, captured) = match format {
        OutputFormat::Text => ("txt", format!("stdout:\n{stdout}stderr:\n{stderr}")),
        OutputFormat::Json => {
            assert!(stderr.is_empty(), "JSON diagnostics leaked: {stderr}");
            serde_json::from_str::<serde_json::Value>(&stdout).expect("valid emitted JSON");
            ("json", stdout)
        }
        OutputFormat::StreamJson => {
            assert!(stderr.is_empty(), "stream diagnostics leaked: {stderr}");
            for line in stdout.lines() {
                serde_json::from_str::<serde_json::Value>(line).expect("valid emitted JSONL");
            }
            ("jsonl", stdout)
        }
    };
    compare_or_update(
        &format!("headless/{name}.{extension}"),
        &normalizer.normalize(&captured),
    );
}

async fn record_flow(
    name: &str,
    host: &HostSession,
    format: OutputFormat,
    prompt: &str,
    brokers: HeadlessBrokers<'_>,
    mut normalizer: Normalizer,
    expected_exit: u8,
) {
    normalizer.session(host.session().id().as_str());
    normalizer.profile(&host.runtime().policy().agent_profile_revision);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let outcome = tokio::time::timeout(
        Duration::from_secs(60),
        Box::pin(run_with_io(
            host,
            prompt.into(),
            format,
            brokers,
            BackgroundExit::Error,
            &mut stdout,
            &mut stderr,
        )),
    )
    .await
    .expect("fixture flow watchdog")
    .expect("fixture outcome");
    assert_eq!(outcome.exit_code, expected_exit, "{name}");
    // Every flow builds its terminal projection after host shutdown.
    normalizer.headless_flow(true);
    record_io(name, format, stdout, stderr, &mut normalizer);
    Box::pin(host.shutdown()).await.expect("fixture shutdown");
}

// Each scenario constructs fresh homes, projects, and hosts for every format.
// Keep only a pinned pointer to the scenario future in each test, and box the
// large runtime futures inside it, rather than accumulating all eight flows
// in one future on the default test-thread stack.
#[tokio::test]
async fn fixtures_headless_answered_turn() {
    for format in FORMATS {
        Box::pin(answered_turn(format)).await;
    }
}

#[tokio::test]
async fn fixtures_headless_rejected_submission() {
    for format in FORMATS {
        Box::pin(rejected_submission(format)).await;
    }
}

#[tokio::test]
async fn fixtures_headless_empty_current_answer() {
    for format in FORMATS {
        Box::pin(empty_current_answer(format)).await;
    }
}

#[tokio::test]
async fn fixtures_headless_goal_complete() {
    for format in FORMATS {
        Box::pin(goal_complete(format)).await;
    }
}

#[tokio::test]
async fn fixtures_headless_denied_edit() {
    for format in FORMATS {
        Box::pin(denied_edit(format)).await;
    }
}

#[tokio::test]
async fn fixtures_headless_attempts_todos_artifacts() {
    for format in FORMATS {
        Box::pin(attempts_todos_artifacts(format)).await;
    }
}

#[tokio::test]
async fn fixtures_headless_forced_question() {
    for format in FORMATS {
        Box::pin(forced_question(format)).await;
    }
}

#[tokio::test]
async fn fixtures_headless_restored_question() {
    for format in FORMATS {
        Box::pin(restored_question(format)).await;
    }
}

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

async fn rejected_submission(format: OutputFormat) {
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
        clock: Some(Arc::new(FixedClock)),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = Box::pin(smith_runtime::host::start(host_request(
        runtime,
        project.path(),
    )))
    .await
    .expect("a host");
    host.session().cancel_session(CancelReason::Shutdown);

    Box::pin(record_flow(
        "rejected-submission",
        &host,
        format,
        "must be rejected",
        HeadlessBrokers::default(),
        Normalizer::new(home.path(), project.path()),
        1,
    ))
    .await;
}

async fn empty_current_answer(format: OutputFormat) {
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
        clock: Some(Arc::new(FixedClock)),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = Box::pin(smith_runtime::host::start(host_request(
        runtime,
        project.path(),
    )))
    .await
    .expect("a host");
    Box::pin(host.session().run(UserInput::text("first turn")))
        .await
        .expect("the older turn runs");

    Box::pin(record_flow(
        "empty-current-answer",
        &host,
        format,
        "current turn",
        HeadlessBrokers::default(),
        Normalizer::new(home.path(), project.path()),
        0,
    ))
    .await;
}

async fn goal_complete(format: OutputFormat) {
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
        clock: Some(Arc::new(FixedClock)),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = Box::pin(smith_runtime::host::start(host_request(
        runtime,
        project.path(),
    )))
    .await
    .expect("a host");
    assert!(host.runtime().goal_component().is_some());

    Box::pin(record_flow("goal-complete", &host, format, "Use create_goal to create an explicit persistent multi-turn goal, then continue it until complete", HeadlessBrokers::default(), Normalizer::new(home.path(), project.path()), 0)).await;
}

async fn denied_edit(format: OutputFormat) {
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
        clock: Some(Arc::new(FixedClock)),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = Box::pin(smith_runtime::host::start(host_request(
        runtime,
        project.path(),
    )))
    .await
    .expect("a host");

    Box::pin(record_flow(
        "denied-edit",
        &host,
        format,
        "edit the file",
        HeadlessBrokers {
            approval: Some(approval.as_ref()),
            ..HeadlessBrokers::default()
        },
        Normalizer::new(home.path(), project.path()),
        APPROVAL_REQUIRED_EXIT,
    ))
    .await;
}

async fn attempts_todos_artifacts(format: OutputFormat) {
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
    let command = r#"awk 'BEGIN { for (i = 0; i < 11397; i++) printf "headless artifact line\n"; printf "headless arti" }'"#;
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
        clock: Some(Arc::new(FixedClock)),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = Box::pin(smith_runtime::host::start(host_request(
        runtime,
        project.path(),
    )))
    .await
    .expect("a host");

    Box::pin(record_flow(
        "attempts-todos-artifacts",
        &host,
        format,
        "Use write_todos, then shell, for this multi-step diagnostic.",
        HeadlessBrokers::default(),
        Normalizer::new(home.path(), project.path()),
        0,
    ))
    .await;
}

async fn forced_question(format: OutputFormat) {
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
        clock: Some(Arc::new(FixedClock)),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = Box::pin(smith_runtime::host::start(host_request(
        runtime,
        project.path(),
    )))
    .await
    .expect("a host");

    Box::pin(record_flow(
        "forced-question",
        &host,
        format,
        "ask me for the codename",
        HeadlessBrokers {
            interaction: Some(interaction.as_ref()),
            ..HeadlessBrokers::default()
        },
        Normalizer::new(home.path(), project.path()),
        INTERACTION_REQUIRED_EXIT,
    ))
    .await;
}

async fn restored_question(format: OutputFormat) {
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
        clock: Some(Arc::new(FixedClock)),
        ..RuntimeRequest::new(first_config, HostSurface::Terminal)
    };
    let first = Box::pin(smith_runtime::host::start(host_request(
        first_runtime,
        project.path(),
    )))
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
        tokio::time::timeout(Duration::from_secs(60), Box::pin(requests.recv()))
            .await
            .expect("questionnaire watchdog")
            .expect("questionnaire presentation")
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
    Box::pin(turn.completed()).await;
    Box::pin(first.shutdown())
        .await
        .expect("first host shutdown");

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
        clock: Some(Arc::new(FixedClock)),
        ..RuntimeRequest::new(recovery_config, HostSurface::Headless)
    };
    let recovered = Box::pin(smith_runtime::host::start(
        host_request(recovery_runtime, project.path()).resume(session_id.clone()),
    ))
    .await
    .expect("headless recovery host");
    let restored = recovered
        .restored_interaction()
        .expect("pending interaction metadata");
    assert_eq!(restored.request_id(), &request_id);
    assert_eq!(restored.turn_id(), &turn_id);
    assert_eq!(restored.question_count(), 1);

    Box::pin(record_flow(
        "restored-question",
        &recovered,
        format,
        NEW_PROMPT,
        HeadlessBrokers {
            interaction: Some(headless_interaction.as_ref()),
            ..HeadlessBrokers::default()
        },
        Normalizer::new(home.path(), project.path()),
        INTERACTION_REQUIRED_EXIT,
    ))
    .await;
}

async fn answered_turn(format: OutputFormat) {
    let home = tempfile::tempdir().expect("home");
    let project = tempfile::tempdir().expect("project");
    std::fs::create_dir_all(project.path().join(".smith")).expect("config directory");
    std::fs::write(project.path().join(".smith/config.toml"), CONFIG).expect("config");
    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved config")
        .config;
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("workspace"),
        )),
        approval: Some(Arc::new(HeadlessApproval::new())),
        provider: Some(Arc::new(FakeProvider::text_reply("fixture answer"))),
        clock: Some(Arc::new(FixedClock)),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    let host = Box::pin(smith_runtime::host::start(host_request(
        runtime,
        project.path(),
    )))
    .await
    .expect("host");
    Box::pin(record_flow(
        "answered-turn",
        &host,
        format,
        "answer the fixture",
        HeadlessBrokers::default(),
        Normalizer::new(home.path(), project.path()),
        0,
    ))
    .await;
}

fn record_value(name: &str, value: &impl Serialize) {
    let mut output = Vec::new();
    write_json(&mut output, value).expect("fixture serialization");
    compare_or_update(
        &format!("headless/{name}.json"),
        &String::from_utf8(output).expect("UTF-8 JSON"),
    );
}

fn record_projection(name: &str, result: &ResultEnvelope, events: &[EventEnvelope]) {
    let mut normalizer = Normalizer::default();
    for format in FORMATS {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        match format {
            OutputFormat::Text => {
                if matches!(result.status, ResultStatus::Ok) {
                    write_text(&mut stdout, &result.output).expect("text output");
                }
                write_text_projection(&mut stderr, result).expect("text projection");
                if !matches!(result.status, ResultStatus::Ok) {
                    let diagnostic = if let Some(required) = &result.approval_required {
                        approval_diagnostic(required)
                    } else if let Some(required) = &result.interaction_required {
                        format!(
                            "interaction required for request `{}` ({} question(s)); rerun in an interactive terminal",
                            required.request_id, required.question_count
                        )
                    } else {
                        result.error.clone().unwrap_or_else(|| {
                            format!("turn ended with status {:?}", result.status)
                        })
                    };
                    writeln!(stderr, "smith: {diagnostic}").expect("text diagnostic");
                }
            }
            OutputFormat::Json => write_json(&mut stdout, result).expect("JSON output"),
            OutputFormat::StreamJson => {
                for event in events {
                    write_json(
                        &mut stdout,
                        &StreamEnvelope {
                            schema_version: OUTPUT_SCHEMA_VERSION,
                            kind: "event",
                            event,
                        },
                    )
                    .expect("stream event");
                }
                write_json(&mut stdout, result).expect("stream result");
            }
        }
        record_io(name, format, stdout, stderr, &mut normalizer);
    }
}

fn fixture_result() -> ResultEnvelope {
    ResultEnvelope {
        schema_version: OUTPUT_SCHEMA_VERSION,
        kind: "result",
        status: ResultStatus::Ok,
        session_id: "session-fixture".into(),
        turn_id: "turn-fixture".into(),
        provider: "fixture-provider".into(),
        model: "fixture-model".into(),
        output: "answer".into(),
        usage: UsageOutput {
            current_turn: UsageDelta::new(),
            session: UsageDelta::new(),
            synthetic_cache: SyntheticUsageOutput::default(),
            current_turn_provenance: UsageProvenance::Unknown,
            session_provenance: UsageProvenance::Unknown,
        },
        lifecycle: LifecycleOutput::default(),
        goal: None,
        goal_continuation_turns: None,
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: None,
        recovery: None,
        account: None,
        background_exit: None,
        reasoning: None,
        cache: None,
        resume_capsule: None,
        error: None,
    }
}

#[test]
fn fixtures_headless_output_cases() {
    for (name, finish, terminal, attempt) in [
        (
            "attempt-budget-error",
            TurnFinish::LimitReached {
                limit: LimitKind::ProviderAttempts,
            },
            None,
            Some("upstream 503: service unavailable"),
        ),
        (
            "output-limit",
            TurnFinish::LimitReached {
                limit: LimitKind::Output,
            },
            None,
            None,
        ),
        (
            "completed-retry",
            TurnFinish::Completed,
            None,
            Some("upstream 503: service unavailable"),
        ),
        (
            "failed-terminal-error",
            TurnFinish::Failed,
            Some("terminal"),
            Some("attempt"),
        ),
        (
            "failed-attempt-error",
            TurnFinish::Failed,
            None,
            Some("attempt"),
        ),
    ] {
        let mut result = fixture_result();
        result.status = outcome(Some(&finish), None, None, None, None).0;
        result.error = terminal_error(
            Some(&finish),
            terminal.map(str::to_owned),
            attempt.map(str::to_owned),
        );
        record_projection(name, &result, &[]);
    }
    let finish = TurnFinish::NeedsInput {
        request: agent_runtime_core::ids::InteractionRequestId::new("child-question"),
    };
    let mut result = fixture_result();
    result.status = outcome(Some(&finish), None, None, None, None).0;
    record_projection("returned-child-input", &result, &[]);
    record_value("empty-usage", &UsageProvenance::of(&UsageDelta::new()));
    let mut last = None;
    let mut error = None;
    for sequence in [4, 5, 8] {
        observe_sequence(&mut last, sequence, &mut error);
    }
    compare_or_update(
        "headless/sequence-gap.txt",
        &format!("{}\n", error.expect("gap diagnostic")),
    );
    compare_or_update(
        "headless/parent-completion-wait.txt",
        &format!(
            "completed: {}\nfailed: {}\n",
            finish_waits_for_required_follow_up(&TurnFinish::Completed),
            finish_waits_for_required_follow_up(&TurnFinish::Failed)
        ),
    );

    approval_exit_status();
    machine_result_v2();
    approval_required_v2();
    interaction_required_v2();
    durable_child();
    synthetic_usage();
    recovery_metadata();
    sensitive_plan();
    lifecycle_text();
    canonical_cache();
    approval_text();
    goal_terminals();
    for (name, source) in [
        (
            "legacy-machine-result-v1",
            include_str!("../../../tests/fixtures/machine-result-v1.json"),
        ),
        (
            "legacy-approval-required-v1",
            include_str!("../../../tests/fixtures/approval-required-v1.json"),
        ),
    ] {
        compare_or_update(&format!("headless/{name}.json"), source);
    }
}

fn machine_result_v2() {
    let usage = UsageDelta::new()
        .with(agent_runtime_core::usage::CounterKind::InputUncached, 12)
        .with(agent_runtime_core::usage::CounterKind::Output, 2);
    let result = ResultEnvelope {
        schema_version: 2,
        kind: "result",
        status: ResultStatus::Ok,
        session_id: "session-fixture".into(),
        turn_id: "turn-fixture".into(),
        provider: "fixture-provider".into(),
        model: "fixture-model".into(),
        output: "fixture answer".into(),
        usage: UsageOutput {
            current_turn: usage.clone(),
            session: usage,
            synthetic_cache: SyntheticUsageOutput::default(),
            current_turn_provenance: UsageProvenance::ProviderReported,
            session_provenance: UsageProvenance::ProviderReported,
        },
        lifecycle: LifecycleOutput {
            attempts_committed: 1,
            attempts_discarded: 0,
            activation: Some(ActivationOutput {
                epoch: 1,
                capabilities: vec!["tool:read".into()],
            }),
            plan: None,
            children: Vec::new(),
            parent_state: None,
        },
        goal: None,
        goal_continuation_turns: None,
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: None,
        recovery: None,
        account: None,
        background_exit: None,
        reasoning: None,
        cache: None,
        resume_capsule: None,
        error: None,
    };

    record_projection("machine-result-v2", &result, &[]);
}

fn approval_required_v2() {
    let result = ResultEnvelope {
        schema_version: 2,
        kind: "result",
        status: ResultStatus::ApprovalRequired,
        session_id: "session-fixture".into(),
        turn_id: "turn-fixture".into(),
        provider: "fixture-provider".into(),
        model: "fixture-model".into(),
        output: String::new(),
        usage: UsageOutput {
            current_turn: UsageDelta::new(),
            session: UsageDelta::new(),
            synthetic_cache: SyntheticUsageOutput::default(),
            current_turn_provenance: UsageProvenance::Unknown,
            session_provenance: UsageProvenance::Unknown,
        },
        lifecycle: LifecycleOutput::default(),
        goal: None,
        goal_continuation_turns: None,
        artifacts: Vec::new(),
        approval_required: Some(ApprovalOutput {
            call_id: "call-fixture".into(),
            tool: "edit".into(),
            argument_keys: vec!["new_string".into(), "old_string".into(), "path".into()],
            mutates: true,
            requires_authorization: true,
            permissions: vec!["fs.write".into()],
            resource: SecurityResource::filesystem("/repo", vec!["src".into(), "lib.rs".into()]),
            authority_warnings: Vec::new(),
            deadline_at_ms: Some(1_750_000_000_000),
            preparation_fingerprint: "0123456789abcdef0123456789abcdef".into(),
        }),
        interaction_required: None,
        recovery: None,
        account: None,
        background_exit: None,
        reasoning: None,
        cache: None,
        resume_capsule: None,
        error: None,
    };

    record_projection("approval-required-v2", &result, &[]);
}

fn interaction_required_v2() {
    let result = ResultEnvelope {
        schema_version: 2,
        kind: "result",
        status: ResultStatus::InteractionRequired,
        session_id: "session-fixture".into(),
        turn_id: "turn-fixture".into(),
        provider: "fixture-provider".into(),
        model: "fixture-model".into(),
        output: String::new(),
        usage: UsageOutput {
            current_turn: UsageDelta::new(),
            session: UsageDelta::new(),
            synthetic_cache: SyntheticUsageOutput::default(),
            current_turn_provenance: UsageProvenance::Unknown,
            session_provenance: UsageProvenance::Unknown,
        },
        lifecycle: LifecycleOutput::default(),
        goal: None,
        goal_continuation_turns: None,
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: Some(InteractionOutput {
            request_id: "interaction-fixture".into(),
            question_count: 2,
        }),
        recovery: None,
        account: None,
        background_exit: None,
        reasoning: None,
        cache: None,
        resume_capsule: None,
        error: None,
    };
    record_projection("interaction-required-v2", &result, &[]);
}

fn durable_child() {
    let lifecycle = LifecycleOutput {
        children: vec![ChildSessionOutput {
            child_id: "child-3".to_owned(),
            child_session_id: "child-session-3".to_owned(),
            durability: "durable".to_owned(),
            state: "interrupted".to_owned(),
            resumable: true,
            turns_used: 1,
            max_turns: None,
            tokens_used: 42,
            incompatibility: None,
        }],
        ..LifecycleOutput::default()
    };
    record_value("durable-child", &lifecycle);
}

fn synthetic_usage() {
    let ordinary = UsageRecord {
        source: UsageSource::ProviderAttempt,
        provenance: Provenance::default(),
        delta: UsageDelta::new().with(CounterKind::InputUncached, 100),
    };
    let keepalive = UsageRecord {
        source: UsageSource::ProviderAttempt,
        provenance: Provenance {
            attempt_purpose: Some(ProviderAttemptPurpose::CacheKeepalive),
            ..Provenance::default()
        },
        delta: UsageDelta::new()
            .with(CounterKind::InputCached, 80)
            .with(CounterKind::Output, 2),
    };
    let idle = UsageRecord {
        source: UsageSource::SemanticSummary,
        provenance: Provenance {
            purpose: Some("cache_idle_compaction".to_owned()),
            attempt_purpose: Some(ProviderAttemptPurpose::IdleCompaction),
            ..Provenance::default()
        },
        delta: UsageDelta::new()
            .with(CounterKind::InputUncached, 10)
            .with(CounterKind::Output, 3),
    };

    assert!(!is_synthetic_usage(&ordinary));
    assert!(is_synthetic_usage(&keepalive));
    assert!(is_synthetic_usage(&idle));
    let projected = SyntheticUsageOutput::from_records(&[ordinary, keepalive, idle]);
    record_value("synthetic-usage", &projected);
    let mut result = fixture_result();
    result.usage.synthetic_cache = projected;
    record_projection("synthetic-usage-result", &result, &[]);
}

fn recovery_metadata() {
    let interruption = EphemeralWorkInterruption::process_exit(
        [agent_runtime_core::ids::ChildId::new("child-2")],
        std::iter::empty::<String>(),
        std::iter::empty::<String>(),
    );
    record_value("recovery-metadata", &RecoveryOutput::from(&interruption));
}

fn sensitive_plan() {
    let protected_item = "PROTECTED PLAN CONTENT";
    let projected = plan_output(
        7,
        PlanSensitivity::Sensitive,
        BTreeMap::from([("pending".to_owned(), 1)]),
        Some(vec![PlanItemProjection {
            id: "protected".to_owned(),
            text: protected_item.to_owned(),
            status: smith_runtime::client::PlanItemStatus::Pending,
            reason: None,
        }]),
    );

    record_value("sensitive-plan", &projected);
}

fn lifecycle_text() {
    let protected_item = "PROTECTED TODO CONTENT";
    let result = ResultEnvelope {
        account: None,
        schema_version: OUTPUT_SCHEMA_VERSION,
        kind: "result",
        status: ResultStatus::Ok,
        session_id: "session-fixture".into(),
        turn_id: "turn-fixture".into(),
        provider: "fixture-provider".into(),
        model: "fixture-model".into(),
        output: "answer".into(),
        usage: UsageOutput {
            current_turn: UsageDelta::new(),
            session: UsageDelta::new(),
            synthetic_cache: SyntheticUsageOutput::default(),
            current_turn_provenance: UsageProvenance::Unknown,
            session_provenance: UsageProvenance::Unknown,
        },
        lifecycle: LifecycleOutput {
            attempts_committed: 2,
            attempts_discarded: 1,
            activation: Some(ActivationOutput {
                epoch: 3,
                capabilities: vec!["tool:read".into(), "tool:write_todos".into()],
            }),
            plan: Some(PlanOutput {
                revision: 4,
                sensitivity: PlanSensitivity::Sensitive,
                counts: BTreeMap::from([("in_progress".to_owned(), 1), ("pending".to_owned(), 2)]),
                items: Some(vec![PlanItemProjection {
                    id: "protected".to_owned(),
                    text: protected_item.to_owned(),
                    status: smith_runtime::client::PlanItemStatus::InProgress,
                    reason: None,
                }]),
            }),
            children: Vec::new(),
            parent_state: None,
        },
        goal: None,
        goal_continuation_turns: None,
        artifacts: vec![ArtifactRef {
            id: ArtifactId::new("artifact-fixture").expect("valid artifact id"),
            digest: ArtifactDigest::new("sha256", "ab12").expect("valid digest"),
            media_type: "text/plain".into(),
            byte_length: 262_144,
            sensitivity: ArtifactSensitivity::Sensitive,
            retention: ArtifactRetention::Session,
            provenance: ArtifactProvenance::new(
                agent_runtime_core::ids::SessionId::new("session-fixture"),
                "tool-output",
            ),
        }],
        approval_required: None,
        interaction_required: None,
        recovery: Some(RecoveryOutput {
            interrupted_turn: None,
            reason: "process_exit",
            interrupted_children: vec!["child-1".into()],
            interrupted_monitors: vec!["monitor-1".into()],
            interrupted_tasks: vec!["task-1".into()],
        }),
        background_exit: None,
        reasoning: None,
        cache: None,
        resume_capsule: None,
        error: None,
    };
    let mut stderr = Vec::new();
    write_text_projection(&mut stderr, &result).expect("text lifecycle projection");
    compare_or_update(
        "headless/lifecycle-text.txt",
        &String::from_utf8(stderr).expect("UTF-8 text"),
    );
}

fn canonical_cache() {
    let turn = TurnId::new("turn-cache");
    let event = |seq: u64, payload: RuntimeEvent| {
        EventEnvelope::new(
            seq,
            EventId::new(format!("cache-event-{seq}")),
            SessionId::new("session-cache"),
            Some(turn.clone()),
            Timestamp(seq.saturating_mul(60_000)),
            payload,
        )
    };
    let observation: RuntimeEvent = serde_json::from_value(serde_json::json!({
        "event": "cache_observation",
        "request": "request-cache",
        "attempt": "attempt-cache",
        "cache_plan": "plan-cache",
        "read_tokens": 0
    }))
    .expect("cache observation fixture");
    let state: RuntimeEvent = serde_json::from_value(serde_json::json!({
        "event": "cache_state_changed",
        "request": "request-cache",
        "attempt": "attempt-cache",
        "cache_plan": "plan-cache",
        "state": "miss_observed",
        "expected_read_tokens": 20_000,
        "observed_read_tokens": 0,
        "missed_tokens": 20_000,
        "confidence": "exact"
    }))
    .expect("cache state fixture");
    assert!(matches!(
        &state,
        RuntimeEvent::CacheStateChanged {
            state: CacheState::MissObserved,
            ..
        }
    ));
    let usage = UsageDelta::new().with(CounterKind::InputUncached, 20_000);
    let events = vec![
        event(
            1,
            RuntimeEvent::ProviderAttemptStarted {
                request: RequestId::new("request-cache"),
                attempt: AttemptId::new("attempt-cache"),
                index: 0,
                model: "fixture-model".to_owned(),
            },
        ),
        event(
            2,
            RuntimeEvent::Usage {
                record: UsageRecord {
                    source: UsageSource::ProviderAttempt,
                    provenance: Provenance {
                        request: Some(RequestId::new("request-cache")),
                        attempt: Some(AttemptId::new("attempt-cache")),
                        ..Provenance::default()
                    },
                    delta: usage.clone(),
                },
            },
        ),
        event(3, observation),
        event(4, state),
        event(
            5,
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ),
    ];

    let mut tui_status = smith_client::status::Status::new("fixture-model", "/fixture");
    for envelope in &events {
        tui_status.record_cache_event(envelope);
    }
    let tui_summary = tui_status.cache_summary().expect("TUI cache summary");

    let mut projection = CacheProjection::default();
    projection.replay(events.clone());
    let headless_summary = projection
        .latest_completed()
        .expect("headless cache summary")
        .clone();
    assert_eq!(tui_summary, headless_summary);
    assert_eq!(headless_summary.missed_tokens, Some(20_000));
    assert_eq!(headless_summary.rebilled_tokens, 20_000);

    let result = ResultEnvelope {
        schema_version: OUTPUT_SCHEMA_VERSION,
        kind: "result",
        status: ResultStatus::Ok,
        session_id: "session-cache".to_owned(),
        turn_id: "turn-cache".to_owned(),
        provider: "fixture-provider".to_owned(),
        model: "fixture-model".to_owned(),
        output: "fixture answer".to_owned(),
        usage: UsageOutput {
            current_turn: usage.clone(),
            session: usage,
            synthetic_cache: SyntheticUsageOutput::default(),
            current_turn_provenance: UsageProvenance::ProviderReported,
            session_provenance: UsageProvenance::ProviderReported,
        },
        lifecycle: LifecycleOutput::default(),
        goal: None,
        goal_continuation_turns: None,
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: None,
        recovery: None,
        account: None,
        background_exit: None,
        reasoning: None,
        cache: Some(CacheOutput::from_summary(&headless_summary, None)),
        resume_capsule: None,
        error: None,
    };
    record_projection("canonical-cache", &result, &events);
}

fn approval_text() {
    let diagnostic = approval_diagnostic(&ApprovalOutput {
        call_id: "call-fixture".into(),
        tool: "edit".into(),
        argument_keys: vec!["new_string".into(), "path".into()],
        mutates: true,
        requires_authorization: true,
        permissions: vec!["fs.read".into(), "fs.write".into()],
        resource: SecurityResource::filesystem("/repo", vec!["src".into(), "lib.rs".into()]),
        authority_warnings: vec!["workspace_root_mutation".into()],
        deadline_at_ms: Some(1_750_000_000_000),
        preparation_fingerprint: "0123456789abcdef0123456789abcdef".into(),
    });

    let rendered = Normalizer::default().normalize(&diagnostic);
    compare_or_update("headless/approval-diagnostic.txt", &format!("{rendered}\n"));
}

fn goal_projection(status: GoalStatus) -> GoalProjection {
    GoalProjection {
        id: GoalId::new("goal-fixture"),
        generation: 4,
        objective: "Finish the fixture".into(),
        status,
        token_budget: Some(100),
        usage: GoalTokenUsage {
            charged_tokens: Some(120),
            provenance: GoalUsageProvenance::ProviderReported,
            active_elapsed_ms: 25,
        },
        created_at: Timestamp(10),
        updated_at: Timestamp(20),
        stopped_reason: None,
    }
}

fn goal_terminals() {
    for (name, status) in [
        ("goal-active", GoalStatus::Active),
        ("goal-paused", GoalStatus::Paused),
        ("goal-blocked", GoalStatus::Blocked),
        ("goal-usage-limited", GoalStatus::UsageLimited),
        ("goal-budget-limited", GoalStatus::BudgetLimited),
        ("goal-complete-output", GoalStatus::Complete),
    ] {
        let goal = goal_projection(status);
        let mut result = fixture_result();
        result.status = outcome(Some(&TurnFinish::Completed), Some(&goal), None, None, None).0;
        result.goal = Some(goal);
        record_projection(name, &result, &[]);
    }
}

fn approval_exit_status() {
    let required = ApprovalRequired {
        call_id: "call-1".into(),
        tool: "edit".into(),
        argument_keys: vec!["path".into()],
        mutates: true,
        requires_authorization: true,
        permissions: vec!["fs.write".into()],
        resource: SecurityResource::filesystem("/repo", vec!["target.txt".into()]),
        authority_warnings: Vec::new(),
        deadline_at_ms: None,
        preparation_fingerprint: "0123456789abcdef0123456789abcdef".into(),
    };

    let mut result = fixture_result();
    result.status = outcome(
        Some(&TurnFinish::Completed),
        None,
        Some(&required),
        None,
        None,
    )
    .0;
    result.approval_required = Some(required.into());
    record_projection("approval-exit-status", &result, &[]);
}
