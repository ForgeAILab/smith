//! Contracts for the scripted executable, independent of Smith and the relay.

use std::process::Command;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use super::support::{
    Deadline, Process, StandInRun, run_stand_in, script_command, socket_directory, stand_in_path,
};

const RESUME_REJECTED: &str = include_str!(
    "../../../smith-runtime/tests/fixtures/cli_bridge/codex-0.162.1/stderr/exec-flags-after-resume-rejected.txt"
);

struct Fixture {
    directory: tempfile::TempDir,
    log: std::path::PathBuf,
    deadline: Deadline,
}

impl Fixture {
    fn new() -> Self {
        let deadline = Deadline::after(Duration::from_secs(15));
        let directory = socket_directory();
        let log = directory.path().join("calls.jsonl");
        // An empty file distinguishes no invocation from a missing assertion.
        std::fs::write(&log, "").expect("empty counting log");
        Self {
            directory,
            log,
            deadline,
        }
    }

    fn args(&self, codex: bool, allowed: bool, resume: Option<&str>) -> Vec<String> {
        let executable = stand_in_path().display().to_string();
        self.args_for(codex, allowed, resume, &executable)
    }

    fn args_for(
        &self,
        codex: bool,
        allowed: bool,
        resume: Option<&str>,
        executable: &str,
    ) -> Vec<String> {
        let server_args = json!(["--counting-mcp-server", "--log", self.log]);
        if codex {
            let mut args = vec![
                "exec".into(),
                "--json".into(),
                "--skip-git-repo-check".into(),
                "-C".into(),
                self.directory.path().display().to_string(),
                "--sandbox".into(),
                "read-only".into(),
                "-m".into(),
                "gpt-6-sol".into(),
            ];
            for value in [
                format!("mcp_servers.smith.command={}", json!(executable)),
                format!("mcp_servers.smith.args={server_args}"),
            ] {
                args.extend(["-c".into(), value]);
            }
            if allowed {
                args.extend([
                    "-c".into(),
                    "mcp_servers.smith.default_tools_approval_mode=\"approve\"".into(),
                ]);
            }
            args.extend([
                "-c".into(),
                "mcp_servers.smith.tool_timeout_sec=604800".into(),
                "-c".into(),
                "mcp_servers.smith.required=true".into(),
                "-c".into(),
                "mcp_servers.smith.startup_timeout_sec=60".into(),
            ]);
            if let Some(id) = resume {
                args.extend(["resume".into(), id.into()]);
            }
            args.push("the prompt".into());
            args
        } else {
            let mut args = vec![
                "-p".into(),
                "the prompt".into(),
                "--output-format".into(),
                "stream-json".into(),
                "--verbose".into(),
            ];
            if allowed {
                args.extend(["--allowedTools".into(), "mcp__smith".into()]);
            }
            args.extend([
                "--model".into(),
                "sonnet".into(),
                "--mcp-config".into(),
                json!({"mcpServers":{"smith":{"command":executable,"args":server_args}}})
                    .to_string(),
            ]);
            if let Some(id) = resume {
                args.extend(["--resume".into(), id.into()]);
            }
            args
        }
    }

    fn run(&self, args: &[String], script: Value) -> StandInRun {
        run_stand_in(self.directory.path(), args, &script, self.deadline)
    }

    fn log(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .expect("counting log")
            .lines()
            .map(|line| serde_json::from_str(line).expect("counting arguments"))
            .collect()
    }
}

fn successful(run: &StandInRun, codex: bool) {
    assert!(run.output.status.success(), "{:?}", run.output);
    assert!(run.output.stderr.is_empty(), "{:?}", run.output);
    let terminal = run.lines.last().expect("terminal event");
    assert_eq!(
        terminal["type"],
        if codex { "turn.completed" } else { "result" }
    );
    if !codex {
        assert_eq!(terminal["is_error"], false);
    }
    assert!(terminal["usage"].is_object());
}

fn sent_calls(record: &Value) -> Vec<&Value> {
    record["wire"]
        .as_array()
        .expect("wire record")
        .iter()
        .filter(|event| event["direction"] == "sent" && event["frame"]["method"] == "tools/call")
        .collect()
}

