//! Layer 4: the installed Smith binary, harness argv, relay, and runtime dispatch.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use super::Fixture;
use super::support::{
    BridgeCleanup, Deadline, StandInRun, run_scripted_smith, session_file, stand_in_path,
};

pub(crate) struct BridgeFixture {
    // Drop before the fixture's record directory disappears, and after any
    // test-local Process/HarnessCleanup guards have stopped their processes.
    pub(crate) cleanup: BridgeCleanup,
    pub(crate) smith: Fixture,
    pub(crate) deadline: Deadline,
    pub(crate) codex: bool,
}

impl BridgeFixture {
    pub(crate) fn new(codex: bool, deny_all: bool) -> Self {
        let settings = if deny_all {
            "[profiles.dev.capabilities]\ndeny = [\"tool:*\", \"skill:*\"]\n"
        } else {
            ""
        };
        Self::configured(codex, &format!("delegation = true\n{settings}"), "")
    }

    pub(crate) fn configured(codex: bool, settings: &str, extra: &str) -> Self {
        let deadline = Deadline::after(Duration::from_secs(30));
        let kind = if codex { "codex" } else { "claude-code" };
        let model = if codex { "gpt-6-sol" } else { "sonnet" };
        let config = format!(
            r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "cli/{kind}/{model}"
posture = "build"
use = ["main"]
{settings}
[providers.local]
kind = "fake"

[harness.{kind}]
executable = {executable}

{extra}
"#,
            executable = json!(stand_in_path().to_str().expect("UTF-8 stand-in path")),
        );
        // Harness executables are owner-controlled and must be in the user layer.
        let smith = Fixture::with_private_user_config(&config);
        Self {
            cleanup: BridgeCleanup::from_record(smith.project.path().join("record.json")),
            smith,
            deadline,
            codex,
        }
    }

    pub(crate) fn run(&self, script: Value, resume: Option<&str>) -> StandInRun {
        self.run_policy(script, resume, "allow-all")
    }

    pub(crate) fn command(&self, resume: Option<&str>, approval: &str) -> std::process::Command {
        self.cleanup.capture();
        let mut command = self.smith.command();
        command.args([
            "-p",
            "bridge contract",
            "--output-format",
            "stream-json",
            "--approval",
            approval,
        ]);
        if let Some(session) = resume {
            command.args(["--resume", session]);
        }
        command
    }

    pub(crate) fn run_policy(
        &self,
        script: Value,
        resume: Option<&str>,
        approval: &str,
    ) -> StandInRun {
        let run = run_scripted_smith(
            &mut self.command(resume, approval),
            self.smith.project.path(),
            &script,
            self.deadline,
        );
        self.cleanup.capture();
        run
    }

    pub(crate) fn append_config(&self, extra: &str) {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(self.smith.home.path().join(".smith/config.toml"))
            .expect("user config")
            .write_all(extra.as_bytes())
            .expect("additional configuration");
    }

    pub(crate) fn cli(&self, args: &[&str]) -> std::process::Output {
        self.cleanup.capture();
        let output = super::support::Process::spawn(
            self.smith.command().args(args).env(
                "CLI_STAND_IN_RECORD",
                self.smith.project.path().join("record.json"),
            ),
            self.deadline,
        )
        .finish(None);
        self.cleanup.capture();
        output
    }

