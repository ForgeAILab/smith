//! Real CLI fixtures prove that reviewer routing uses the ordinary harness.
use super::*;
use std::os::unix::fs::PermissionsExt as _;

use agent_runtime_core::cancel::{CancelReason, Cancellation};
use agent_runtime_core::clock::{Deadline, SystemClock};
use agent_runtime_core::content::Message;
use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId};
use agent_runtime_core::usage::CounterKind;

struct CliFixture {
    fixture: Fixture,
    directory: tempfile::TempDir,
}

impl CliFixture {
    fn new(kind: &str, profile: bool, events: &str, blocked: bool) -> Self {
        let directory = tempfile::tempdir().expect("CLI fixture");
        let executable = directory.path().join("advisor-cli");
        let script = format!(
            r#"#!/bin/sh
printf '%s\0' "$@" >> "$ADVISOR_ARGS"
printf '%s\0' END >> "$ADVISOR_ARGS"
pwd > "$ADVISOR_CWD"
cat "$ADVISOR_EVENTS"
printf '%s' "$$" > "$ADVISOR_STARTED"
{}
"#,
            if blocked { "exec sleep 300" } else { "exit 0" }
        );
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(directory.path().join("events"), events).unwrap();
        let id = format!("cli/{kind}/fixture-model");
        let config = if profile {
            CONFIG.replace("review-model", &id)
        } else {
            CONFIG.replace("advisor = \"review\"", &format!("advisor = \"{id}\""))
        };
        let harness = format!(
            r#"[persistence]
enabled = false
[harness.{kind}]
executable = {}
args = ["--fixture-marker"]
allow_own_tools = true
[harness.{kind}.env]
ADVISOR_ARGS = {}
ADVISOR_CWD = {}
ADVISOR_EVENTS = {}
ADVISOR_STARTED = {}
"#,
            json!(executable),
            json!(directory.path().join("args")),
            json!(directory.path().join("cwd")),
            json!(directory.path().join("events")),
            json!(directory.path().join("started")),
        );
        let fixture = Fixture::new(&config);
        std::fs::write(fixture.home.path().join(".smith/config.toml"), harness).unwrap();
        Self { fixture, directory }
    }

    fn request(&self, main: Arc<dyn Provider>, advisor: Arc<dyn Provider>) -> RuntimeRequest {
        let mut request = self.fixture.request(HostSurface::Headless, main, advisor);
        request.workspace = Some(Arc::new(LexicalWorkspace::new(
            self.fixture.project.path().to_str().unwrap(),
        )));
        request
    }

    fn context(&self) -> InvocationContext {
        InvocationContext {
            session: SessionId::new("cli-advisor"),
            turn: None,
            request: RequestId::new("request"),
            call_id: ToolCallId::new("advisor-call"),
            workspace: Arc::new(LexicalWorkspace::new(
                self.fixture.project.path().to_str().unwrap(),
            )),
            clock: Arc::new(SystemClock),
            cancel: Cancellation::new(),
            deadline: Deadline::never(),
            output_limit: 1000,
        }
    }

    fn calls(&self) -> Vec<Vec<String>> {
        let bytes = std::fs::read(self.directory.path().join("args")).expect("recorded argv");
        let args = String::from_utf8(bytes).unwrap();
        args.split("END\0")
            .filter(|call| !call.is_empty())
            .map(|call| call.split_terminator('\0').map(str::to_owned).collect())
            .collect()
    }

    async fn started(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while std::fs::read_to_string(self.directory.path().join("started"))
                .ok()
                .and_then(|value| value.parse::<i32>().ok())
                .is_none()
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("CLI starts");
    }
}

impl Drop for CliFixture {
    fn drop(&mut self) {
        // A failed cancellation assertion must not leave the fixture waiting.
        if let Some(pid) = std::fs::read_to_string(self.directory.path().join("started"))
            .ok()
            .and_then(|value| value.parse::<i32>().ok())
        {
            let pid = nix::unistd::Pid::from_raw(pid);
            let _ = nix::sys::signal::killpg(pid, nix::sys::signal::Signal::SIGKILL);
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
        }
    }
}

const CLAUDE_ADVICE: &str = r#"{"type":"system","subtype":"init","session_id":"advisor-session"}
{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"private"},{"type":"tool_use","id":"read","name":"Read","input":{}},{"type":"text","text":"检查cancellation"}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"read","content":"read evidence"}]}}
{"type":"result","usage":{"input_tokens":20,"output_tokens":5}}
"#;
const CODEX_ADVICE: &str = r#"{"type":"thread.started","thread_id":"advisor-thread"}
{"type":"item.completed","item":{"id":"read","type":"command_execution","command":"cat evidence","aggregated_output":"read evidence","exit_code":0,"status":"completed"}}
{"type":"item.completed","item":{"id":"answer","type":"agent_message","text":"检查cancellation"}}
{"type":"turn.completed","usage":{"input_tokens":20,"output_tokens":5}}
"#;