fn happy_path(codex: bool) {
    let fixture = Fixture::new();
    let args = fixture.args(codex, true, None);
    let arguments = json!({"marker":"happy","nested":{"n":7}});
    let run = fixture.run(
        &args,
        json!([
            {"say":"before"}, {"call":{"tool":"count","arguments":arguments}}, {"say":"after"}
        ]),
    );
    successful(&run, codex);
    assert_eq!(
        run.record["dialect"],
        if codex { "codex" } else { "claude-code" }
    );
    assert_eq!(
        run.record["argv"].as_array().expect("argv")[1..],
        args.iter().map(|arg| json!(arg)).collect::<Vec<_>>()
    );
    assert_eq!(run.record["server"]["name"], "smith");
    assert_eq!(
        run.record["server"]["command"],
        stand_in_path().display().to_string()
    );
    assert_eq!(
        run.record["server"]["args"],
        json!(["--counting-mcp-server", "--log", fixture.log])
    );
    assert_eq!(run.record["prompt"], "the prompt");
    // current_dir() resolves symlinks such as macOS's /tmp -> /private/tmp.
    assert_eq!(
        run.record["cwd"],
        fixture
            .directory
            .path()
            .canonicalize()
            .expect("canonical working directory")
            .display()
            .to_string()
    );
    assert_eq!(run.record["environment"]["MCP_TOOL_TIMEOUT"], "604800000");
    assert_eq!(run.record["environment"]["MCP_TIMEOUT"], "60000");
    assert!(run.record["environment"]["CLI_STAND_IN_SCRIPT"].is_string());
    assert_eq!(run.record["session_source"], "fresh");
    assert!(
        run.record["instructions"]
            .as_str()
            .expect("instructions")
            .contains("append-only")
    );
    assert_eq!(run.record["tool_names"], json!(["count", "slow_count"]));
    assert!(
        run.record["steps"]
            .as_array()
            .expect("steps")
            .iter()
            .all(|step| step["outcome"] == "completed")
    );
    assert_eq!(fixture.log(), vec![arguments.clone()]);
    let wire = run.record["wire"].as_array().expect("wire");
    assert_eq!(
        wire[0]["frame"]["method"],
        if codex {
            "initialize"
        } else {
            "server/discover"
        }
    );
    let initialize = wire
        .iter()
        .find(|event| event["frame"]["method"] == "initialize")
        .expect("initialize");
    assert_eq!(
        initialize["frame"]["params"]["protocolVersion"],
        if codex { "2025-06-18" } else { "2025-11-25" }
    );
    assert_eq!(sent_calls(&run.record).len(), 1);
    let calls = sent_calls(&run.record);
    let meta = &calls[0]["frame"]["params"]["_meta"];
    assert!(meta["progressToken"].is_number());
    assert!(
        meta[if codex {
            "callId"
        } else {
            "claudecode/toolUseId"
        }]
        .is_string()
    );
    if codex {
        assert_eq!(run.lines[0]["type"], "thread.started");
        assert_eq!(run.record["flags"]["--sandbox"], "read-only");
        assert_eq!(
            run.record["flags"]["-C"],
            fixture.directory.path().display().to_string()
        );
        assert_eq!(run.record["flags"]["-m"], "gpt-6-sol");
        assert_eq!(run.record["server"]["tool_timeout_sec"], 604800);
        assert_eq!(run.record["server"]["required"], true);
        assert_eq!(run.record["server"]["startup_timeout_sec"], 60);
        assert_eq!(
            run.record["server"]["default_tools_approval_mode"],
            "approve"
        );
        let start = run
            .lines
            .iter()
            .find(|v| v["type"] == "item.started" && v["item"]["type"] == "mcp_tool_call")
            .expect("echo start");
        let end = run
            .lines
            .iter()
            .find(|v| v["type"] == "item.completed" && v["item"]["id"] == start["item"]["id"])
            .expect("echo end");
        assert_eq!(start["item"]["server"], "smith");
        assert_eq!(start["item"]["tool"], "count");
        assert_eq!(end["item"]["arguments"], arguments);
        assert_eq!(end["item"]["result"]["content"][0]["text"], "counted:1");
    } else {
        assert_eq!(run.lines[0]["subtype"], "init");
        assert_eq!(run.record["flags"]["--allowedTools"], "mcp__smith");
        assert_eq!(run.record["flags"]["--model"], "sonnet");
        assert_eq!(run.record["handshake_path"], "initialize");
        let start = run
            .lines
            .iter()
            .find(|v| v["message"]["content"][0]["name"] == "mcp__smith__count")
            .expect("tool use");
        let end = run
            .lines
            .iter()
            .find(|v| {
                v["message"]["content"][0]["tool_use_id"] == start["message"]["content"][0]["id"]
            })
            .expect("tool result");
        assert_eq!(start["message"]["content"][0]["input"], arguments);
        assert_eq!(
            end["message"]["content"][0]["content"][0]["text"],
            "counted:1"
        );
    }
}

#[test]
fn claude_happy_path_uses_the_recorded_wire_and_stream() {
    happy_path(false);
}

#[test]
fn codex_happy_path_uses_the_recorded_wire_and_stream() {
    happy_path(true);
}

#[test]
fn resumed_turns_reuse_the_given_identity() {
    for codex in [false, true] {
        let fixture = Fixture::new();
        let run = fixture.run(
            &fixture.args(codex, true, Some("saved-session")),
            json!([
                {"call":{"tool":"count","arguments":{"resumed":true}}}
            ]),
        );
        successful(&run, codex);
        assert_eq!(run.record["resume"], "saved-session");
        assert_eq!(run.record["session_source"], "resume");
        assert_eq!(
            run.lines[0][if codex { "thread_id" } else { "session_id" }],
            "saved-session"
        );
        assert_eq!(fixture.log(), vec![json!({"resumed":true})]);
    }
}