    /// The same installed binary and workspace, with a deterministic native
    /// provider instead of an external turn. Records the real provider view.
    pub(crate) fn native(&self, tool: &str, arguments: Value, approval: &str) -> StandInRun {
        let call = self.smith.project.path().join("native-call.json");
        let requests = self.smith.project.path().join("native-requests.jsonl");
        std::fs::write(
            &call,
            json!({"tool":tool,"arguments":arguments}).to_string(),
        )
        .unwrap();
        std::fs::write(&requests, "").unwrap();
        self.append_config(&format!(
            r#"
[profiles.native]
provider = "native"
model = "native-model"
posture = "build"
use = ["main"]
delegation = true
[providers.native]
kind = "command-jsonl"
[providers.native.command]
executable = {}
args = {}
cwd = "workspace"
[models."native/native-model"]
context_tokens = 64000
max_input_tokens = 60000
max_output_tokens = 2048
"#,
            json!(stand_in_path()),
            json!(["--native-contract-provider", call, requests])
        ));
        let output = self.cli(&[
            "--profile",
            "native",
            "-p",
            "delegate the native contract task",
            "--output-format",
            "stream-json",
            "--approval",
            approval,
        ]);
        let lines = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| serde_json::from_str(line).expect("native headless JSON"))
            .collect();
        let record = json!({"requests":log_lines(&requests)});
        StandInRun {
            output,
            lines,
            record,
        }
    }

    pub(crate) fn counting(&self, trusted: bool, credential: bool) -> PathBuf {
        let log = self.smith.project.path().join("counting.jsonl");
        std::fs::write(&log, "").expect("empty effect log");
        if credential {
            use std::os::unix::fs::PermissionsExt;
            let root = self.smith.home.path().join(".smith");
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            let auth = root.join("auth.json");
            std::fs::write(
                &auth,
                json!({"schema_version":1,"credentials":{
                    "bridge-test-token":"bridge-secret-must-not-reach-cli"
                }})
                .to_string(),
            )
            .unwrap();
            std::fs::set_permissions(auth, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let env = if credential {
            "[mcp.servers.docs.env]\nBRIDGE_TEST_TOKEN = \"authfile:bridge-test-token\"\n"
        } else {
            ""
        };
        self.append_config(&format!(
            "\n[mcp.servers.docs]\ncommand = {}\nargs = {}\n{env}",
            json!(stand_in_path()),
            json!(["--counting-mcp-server", "--log", log])
        ));
        if trusted {
            let config = smith_config::resolve::resolve(
                &smith_config::resolve::ResolveRequest::new(self.smith.project.path())
                    .with_home_dir(self.smith.home.path()),
            )
            .expect("MCP configuration")
            .config;
            smith_config::trust::TrustStore::open(self.smith.home.path().join(".smith"))
                .expect("trust store")
                .record(
                    self.smith.project.path(),
                    &smith_config::mcp::executable(&config.mcp.servers["docs"]),
                    smith_config::trust::TrustDecision::Allow,
                )
                .expect("trusted server");
        }
        log
    }

    pub(crate) fn snapshot(&self, session: &str) -> Value {
        let path = session_file(self.smith.home.path(), session, ".snapshot.json");
        serde_json::from_slice(&std::fs::read(path).expect("saved snapshot"))
            .expect("snapshot JSON")
    }

    pub(crate) fn journal(&self, session: &str) -> Vec<Value> {
        let path = session_file(self.smith.home.path(), session, ".jsonl");
        std::fs::read_to_string(path)
            .expect("saved journal")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("journal JSON"))
            .filter(|line| line["record"] == "event")
            .map(|line| line["event"].clone())
            .collect()
    }
}

pub(crate) fn result(run: &StandInRun) -> &Value {
    let results = run
        .lines
        .iter()
        .filter(|line| line["type"] == "result")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1, "one Smith result: {:?}", run.output);
    results[0]
}

pub(crate) fn successful(run: &StandInRun) {
    assert!(run.output.status.success(), "{:?}", run.output);
    assert_eq!(result(run)["status"], "ok", "{:?}", run.output);
}

pub(crate) fn events(run: &StandInRun) -> Vec<Value> {
    run.lines
        .iter()
        .filter(|line| line["type"] == "runtime_event")
        .map(|line| line["event"].clone())
        .collect()
}

pub(crate) fn argv(run: &StandInRun) -> Vec<&str> {
    run.record["argv"]
        .as_array()
        .expect("recorded argv")
        .iter()
        .map(|arg| arg.as_str().expect("argv string"))
        .skip(1)
        .collect()
}

pub(crate) fn socket(run: &StandInRun) -> PathBuf {
    assert_eq!(run.record["server"]["args"][0], "mcp-relay");
    assert_eq!(
        run.record["server"]["args"]
            .as_array()
            .expect("relay args")
            .len(),
        2
    );
    PathBuf::from(
        run.record["server"]["args"][1]
            .as_str()
            .expect("socket path"),
    )
}

pub(crate) fn cleaned(socket: &Path) {
    assert!(!socket.exists(), "turn left socket {}", socket.display());
    let directory = socket.parent().expect("private socket directory");
    assert!(
        !directory.exists(),
        "turn left directory {}",
        directory.display()
    );
}