#[tokio::test]
async fn installed_advisor_uses_harness_instructions_transcript_and_fresh_read_only_calls() {
    for (kind, events) in [("claude-code", CLAUDE_ADVICE), ("codex", CODEX_ADVICE)] {
        for profile in [false, true] {
            let cli = CliFixture::new(kind, profile, events, false);
            let main = Arc::new(FakeProvider::new(
                "worker",
                Capabilities::basic_streaming(),
                vec![
                    step("first-review", "advisor", "{}"),
                    step("second-review", "advisor", "{}"),
                    ScriptedStream::new(stop_events("continued")),
                ],
            ));
            let unused = advisor_provider(Vec::new());
            let request = cli.request(main.clone(), unused.clone());
            let host = start(HostSessionRequest::new(request, cli.fixture.project.path()))
                .await
                .expect("host");
            let route = host.runtime().advisor_route().unwrap();
            assert_eq!(
                route.provider_name,
                if profile { "reviewer" } else { "local" }
            );
            assert_eq!(route.model.as_str(), format!("cli/{kind}/fixture-model"));
            assert!(route.price.is_none());
            tokio::time::timeout(
                Duration::from_secs(10),
                host.session()
                    .run(UserInput::text("USER_TASK: review cancellation")),
            )
            .await
            .expect("turn completes")
            .expect("main continues");
            assert!(
                unused.requests().is_empty(),
                "the identity provider must never be called"
            );
            assert_eq!(main.requests().len(), 3);
            let snapshot = host.session().snapshot();
            let records = snapshot
                .usage
                .records()
                .iter()
                .filter(|record| record.provenance.purpose.as_deref() == Some("advisor"))
                .collect::<Vec<_>>();
            assert_eq!(records.len(), 2);
            for record in records {
                assert_eq!(record.delta.get(CounterKind::InputUncached), 20);
                assert_eq!(record.delta.get(CounterKind::Output), 5);
                assert!(!record.provenance.failed);
                assert!(record.provenance.tool_call.is_some());
            }
            let calls = cli.calls();
            assert_eq!(calls.len(), 2);
            for args in calls {
                assert!(args.iter().any(|arg| arg == "--fixture-marker"));
                assert!(!args.iter().any(|arg| matches!(
                    arg.as_str(),
                    "--resume"
                        | "resume"
                        | "--mcp-config"
                        | "--permission-mode"
                        | "--dangerously-bypass-approvals-and-sandbox"
                )));
                let prompt = if kind == "claude-code" {
                    assert!(args.windows(2).any(|pair| pair == ["--allowedTools", ""]));
                    let instructions = &args[args
                        .iter()
                        .position(|arg| arg == "--append-system-prompt")
                        .unwrap()
                        + 1];
                    assert!(instructions.contains("You are reviewing another agent's work"));
                    assert_eq!(instructions.contains("REVIEW_PROFILE_INSTRUCTION"), profile);
                    &args[1]
                } else {
                    assert!(
                        args.windows(2)
                            .any(|pair| pair == ["--sandbox", "read-only"])
                    );
                    let prompt = args.last().unwrap();
                    assert!(prompt.starts_with("You are reviewing another agent's work"));
                    assert_eq!(prompt.contains("REVIEW_PROFILE_INSTRUCTION"), profile);
                    prompt
                };
                assert!(prompt.contains("<conversation-transcript>"));
                assert!(prompt.contains("USER_TASK: review cancellation"));
            }
            // The CLI reports its physical directory, and macOS temp roots
            // sit behind the `/var` -> `/private/var` symlink.
            assert_eq!(
                std::fs::canonicalize(
                    std::fs::read_to_string(cli.directory.path().join("cwd"))
                        .unwrap()
                        .trim()
                )
                .unwrap(),
                std::fs::canonicalize(cli.fixture.project.path()).unwrap()
            );
            host.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn installed_advisor_bounds_text_and_keeps_usage() {
    let cli = CliFixture::new("claude-code", false, CLAUDE_ADVICE, false);
    let unused = advisor_provider(Vec::new());
    let runtime = build(cli.request(unused.clone(), unused)).await.unwrap();
    let mut ctx = cli.context();
    ctx.output_limit = 3;
    let result = runtime
        .advisor_route()
        .unwrap()
        .consult(&[Message::user("review")], &ctx)
        .await;
    assert!(!result.outcome.is_error);
    assert_eq!(result.outcome.value, json!("检查c"));
    assert_eq!(result.usage.get(CounterKind::InputUncached), 20);
}

#[tokio::test]
async fn installed_advisor_failures_are_tool_errors_and_main_continues() {
    for (events, expected) in [
        (
            "{\"type\":\"result\",\"is_error\":true,\"result\":\"fixture failed\",\"usage\":{\"input_tokens\":7}}\n",
            "fixture failed",
        ),
        ("{\"type\":\"result\"}\n", "empty answer"),
        (
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"   \"}]}}\n{\"type\":\"result\"}\n",
            "empty answer",
        ),
        (
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"unfinished\"}]}}\n",
            "CLI agent exited before reporting a result",
        ),
    ] {
        let cli = CliFixture::new("claude-code", false, events, false);
        let main = Arc::new(FakeProvider::new(
            "worker",
            Capabilities::basic_streaming(),
            vec![
                step("review", "advisor", "{}"),
                ScriptedStream::new(stop_events("continued")),
            ],
        ));
        let unused = advisor_provider(Vec::new());
        let host = start(HostSessionRequest::new(
            cli.request(main, unused.clone()),
            cli.fixture.project.path(),
        ))
        .await
        .unwrap();
        host.session().run(UserInput::text("review")).await.unwrap();
        let history = host.session().history();
        let result = history
            .iter()
            .flat_map(|message| &message.content)
            .find_map(|part| match part {
                ContentPart::ToolResult(result) if result.name == "advisor" => Some(result),
                _ => None,
            })
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.content[0].as_text().unwrap().contains(expected),
            "{result:?}"
        );
        assert_eq!(history.last().unwrap().joined_text(), "continued");
        assert!(unused.requests().is_empty());
        if expected == "fixture failed" {
            let snapshot = host.session().snapshot();
            let record = snapshot
                .usage
                .records()
                .iter()
                .find(|record| record.provenance.purpose.as_deref() == Some("advisor"))
                .unwrap();
            assert_eq!(record.delta.get(CounterKind::InputUncached), 7);
            assert!(record.provenance.failed);
        }
        host.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn installed_advisor_missing_program_defers_failure_until_consult() {
    let cli = CliFixture::new("claude-code", false, "", false);
    let unused = advisor_provider(Vec::new());
    let mut request = cli.request(unused.clone(), unused.clone());
    let harness = request
        .advisor_profile
        .as_mut()
        .unwrap()
        .config
        .harness
        .as_mut()
        .unwrap();
    harness.executable = None;
    harness.program = "smith-no-such-advisor-program".to_owned();
    let runtime = build(request)
        .await
        .expect("a missing program does not block composition");
    let response = runtime
        .advisor_route()
        .unwrap()
        .consult(&[Message::user("review")], &cli.context())
        .await;
    assert!(response.outcome.is_error);
    let text = response.outcome.value.as_str().unwrap();
    assert!(
        text.contains("smith-no-such-advisor-program") && text.contains("not on PATH"),
        "{text}"
    );
    assert!(unused.requests().is_empty());
}

#[tokio::test]
async fn installed_advisor_interrupt_deadline_and_dropped_future_stop_the_process() {
    for stop in ["interrupt", "deadline", "drop"] {
        let cli = CliFixture::new(
            "claude-code",
            false,
            "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"blocked\"}\n",
            true,
        );
        let unused = advisor_provider(Vec::new());
        let runtime = build(cli.request(unused.clone(), unused)).await.unwrap();
        let mut ctx = cli.context();
        if stop == "deadline" {
            ctx.deadline = Deadline::after(ctx.clock.as_ref(), 500);
        }
        let cancel = ctx.cancel.clone();
        let review = tokio::spawn(async move {
            runtime
                .advisor_route()
                .unwrap()
                .consult(&[Message::user("review")], &ctx)
                .await
        });
        cli.started().await;
        let pid: i32 = std::fs::read_to_string(cli.directory.path().join("started"))
            .unwrap()
            .parse()
            .unwrap();
        match stop {
            "interrupt" => cancel.cancel(CancelReason::UserRequested),
            "drop" => review.abort(),
            _ => {}
        }
        let response = tokio::time::timeout(Duration::from_secs(5), review)
            .await
            .expect("review stops");
        if stop == "drop" {
            assert!(response.unwrap_err().is_cancelled());
        } else {
            let response = response.unwrap();
            assert!(response.outcome.is_error);
            assert!(
                response
                    .outcome
                    .value
                    .as_str()
                    .unwrap()
                    .contains(if stop == "interrupt" {
                        "cancelled"
                    } else {
                        "timed out"
                    })
            );
        }
        assert_eq!(cancel.is_cancelled(), stop == "interrupt");
        tokio::time::timeout(Duration::from_secs(5), async {
            while nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None)
                != Err(nix::errno::Errno::ESRCH)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("CLI is reaped");
    }
}