#[test]
fn codex_rejects_exec_flags_after_resume() {
    for flag in ["-C", "--sandbox", "-c"] {
        let fixture = Fixture::new();
        let run = fixture.run(
            &[
                "exec".into(),
                "resume".into(),
                "saved".into(),
                flag.into(),
                "value".into(),
            ],
            json!([]),
        );
        assert_eq!(run.output.status.code(), Some(2));
        assert_eq!(String::from_utf8_lossy(&run.output.stderr), RESUME_REJECTED);
        assert!(run.lines.is_empty());
        assert!(run.record["wire"].as_array().expect("wire").is_empty());
        assert_eq!(fixture.log().len(), 0);
    }
}

#[test]
fn missing_allow_rules_refuse_calls_without_invoking_the_tool() {
    for codex in [false, true] {
        let fixture = Fixture::new();
        let run = fixture.run(
            &fixture.args(codex, false, None),
            json!([
                {"call":{"tool":"count","arguments":{"must_not_run":true}}}
            ]),
        );
        successful(&run, codex);
        assert!(sent_calls(&run.record).is_empty());
        assert!(fixture.log().is_empty());
        if codex {
            let refused = run
                .lines
                .iter()
                .find(|v| v["item"]["type"] == "mcp_tool_call" && v["item"]["status"] == "failed")
                .expect("refusal");
            assert_eq!(
                refused["item"]["error"]["message"],
                "MCP tool call requires approval, but approval policy is never"
            );
        } else {
            let refused = run
                .lines
                .iter()
                .find(|v| v["subtype"] == "permission_denied")
                .expect("refusal");
            assert_eq!(refused["tool_name"], "mcp__smith__count");
            let result = run
                .lines
                .iter()
                .find(|v| v["message"]["content"][0]["tool_use_id"] == refused["tool_use_id"])
                .expect("refused result");
            assert_eq!(result["message"]["content"][0]["is_error"], true);
        }
    }
}

#[test]
fn together_sends_both_calls_before_reading_either_result() {
    for codex in [false, true] {
        let fixture = Fixture::new();
        let first = json!({"seconds":0.05,"marker":"first"});
        let second = json!({"seconds":0.01,"marker":"second"});
        let run = fixture.run(&fixture.args(codex, true, None), json!([
            {"call_together":[{"tool":"slow_count","arguments":first},{"tool":"slow_count","arguments":second}]}
        ]));
        successful(&run, codex);
        let calls = sent_calls(&run.record);
        assert_eq!(calls.len(), 2);
        let wire = run.record["wire"].as_array().expect("wire");
        let received = wire
            .iter()
            .find(|event| {
                event["direction"] == "received"
                    && calls
                        .iter()
                        .any(|call| event["frame"]["id"] == call["frame"]["id"])
            })
            .expect("first call response");
        for call in calls {
            assert!(call["sequence"].as_u64() < received["sequence"].as_u64());
        }
        assert_eq!(fixture.log(), vec![first, second]);
    }
}

#[test]
fn abandon_records_cancellation_or_stdin_close_with_the_outstanding_id() {
    for codex in [false, true] {
        for how in ["cancel", "close"] {
            let fixture = Fixture::new();
            let run = fixture.run(&fixture.args(codex, true, None), json!([
                {"abandon":{"tool":"slow_count","arguments":{"seconds":60},"after_ms":10,"how":how}}
            ]));
            successful(&run, codex);
            let calls = sent_calls(&run.record);
            assert_eq!(calls.len(), 1);
            assert!(fixture.log().is_empty());
            let id = &calls[0]["frame"]["id"];
            let wire = run.record["wire"].as_array().expect("wire");
            assert!(
                !wire
                    .iter()
                    .any(|event| event["direction"] == "received" && event["frame"]["id"] == *id)
            );
            if how == "cancel" {
                let cancel = wire
                    .iter()
                    .find(|event| event["frame"]["method"] == "notifications/cancelled")
                    .expect("cancellation");
                assert_eq!(cancel["direction"], "sent");
                assert_eq!(cancel["frame"]["params"]["requestId"], *id);
                assert_eq!(
                    cancel["frame"]["params"]["reason"],
                    "SdkError: Request timed out"
                );
            } else {
                let close = wire
                    .iter()
                    .find(|event| event["direction"] == "stdin_closed")
                    .expect("closed stdin");
                assert_eq!(close["frame"]["outstanding_request_id"], *id);
                assert!(close["sequence"].as_u64() > calls[0]["sequence"].as_u64());
            }
            if codex {
                assert!(run.lines.iter().any(|v| {
                    v["item"]["status"] == "failed"
                        && v["item"]["error"]["message"]
                            .as_str()
                            .is_some_and(|text| text.contains("timed out"))
                }));
            } else {
                assert!(run.lines.iter().any(|v| {
                    v["message"]["content"][0]["is_error"] == true
                        && v["message"]["content"][0]["content"]
                            .as_str()
                            .is_some_and(|text| text.contains("timed out"))
                }));
            }
        }
    }
}