fn declaration(fixture: &BridgeFixture, run: &StandInRun) {
    let args = argv(run);
    assert_eq!(
        run.record["dialect"],
        if fixture.codex {
            "codex"
        } else {
            "claude-code"
        }
    );
    assert_eq!(run.record["server"]["name"], "smith");
    assert_eq!(run.record["server"]["command"], env!("CARGO_BIN_EXE_smith"));
    assert_eq!(run.record["prompt"], "bridge contract");
    let socket = socket(run);
    cleaned(&socket);
    if fixture.codex {
        let overrides = args
            .windows(2)
            .filter(|pair| pair[0] == "-c")
            .map(|pair| pair[1])
            .collect::<Vec<_>>();
        let command = format!(
            "mcp_servers.smith.command={}",
            json!(env!("CARGO_BIN_EXE_smith"))
        );
        let relay_args = format!("mcp_servers.smith.args={}", json!(["mcp-relay", socket]));
        assert_eq!(
            overrides,
            [
                command.as_str(),
                relay_args.as_str(),
                "mcp_servers.smith.default_tools_approval_mode=\"approve\"",
                "mcp_servers.smith.tool_timeout_sec=604800",
                "mcp_servers.smith.required=true",
                "mcp_servers.smith.startup_timeout_sec=60",
            ]
        );
        assert_eq!(run.record["flags"]["--sandbox"], "read-only");
        if let Some(index) = args.iter().position(|arg| *arg == "resume") {
            assert!(args.iter().take(index).any(|arg| *arg == "-c"));
            assert!(!args[index..].contains(&"-c"));
        }
    } else {
        let config = args
            .iter()
            .position(|arg| *arg == "--mcp-config")
            .expect("inline MCP config");
        assert_eq!(
            serde_json::from_str::<Value>(args[config + 1]).expect("inline JSON"),
            json!({
                "mcpServers": {"smith": {
                    "command": env!("CARGO_BIN_EXE_smith"), "args": ["mcp-relay", socket]
                }}
            })
        );
        assert_eq!(run.record["flags"]["--allowedTools"], "mcp__smith");
        assert_eq!(run.record["environment"]["MCP_TOOL_TIMEOUT"], "604800000");
        assert_eq!(run.record["environment"]["MCP_TIMEOUT"], "60000");
    }
}

pub(crate) fn call_response<'a>(run: &'a StandInRun, tool: &str) -> &'a Value {
    let wire = run.record["wire"].as_array().expect("wire record");
    let calls = wire
        .iter()
        .filter(|entry| {
            entry["direction"] == "sent"
                && entry["frame"]["method"] == "tools/call"
                && entry["frame"]["params"]["name"] == tool
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1, "one wire call to {tool}");
    let replies = wire
        .iter()
        .filter(|entry| {
            entry["direction"] == "received" && entry["frame"]["id"] == calls[0]["frame"]["id"]
        })
        .collect::<Vec<_>>();
    assert_eq!(replies.len(), 1, "one wire response to {tool}");
    &replies[0]["frame"]
}

pub(crate) fn no_tool_event(events: &[Value], name: &str) {
    assert!(
        events.iter().all(|event| {
            let payload = &event["payload"];
            !(matches!(
                payload["event"].as_str(),
                Some("tool_call_requested" | "tool_call_completed" | "external_tool_invoked")
            ) && payload["name"] == name)
        }),
        "unoffered tool {name} appeared in events: {events:?}"
    );
}