#[test]
fn exiting_with_an_outstanding_call_prints_no_terminal_event() {
    for codex in [false, true] {
        let fixture = Fixture::new();
        let run = fixture.run(
            &fixture.args(codex, true, None),
            json!([
                {"call_then_exit":{"tool":"slow_count","arguments":{"seconds":60},"code":7}}
            ]),
        );
        assert_eq!(run.output.status.code(), Some(7));
        assert_eq!(sent_calls(&run.record).len(), 1);
        assert!(fixture.log().is_empty());
        assert!(!run.lines.iter().any(|v| matches!(
            v["type"].as_str(),
            Some("result" | "turn.completed" | "turn.failed")
        )));
        assert_eq!(
            run.record["steps"][0]["outcome"],
            "exited_with_call_outstanding"
        );
    }
}

#[test]
fn plain_exit_has_the_requested_status_without_a_terminal_event() {
    let fixture = Fixture::new();
    let run = fixture.run(
        &fixture.args(false, true, None),
        json!([{"exit":{"code":9}}]),
    );
    assert_eq!(run.output.status.code(), Some(9));
    assert_eq!(run.lines.len(), 1);
    assert_eq!(run.record["server_spawned"], false);
}

#[test]
fn text_only_scripts_never_spawn_the_declared_command() {
    for codex in [false, true] {
        let fixture = Fixture::new();
        let run = fixture.run(
            &fixture.args_for(codex, true, None, "/does-not-exist/stand-in-server"),
            json!([
                {"say":"one"}, {"say":"two"}
            ]),
        );
        successful(&run, codex);
        assert_eq!(run.record["server_spawned"], false);
        assert!(run.record["wire"].as_array().expect("wire").is_empty());
        assert!(fixture.log().is_empty());
    }
}

#[test]
fn codex_required_server_never_starts_fails_before_stdout() {
    for (server, timeout) in [("smith", 60), ("other_server", 17)] {
        let fixture = Fixture::new();
        let args = fixture
            .args(true, true, None)
            .into_iter()
            .map(|arg| {
                arg.replace("mcp_servers.smith.", &format!("mcp_servers.{server}."))
                    .replace(
                        "startup_timeout_sec=60",
                        &format!("startup_timeout_sec={timeout}"),
                    )
            })
            .collect::<Vec<_>>();
        let run = fixture.run(
            &args,
            json!([
                {"server_never_starts":true},
                {"call":{"tool":"count","arguments":{"must_not_run":true}}},
                {"say":"must not print"}
            ]),
        );
        assert_eq!(run.output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8_lossy(&run.output.stderr),
            format!(
                "Error: thread/start: thread/start failed: error creating thread: Fatal error: Failed to initialize session: required MCP servers failed to initialize: {server}: timed out handshaking with MCP server after {timeout}s (code -32603)\n"
            )
        );
        assert!(run.output.stdout.is_empty());
        assert!(run.lines.is_empty());
        assert_eq!(run.record["server"]["required"], true);
        assert_eq!(run.record["server"]["startup_timeout_sec"], timeout);
        assert_eq!(run.record["server_spawned"], false);
        assert!(run.record["wire"].as_array().expect("wire").is_empty());
        assert!(fixture.log().is_empty());
        assert_eq!(run.record["steps"].as_array().expect("steps").len(), 1);
        assert_eq!(run.record["steps"][0]["outcome"], "required_server_failed");
    }
}

fn unavailable_server_completes_without_spawning(
    fixture: &Fixture,
    args: &[String],
    codex: bool,
) -> StandInRun {
    let run = fixture.run(
        args,
        json!([
            {"server_never_starts":true},
            {"list_tools":true},
            {"call":{"tool":"count","arguments":{"must_not_run":true}}},
            {"call_together":[{"tool":"count","arguments":{}}]}
        ]),
    );
    successful(&run, codex);
    assert_eq!(run.record["server_spawned"], false);
    assert!(run.record["wire"].as_array().expect("wire").is_empty());
    assert!(fixture.log().is_empty());
    assert_eq!(run.record["tool_names"], json!([]));
    assert_eq!(
        run.lines
            .iter()
            .filter(|event| {
                if codex {
                    event["item"]["type"] == "agent_message"
                        && event["item"]["text"] == "UNAVAILABLE"
                } else {
                    event["type"] == "assistant"
                        && event["message"]["content"][0]["type"] == "text"
                        && event["message"]["content"][0]["text"] == "UNAVAILABLE"
                }
            })
            .count(),
        2
    );
    if !codex {
        assert_eq!(run.lines[0]["mcp_servers"], json!([]));
        assert_eq!(run.lines.last().expect("result")["result"], "UNAVAILABLE");
    }
    run
}

#[test]
fn codex_optional_server_never_starts_continues_without_spawning() {
    for required in [None, Some(false)] {
        let fixture = Fixture::new();
        let mut args = fixture.args(true, true, None);
        let index = args
            .iter()
            .position(|arg| arg == "mcp_servers.smith.required=true")
            .expect("required override");
        if let Some(required) = required {
            args[index] = format!("mcp_servers.smith.required={required}");
        } else {
            args.drain(index - 1..=index);
        }
        let run = unavailable_server_completes_without_spawning(&fixture, &args, true);
        assert_eq!(run.record["server"]["required"], json!(required));
        assert_eq!(run.record["server"]["startup_timeout_sec"], 60);
    }
}

#[test]
fn claude_server_never_starts_continues_without_spawning() {
    let fixture = Fixture::new();
    unavailable_server_completes_without_spawning(
        &fixture,
        &fixture.args(false, true, None),
        false,
    );
}

#[test]
fn failures_have_the_fields_smiths_normalizers_read() {
    for codex in [false, true] {
        let fixture = Fixture::new();
        let run = fixture.run(
            &fixture.args(codex, true, None),
            json!([{"fail":"scripted failure"}]),
        );
        assert_eq!(run.output.status.code(), Some(1));
        let terminal = run.lines.last().expect("terminal failure");
        if codex {
            assert_eq!(terminal["type"], "turn.failed");
            assert_eq!(terminal["error"]["message"], "scripted failure");
            assert!(run.lines.iter().any(|v| v["type"] == "error"));
        } else {
            assert_eq!(terminal["type"], "result");
            assert_eq!(terminal["subtype"], "success");
            assert_eq!(terminal["is_error"], true);
            assert_eq!(terminal["result"], "scripted failure");
        }
    }
}

#[test]
fn repeated_lists_are_wire_noops_and_own_tools_keep_recorded_shapes() {
    for codex in [false, true] {
        let fixture = Fixture::new();
        let run = fixture.run(
            &fixture.args(codex, true, None),
            json!([
                {"list_tools":true},{"list_tools":true},{"own_tool":{"name":"fixture shell"}}
            ]),
        );
        successful(&run, codex);
        assert_eq!(
            run.record["wire"]
                .as_array()
                .expect("wire")
                .iter()
                .filter(|event| event["direction"] == "sent"
                    && event["frame"]["method"] == "tools/list")
                .count(),
            1
        );
        assert!(fixture.log().is_empty());
        if codex {
            let items: Vec<_> = run
                .lines
                .iter()
                .filter(|v| v["item"]["type"] == "command_execution")
                .collect();
            assert_eq!(items.len(), 2);
            assert_eq!(items[0]["item"]["id"], items[1]["item"]["id"]);
            assert_eq!(items[1]["item"]["exit_code"], 0);
        } else {
            let tool = run
                .lines
                .iter()
                .find(|v| v["message"]["content"][0]["name"] == "Bash")
                .expect("Bash");
            assert!(
                run.lines
                    .iter()
                    .any(|v| v["message"]["content"][0]["tool_use_id"]
                        == tool["message"]["content"][0]["id"])
            );
        }
    }
}

fn process_field(pid: u32, field: &str, deadline: Deadline) -> String {
    let field = format!("{field}=");
    let output = deadline.work("inspecting process group and liveness", move || {
        super::support::run_command(
            Command::new("/bin/ps").args(["-o", &field, "-p", &pid.to_string()]),
            deadline,
        )
    });
    String::from_utf8(output.stdout)
        .expect("ps text")
        .trim()
        .to_owned()
}

fn live(pid: u32, deadline: Deadline) -> bool {
    let status = process_field(pid, "stat", deadline);
    // A killed orphan can briefly await reaping on Linux; a zombie executes
    // nothing and holds no pipes, so it is no longer a live group member.
    !status.is_empty() && !status.starts_with('Z')
}