fn call_order(events: &[Value], turn: &Value) -> Value {
    let requests = events
        .iter()
        .enumerate()
        .filter(|(_, event)| {
            event["turn"] == *turn
                && event["payload"]["event"] == "tool_call_requested"
                && event["payload"]["name"] == "agent"
        })
        .collect::<Vec<_>>();
    let completions = events
        .iter()
        .enumerate()
        .filter(|(_, event)| {
            event["turn"] == *turn
                && event["payload"]["event"] == "tool_call_completed"
                && event["payload"]["name"] == "agent"
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 1, "agent invoked once: {events:?}");
    assert_eq!(completions.len(), 1, "agent completed once: {events:?}");
    let (requested, request) = requests[0];
    let (completed, completion) = completions[0];
    assert_eq!(completion["payload"]["call"], request["payload"]["call"]);
    assert_eq!(completion["payload"]["is_error"], false);
    let text = |expected: &str| {
        events
            .iter()
            .position(|event| {
                event["turn"] == *turn
                    && event["payload"]["event"] == "external_text"
                    && event["payload"]["text"] == expected
            })
            .unwrap_or_else(|| panic!("missing text {expected}: {events:?}"))
    };
    assert!(
        text("before") < requested && requested < completed && completed < text("after"),
        "canonical text/call/result order: {events:?}"
    );
    no_tool_event(events, "mcp__smith__agent");
    assert!(
        events.iter().all(|event| !matches!(
            event["payload"]["event"].as_str(),
            Some("external_tool_invoked" | "external_tool_completed")
        )),
        "the scripted MCP call must have no CLI observation rows: {events:?}"
    );
    assert!(
        events
            .iter()
            .all(|event| event["payload"]["event"] != "provider_attempt_started"),
        "the fake provider must never produce a CLI turn"
    );
    request["payload"]["call"].clone()
}

fn history_call(snapshot: &Value, call: &Value) {
    let history = snapshot["snapshot"]["history"]
        .as_array()
        .expect("canonical history");
    let requests = history
        .iter()
        .enumerate()
        .flat_map(|(index, message)| {
            message["content"]
                .as_array()
                .expect("message content")
                .iter()
                .map(move |part| (index, part))
        })
        .filter(|(_, part)| part["type"] == "tool_call" && part["id"] == *call)
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 1, "one committed agent call");
    let (index, request) = requests[0];
    assert_eq!(request["name"], "agent");
    assert_eq!(request["arguments"], json!({"action":"list"}));
    assert_eq!(history[index]["role"], "assistant");
    assert!(
        history[index]["content"]
            .as_array()
            .expect("assistant parts")
            .iter()
            .any(|part| part["type"] == "text" && part["text"] == "before")
    );
    let outcome = &history[index + 1];
    assert_eq!(outcome["role"], "tool");
    let results = outcome["content"]
        .as_array()
        .expect("tool parts")
        .iter()
        .filter(|part| part["type"] == "tool_result" && part["call_id"] == *call)
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1, "one committed result");
    assert_eq!(results[0]["name"], "agent");
    assert_eq!(results[0]["is_error"], false);
    assert!(
        results[0]["content"].to_string().contains("children"),
        "agent list ran: {outcome}"
    );
    assert_eq!(history[index + 2]["role"], "assistant");
    assert!(
        history[index + 2]["content"]
            .as_array()
            .expect("remaining text")
            .iter()
            .any(|part| part["type"] == "text" && part["text"] == "after")
    );
}