#[test]
fn spawned_child_is_live_in_the_same_group_and_dies_with_it() {
    let fixture = Fixture::new();
    let pid_file = fixture.directory.path().join("pids.json");
    let (mut command, _) = script_command(
        fixture.directory.path(),
        &fixture.args(true, true, None),
        &json!([
            {"spawn_child":{"seconds":60,"pid_file":pid_file}}, {"sleep_ms":60000}
        ]),
    );
    let process = Process::spawn(&mut command, fixture.deadline);
    let pids: Value = loop {
        fixture.deadline.remaining("spawn_child pid file");
        if let Ok(bytes) = std::fs::read(&pid_file)
            && let Ok(value) = serde_json::from_slice(&bytes)
        {
            break value;
        }
        thread::sleep(Duration::from_millis(5));
    };
    let parent =
        u32::try_from(pids["stand_in_pid"].as_u64().expect("stand-in pid")).expect("pid fits");
    let child = u32::try_from(pids["child_pid"].as_u64().expect("child pid")).expect("pid fits");
    assert_eq!(parent, process.id());
    assert!(live(parent, fixture.deadline));
    assert!(live(child, fixture.deadline));
    assert_eq!(
        process_field(parent, "pgid", fixture.deadline),
        parent.to_string()
    );
    assert_eq!(
        process_field(child, "pgid", fixture.deadline),
        parent.to_string()
    );
    process.kill_group();
    let output = process.finish(None);
    assert!(!output.status.success());
    while live(parent, fixture.deadline) || live(child, fixture.deadline) {
        fixture
            .deadline
            .remaining("both process group members stopped");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn counting_server_serves_mcp_and_logs_exact_arguments() {
    let fixture = Fixture::new();
    let mut command = Command::new(stand_in_path());
    command
        .arg("--counting-mcp-server")
        .arg("--log")
        .arg(&fixture.log);
    let mut server = Process::spawn(&mut command, fixture.deadline);
    server.send(&json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}), "counting initialize write");
    let initialize = server.next("counting initialize response");
    assert_eq!(initialize["result"]["protocolVersion"], "2025-06-18");
    assert!(initialize["result"]["instructions"].is_string());
    server.send(
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        "initialized notification",
    );
    server.send(
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        "counting list write",
    );
    let list = server.next("counting list response");
    assert_eq!(
        list["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .map(|v| v["name"].clone())
            .collect::<Vec<_>>(),
        vec![json!("count"), json!("slow_count")]
    );
    for n in 1..=3 {
        server.send(&json!({"jsonrpc":"2.0","id":n+1,"method":"tools/call","params":{"name":"count","arguments":{"n":n}}}), "counting call write");
        let response = server.next(&format!("counting call {n} response"));
        assert_eq!(response["id"], n + 1);
        assert_eq!(
            response["result"]["content"][0]["text"],
            format!("counted:{n}")
        );
    }
    server.send(
        &json!({"jsonrpc":"2.0","id":5,"method":"unknown"}),
        "unknown method write",
    );
    assert_eq!(
        server.next("unknown method response")["error"]["code"],
        -32601
    );
    server.send(
        &json!({"jsonrpc":"2.0","id":6,"method":"ping"}),
        "ping write",
    );
    assert_eq!(server.next("ping response")["result"], json!({}));
    server.close_stdin();
    assert!(server.finish(None).status.success());
    assert_eq!(
        fixture.log(),
        vec![json!({"n":1}), json!({"n":2}), json!({"n":3})]
    );
}

#[test]
fn malformed_declarations_fail_before_stdout_or_wire_activity() {
    for (args, argument) in [
        (vec!["-p", "hello", "--mcp-config", "{bad"], "--mcp-config"),
        (
            vec![
                "exec",
                "-c",
                "mcp_servers.smith.args=[\"unterminated]",
                "hello",
            ],
            "-c",
        ),
        (
            vec!["exec", "-c", "mcp_servers.smith.command=42", "hello"],
            "-c",
        ),
        (
            vec!["exec", "-c", "mcp_servers.smith.args=[1]", "hello"],
            "-c",
        ),
        (
            vec!["exec", "-c", "mcp_servers.smith.required=1", "hello"],
            "-c",
        ),
        (
            vec!["exec", "-c", "mcp_servers.smith.required=\"true\"", "hello"],
            "-c",
        ),
        (
            vec![
                "exec",
                "-c",
                "mcp_servers.smith.startup_timeout_sec=1.5",
                "hello",
            ],
            "-c",
        ),
    ] {
        let fixture = Fixture::new();
        let args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
        let run = fixture.run(&args, json!([{"call":{"tool":"count","arguments":{}}}]));
        assert_eq!(run.output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&run.output.stderr).contains(argument));
        assert!(run.lines.is_empty());
        assert_eq!(run.record["server_spawned"], false);
        assert!(run.record["wire"].as_array().expect("wire").is_empty());
        assert!(fixture.log().is_empty());
    }
}

#[test]
fn unrecognized_dialect_is_a_usage_error() {
    let fixture = Fixture::new();
    let run = fixture.run(&["unknown".into()], json!([]));
    assert_eq!(run.output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&run.output.stderr).contains("expected exec"));
    assert!(run.lines.is_empty());
}

#[test]
fn declarations_preserve_escaped_paths_and_basic_string_arrays() {
    let fixture = Fixture::new();
    let executable = "/missing/path with spaces \"quotes\" and \\slash";
    for codex in [false, true] {
        let args = fixture.args_for(codex, true, None, executable);
        let run = fixture.run(&args, json!([{"say":"parsed only"}]));
        successful(&run, codex);
        assert_eq!(run.record["server"]["command"], executable);
    }
}

#[test]
fn tool_names_pass_unchanged_on_the_wire_and_normalize_only_in_claudes_echo() {
    for codex in [false, true] {
        let fixture = Fixture::new();
        let tool = "registry.search/雪";
        let run = fixture.run(
            &fixture.args(codex, true, None),
            json!([
                {"call":{"tool":tool,"arguments":{}}}
            ]),
        );
        successful(&run, codex);
        assert_eq!(sent_calls(&run.record)[0]["frame"]["params"]["name"], tool);
        assert!(fixture.log().is_empty());
        if codex {
            assert!(
                run.lines
                    .iter()
                    .any(|v| v["item"]["tool"] == tool && v["item"]["status"] == "failed")
            );
        } else {
            assert!(
                run.lines
                    .iter()
                    .any(|v| v["message"]["content"][0]["name"] == "mcp__smith__registry_search__")
            );
            assert!(
                run.lines
                    .iter()
                    .any(|v| v["message"]["content"][0]["is_error"] == true)
            );
        }
    }
}

#[test]
fn last_result_pointers_drive_later_calls_and_say_last_uses_received_content() {
    for codex in [false, true] {
        let fixture = Fixture::new();
        let run = fixture.run(&fixture.args(codex, true, None), json!([
            {"call":{"tool":"count","arguments":{"output":"{\"spawned\":\"child-from-response\"}"}}},
            {"say_last":true},
            {"call":{"tool":"count","arguments":{"child_id":{"$last":"/spawned"}}}}
        ]));
        successful(&run, codex);
        assert_eq!(
            fixture.log(),
            [
                json!({"output":"{\"spawned\":\"child-from-response\"}"}),
                json!({"child_id":"child-from-response"})
            ]
        );
        let calls = sent_calls(&run.record);
        assert_eq!(
            calls[1]["frame"]["params"]["arguments"]["child_id"],
            "child-from-response"
        );
        assert!(run.lines.iter().any(|line| if codex {
            line["item"]["type"] == "agent_message"
                && line["item"]["text"] == "{\"spawned\":\"child-from-response\"}"
        } else {
            line["message"]["content"][0]["type"] == "text"
                && line["message"]["content"][0]["text"] == "{\"spawned\":\"child-from-response\"}"
        }));
    }
}

#[test]
fn scripts_select_fresh_resumed_and_child_prompt_flows_and_archive_invocations() {
    for codex in [false, true] {
        for (resume, by_prompt, expected) in [
            (None, false, "fresh-script"),
            (
                Some("12345678-1234-1234-1234-123456789012"),
                false,
                "resume-script",
            ),
            (None, true, "child-script"),
        ] {
            let fixture = Fixture::new();
            let mut script = json!({"initial":[{"say":"fresh-script"}],
                "resumed":[{"say":"resume-script"}]});
            if by_prompt {
                script["by_prompt"] = json!({"the prompt":[{"say":"child-script"}]});
            }
            let run = fixture.run(&fixture.args(codex, true, resume), script);
            successful(&run, codex);
            assert_eq!(run.record["steps"][0]["step"]["say"], expected);
            assert_eq!(run.invocations().len(), 1);
            assert_eq!(run.invocations()[0]["argv"], run.record["argv"]);
            assert!(run.record["pid"].as_u64().is_some());
            assert!(
                run.record["started"]
                    .as_str()
                    .unwrap()
                    .parse::<u128>()
                    .unwrap()
                    > 0
            );
            assert_eq!(run.record["environment"]["PATH"], "/usr/bin:/bin");
        }
    }
}

#[test]
fn together_cancel_targets_the_second_outstanding_request_before_reading() {
    for codex in [false, true] {
        let fixture = Fixture::new();
        let run = fixture.run(&fixture.args(codex, true, None), json!([
            {"together_cancel":{"calls":[{"tool":"slow_count","arguments":{"seconds":0.2,"n":1}},
                {"tool":"slow_count","arguments":{"seconds":60,"n":2}}],"cancel_index":1,"after_ms":10}},
            {"say":"continued"}
        ]));
        successful(&run, codex);
        let calls = sent_calls(&run.record);
        assert_eq!(calls.len(), 2);
        let wire = run.record["wire"].as_array().unwrap();
        let cancel = wire
            .iter()
            .find(|entry| entry["frame"]["method"] == "notifications/cancelled")
            .unwrap();
        assert_eq!(
            cancel["frame"]["params"]["requestId"],
            calls[1]["frame"]["id"]
        );
        let received = wire
            .iter()
            .find(|entry| {
                entry["direction"] == "received" && entry["frame"]["id"] == calls[0]["frame"]["id"]
            })
            .unwrap();
        assert!(cancel["sequence"].as_u64().unwrap() < received["sequence"].as_u64().unwrap());
        assert!(wire.iter().all(|entry| entry["direction"] != "received"
            || entry["frame"]["id"] != calls[1]["frame"]["id"]));
        assert_eq!(fixture.log(), [json!({"seconds":0.2,"n":1})]);
    }
}