fn bridged_turn(fixture: &BridgeFixture, resume: Option<&str>) -> StandInRun {
    let mut steps = vec![
        json!({"say":"before"}),
        json!({"list_tools":true}),
        json!({"call":{"tool":"agent","arguments":{"action":"list"}}}),
        json!({"say":"after"}),
    ];
    if resume.is_some() {
        steps.push(
            json!({"call":{"tool":"registry.search", "arguments":{"query":"stable discovery"}}}),
        );
    }
    let run = fixture.run(json!(steps), resume);
    successful(&run);
    declaration(fixture, &run);
    assert_eq!(run.record["handshake_path"], "initialize");
    if !fixture.codex {
        let wire = run.record["wire"].as_array().expect("wire");
        let probe = wire
            .iter()
            .find(|entry| {
                entry["direction"] == "sent" && entry["frame"]["method"] == "server/discover"
            })
            .expect("discover probe");
        assert!(wire.iter().any(|entry| entry["direction"] == "received"
            && entry["frame"]["id"] == probe["frame"]["id"]
            && entry["frame"]["error"]["code"] == -32601));
    }
    let names = run.record["tool_names"].as_array().expect("listed names");
    assert!(names.contains(&json!("agent")));
    for excluded in [
        "read",
        "list",
        "search",
        "edit",
        "shell",
        "task_output",
        "task_stop",
        "write_todos",
        "get_goal",
        "create_goal",
        "update_goal",
    ] {
        assert!(
            !names.contains(&json!(excluded)),
            "CLI received {excluded}: {names:?}"
        );
    }
    let response = call_response(&run, "agent");
    assert!(response.get("error").is_none(), "{response}");
    assert_ne!(response["result"]["isError"], true, "{response}");
    // The list result observes the real coordinator; a protocol success alone
    // would also pass if dispatch returned a canned value without running agent.
    assert!(
        response["result"]["content"]
            .to_string()
            .contains("children"),
        "{response}"
    );
    let result = result(&run);
    let observed = events(&run);
    let call = call_order(&observed, &result["turn_id"]);
    let usage = observed
        .iter()
        .filter(|event| {
            event["turn"] == result["turn_id"]
                && event["payload"]["event"] == "usage"
                && event["payload"]["record"]["source"] == "external_agent"
        })
        .collect::<Vec<_>>();
    assert_eq!(usage.len(), 1, "one harness usage record: {observed:?}");
    assert_eq!(
        usage[0]["payload"]["record"]["provenance"]["purpose"],
        "agent.external"
    );
    assert!(
        result["usage"]["current_turn"]["input_uncached"]
            .as_u64()
            .expect("input usage")
            > 0
    );
    let session = result["session_id"].as_str().expect("Smith session id");
    history_call(&fixture.snapshot(session), &call);
    assert_eq!(
        call_order(&fixture.journal(session), &result["turn_id"]),
        call
    );
    run
}

fn first_and_resumed_turn(codex: bool) {
    let fixture = BridgeFixture::new(codex, false);
    let skill = fixture
        .smith
        .home
        .path()
        .join(".smith/skills/stability/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        skill,
        "---\ndescription: stable discovery\n---\nInspect stability.",
    )
    .unwrap();
    let first = bridged_turn(&fixture, None);
    let session = result(&first)["session_id"].as_str().expect("session id");
    let second = bridged_turn(&fixture, Some(session));
    assert_eq!(result(&second)["session_id"], session);
    assert_ne!(result(&first)["turn_id"], result(&second)["turn_id"]);
    assert_eq!(second.record["resume"], first.record["session_id"]);
    assert_eq!(second.record["session_source"], "resume");
    assert_ne!(socket(&second), socket(&first));
    cleaned(&socket(&first));
    let history = fixture.snapshot(session);
    let calls = history["snapshot"]["history"]
        .as_array()
        .expect("history")
        .iter()
        .flat_map(|message| message["content"].as_array().expect("parts"))
        .filter(|part| part["type"] == "tool_call" && part["name"] == "agent")
        .count();
    assert_eq!(calls, 2, "both calls survive session resume");
    // The dotted Smith name is unchanged on each invocation, including resume.
    for run in [&first, &second] {
        let names = run.record["tool_names"].as_array().unwrap();
        assert_eq!(
            names
                .iter()
                .filter(|name| **name == json!("registry.search"))
                .count(),
            1
        );
    }
    let reply = call_response(&second, "registry.search");
    assert!(reply.get("error").is_none(), "{reply}");
    assert_ne!(reply["result"]["isError"], true, "{reply}");
    assert!(
        response_value(reply)["cards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|card| card["id"] == json!({"domain":"skill", "name":"stability"}))
    );
    assert_eq!(
        events(&second)
            .iter()
            .filter(|event| event["payload"]["event"] == "tool_call_completed"
                && event["payload"]["name"] == "registry.search")
            .count(),
        1
    );
}

#[test]
fn claude_bridged_agent_call_and_resume_commit_events_and_history() {
    first_and_resumed_turn(false);
}

#[test]
fn codex_bridged_agent_call_and_resume_commit_events_and_history() {
    first_and_resumed_turn(true);
}

fn denied_set(codex: bool) {
    let fixture = BridgeFixture::new(codex, true);
    let run = fixture.run(json!([{"say":"nothing to bridge"}]), None);
    successful(&run);
    assert_eq!(run.record["server"], Value::Null);
    assert_eq!(run.record["server_spawned"], false);
    assert!(run.record["wire"].as_array().expect("wire").is_empty());
    let args = argv(&run);
    let project = fixture
        .smith
        .project
        .path()
        .canonicalize()
        .expect("canonical project");
    let expected = if codex {
        vec![
            "exec",
            "--json",
            "--skip-git-repo-check",
            "-C",
            project.to_str().expect("project path"),
            "--sandbox",
            "read-only",
            "-m",
            "gpt-6-sol",
            "bridge contract",
        ]
    } else {
        vec![
            "-p",
            "bridge contract",
            "--output-format",
            "stream-json",
            "--verbose",
            "--allowedTools",
            "",
            "--model",
            "sonnet",
        ]
    };
    assert_eq!(
        args, expected,
        "all-denied invocation is byte-for-byte the original argv"
    );
    assert!(events(&run).iter().all(|event| !matches!(
        event["payload"]["event"].as_str(),
        Some("tool_call_requested" | "tool_call_completed")
    )));
}

#[test]
fn claude_deny_tools_and_skills_omits_the_bridge() {
    denied_set(false);
}

#[test]
fn codex_deny_tools_and_skills_omits_the_bridge() {
    denied_set(true);
}

fn unoffered_call(codex: bool) {
    let fixture = BridgeFixture::new(codex, false);
    let run = fixture.run(
        json!([
            {"call":{"tool":"shell","arguments":{"command":"echo hi"}}}, {"say":"refusal survived"}
        ]),
        None,
    );
    successful(&run);
    declaration(&fixture, &run);
    let response = call_response(&run, "shell");
    assert!(
        response.get("error").is_some() || response["result"]["isError"] == true,
        "unoffered shell must be refused: {response}"
    );
    no_tool_event(&events(&run), "shell");
    no_tool_event(&events(&run), "mcp__smith__shell");
    let session = result(&run)["session_id"].as_str().expect("session id");
    no_tool_event(&fixture.journal(session), "shell");
    let snapshot = fixture.snapshot(session);
    assert!(
        snapshot["snapshot"]["history"]
            .as_array()
            .expect("history")
            .iter()
            .flat_map(|message| message["content"].as_array().expect("parts"))
            .all(|part| part["type"] != "tool_call" || part["name"] != "shell")
    );
}

#[test]
fn claude_refuses_an_unoffered_tool_without_dispatch_or_a_failed_turn() {
    unoffered_call(false);
}

#[test]
fn codex_refuses_an_unoffered_tool_without_dispatch_or_a_failed_turn() {
    unoffered_call(true);
}

fn exit_mid_call(codex: bool) {
    let fixture = BridgeFixture::new(codex, false);
    let run = fixture.run(
        json!([
            {"call_then_exit":{"tool":"agent","arguments":{"action":"list"},"code":7}}
        ]),
        None,
    );
    declaration(&fixture, &run);
    assert!(!run.output.status.success(), "{:?}", run.output);
    assert_eq!(result(&run)["status"], "failed");
    assert!(
        result(&run)["error"]
            .as_str()
            .expect("failure reason")
            .contains("exited before reporting a result")
    );
    assert_eq!(
        run.record["steps"][0]["outcome"],
        "exited_with_call_outstanding"
    );
    let wire = run.record["wire"].as_array().expect("wire");
    assert_eq!(wire.iter().filter(|entry| entry["direction"] == "sent"
        && entry["frame"]["method"] == "tools/call").count(), 1);
    no_tool_event(&events(&run), "mcp__smith__agent");
}

#[test]
fn claude_exiting_mid_call_fails_and_removes_the_endpoint() {
    exit_mid_call(false);
}

#[test]
fn codex_exiting_mid_call_fails_and_removes_the_endpoint() {
    exit_mid_call(true);
}

#[test]
fn codex_required_bridge_that_never_starts_preserves_the_failure_reason() {
    let fixture = BridgeFixture::new(true, false);
    let run = fixture.run(json!([{"server_never_starts":true}]), None);
    declaration(&fixture, &run);
    assert!(!run.output.status.success(), "{:?}", run.output);
    assert_eq!(result(&run)["status"], "failed");
    assert!(
        result(&run)["error"]
            .as_str()
            .expect("startup failure reason")
            .contains("required MCP servers failed to initialize")
    );
    assert_eq!(run.record["server_spawned"], false);
    assert!(run.record["wire"].as_array().expect("wire").is_empty());
    assert_eq!(run.record["steps"][0]["outcome"], "required_server_failed");
}

/// Decode the actual MCP content rather than assuming a protocol success ran a tool.
pub(crate) fn responses(record: &Value, tool: &str) -> Vec<Value> {
    let wire = record["wire"].as_array().expect("wire");
    wire.iter()
        .filter(|entry| {
            entry["direction"] == "sent"
                && entry["frame"]["method"] == "tools/call"
                && entry["frame"]["params"]["name"] == tool
        })
        .map(|call| {
            wire.iter()
                .find(|entry| {
                    entry["direction"] == "received" && entry["frame"]["id"] == call["frame"]["id"]
                })
                .unwrap_or_else(|| panic!("missing reply for {call}"))["frame"]
                .clone()
        })
        .collect()
}

pub(crate) fn response_value(response: &Value) -> Value {
    assert!(response.get("error").is_none(), "{response}");
    assert_ne!(response["result"]["isError"], true, "{response}");
    let text = response_text(response);
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

pub(crate) fn response_text(response: &Value) -> String {
    response["result"]["content"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|| response["error"].to_string())
}

pub(crate) fn history_parts(snapshot: &Value) -> Vec<&Value> {
    snapshot["snapshot"]["history"]
        .as_array()
        .expect("history")
        .iter()
        .flat_map(|message| message["content"].as_array().expect("content"))
        .collect()
}

pub(crate) fn log_lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .expect("counting log")
        .lines()
        .map(|line| serde_json::from_str(line).expect("log JSON"))
        .collect()
}

pub(crate) fn ordinary_calls(fixture: &BridgeFixture, run: &StandInRun, expected: &[&str]) {
    let observed = events(run);
    let requested = observed
        .iter()
        .filter(|event| event["payload"]["event"] == "tool_call_requested")
        .collect::<Vec<_>>();
    assert_eq!(
        requested
            .iter()
            .map(|event| event["payload"]["name"].as_str().expect("name"))
            .collect::<Vec<_>>(),
        expected
    );
    for request in requested {
        assert_eq!(
            observed
                .iter()
                .filter(|event| event["payload"]["event"] == "tool_call_completed"
                    && event["payload"]["call"] == request["payload"]["call"])
                .count(),
            1
        );
    }
    assert!(observed.iter().all(|event| {
        event["payload"]["event"] != "external_tool_invoked"
            || !event["payload"]["name"]
                .as_str()
                .unwrap_or("")
                .starts_with("mcp__smith")
    }));
    let session = result(run)["session_id"].as_str().expect("session");
    let snapshot = fixture.snapshot(session);
    let parts = history_parts(&snapshot);
    assert_eq!(
        parts
            .iter()
            .filter(|part| part["type"] == "tool_call")
            .map(|part| part["name"].as_str().expect("name"))
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        parts
            .iter()
            .filter(|part| part["type"] == "tool_result")
            .count(),
        expected.len()
    );
    assert_eq!(
        fixture
            .journal(session)
            .iter()
            .filter(|event| event["payload"]["event"] == "tool_call_completed")
            .count(),
        expected.len()
    );
    for invocation in run.invocations() {
        let path = PathBuf::from(invocation["server"]["args"][1].as_str().expect("socket"));
        cleaned(&path);
    }
}

#[test]
fn bridge_turns_run_with_neither_cli_on_path() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        let shims = fixture.smith.project.path().join("stand-in-only-path");
        std::fs::create_dir(&shims).unwrap();
        std::os::unix::fs::symlink(stand_in_path(), shims.join("cli_stand_in")).unwrap();
        assert!(!shims.join("claude").exists());
        assert!(!shims.join("codex").exists());
        let mut command = fixture.command(None, "allow-all");
        command.env("PATH", &shims);
        let run = run_scripted_smith(
            &mut command,
            fixture.smith.project.path(),
            &json!([{"list_tools":true},{"call":{"tool":"agent", "arguments":{"action":"list"}}}]),
            fixture.deadline,
        );
        successful(&run);
        assert_eq!(run.record["environment"]["PATH"], shims.to_str().unwrap());
        assert!(response_value(call_response(&run, "agent"))["children"].is_array());
        ordinary_calls(&fixture, &run, &["agent"]);
    }
}