#[test]
fn wait_file_is_a_barrier_before_later_script_effects() {
    let fixture = Fixture::new();
    let release = fixture.directory.path().join("release");
    let (mut command, record) = script_command(
        fixture.directory.path(),
        &fixture.args(false, true, None),
        &json!([
            {"wait_file":release}, {"call":{"tool":"count","arguments":{"released":true}}}
        ]),
    );
    let process = Process::spawn(&mut command, fixture.deadline);
    loop {
        fixture.deadline.remaining("wait_file entered");
        if let Ok(bytes) = std::fs::read(&record)
            && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
            && value["steps"][0]["outcome"] == "running"
        {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(fixture.log().is_empty());
    std::fs::write(release, "continue").unwrap();
    let run = super::support::recorded_run(process.finish(Some(&record)), &record);
    successful(&run, false);
    assert_eq!(fixture.log(), [json!({"released":true})]);
}

#[test]
fn abandonment_and_exit_can_wait_for_execution_evidence() {
    for exit in [false, true] {
        let fixture = Fixture::new();
        let started = fixture.log.with_extension("started.jsonl");
        let step = if exit {
            json!({"call_then_exit":{"tool":"slow_count","arguments":{"seconds":60},"code":7,
                "started_file":started}})
        } else {
            json!({"abandon":{"tool":"slow_count","arguments":{"seconds":60},"how":"cancel","after_ms":0,
                "started_file":started}})
        };
        let run = fixture.run(&fixture.args(false, true, None), json!([step]));
        assert_eq!(run.output.status.code(), Some(if exit { 7 } else { 0 }));
        assert!(fixture.log().is_empty());
        let entered = std::fs::read_to_string(started).unwrap();
        let entries = entered
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["tool"], "slow_count");
        assert_eq!(entries[0]["arguments"], json!({"seconds":60}));
        if !exit {
            assert!(
                run.record["wire"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entry| entry["frame"]["method"] == "notifications/cancelled")
            );
        }
    }
}

#[test]
fn counting_server_records_authentication_without_recording_secret_values() {
    let fixture = Fixture::new();
    let mut command = Command::new(stand_in_path());
    command
        .args(["--counting-mcp-server", "--log"])
        .arg(&fixture.log)
        .env("BRIDGE_TEST_TOKEN", "bridge-secret-must-not-reach-cli");
    let mut server = Process::spawn(&mut command, fixture.deadline);
    server.send(
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
        "name":"count","arguments":{"output":"large fixture output"}}}),
        "credential probe",
    );
    assert_eq!(
        server.next("credential probe response")["result"]["content"][0]["text"],
        "large fixture output"
    );
    server.close_stdin();
    assert!(server.finish(None).status.success());
    let started = std::fs::read_to_string(fixture.log.with_extension("started.jsonl")).unwrap();
    assert!(!started.contains("bridge-secret-must-not-reach-cli"));
    assert_eq!(
        serde_json::from_str::<Value>(started.trim()).unwrap()["authenticated"],
        true
    );
    assert_eq!(fixture.log(), [json!({"output":"large fixture output"})]);
}

#[test]
fn default_tool_timeout_fires_when_configured_timeout_is_ignored() {
    for codex in [false, true] {
        let fixture = Fixture::new();
        let (mut command, recorded) = script_command(
            fixture.directory.path(),
            &fixture.args(codex, true, None),
            &json!([
                {"call":{"tool":"slow_count", "arguments":{"seconds":10}}},
                {"say":"default deadline fired"}
            ]),
        );
        command
            .env("CLI_STAND_IN_DEFAULT_TOOL_TIMEOUT_MS", "2000")
            .env("CLI_STAND_IN_IGNORE_TOOL_TIMEOUT", "true");
        let started = std::time::Instant::now();
        let run = super::support::recorded_run(
            Process::spawn(&mut command, fixture.deadline).finish(Some(&recorded)),
            &recorded,
        );
        successful(&run, codex);
        assert!(started.elapsed() >= Duration::from_secs(2));
        assert_eq!(run.record["tool_timed_out"], true);
        assert_eq!(run.record["tool_timeout_ms"], 2000);
        assert!(run.record["last_result"].is_null());
        assert!(fixture.log().is_empty());
        let entered = std::fs::read_to_string(fixture.log.with_extension("started.jsonl")).unwrap();
        assert_eq!(entered.lines().count(), 1, "the call reached the server");
        let call = sent_calls(&run.record)[0]["frame"]["id"].clone();
        assert!(
            run.record["wire"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["direction"] != "received" || entry["frame"]["id"] != call)
        );
        if codex {
            assert!(
                run.record["argv"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("mcp_servers.smith.tool_timeout_sec=604800"))
            );
        } else {
            assert_eq!(run.record["environment"]["MCP_TOOL_TIMEOUT"], "604800000");
            assert!(run.record["wire"].as_array().unwrap().iter().any(
                |entry| entry["frame"]["method"] == "notifications/cancelled"
                    && entry["frame"]["params"]["requestId"] == call
            ));
        }
    }
}
