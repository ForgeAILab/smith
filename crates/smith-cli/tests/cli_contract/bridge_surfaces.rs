//! Surface contracts use the real bridge and the fixture-backed installed CLI.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use super::Fixture;
use super::support::{BridgeCleanup, Deadline, Process, StandInRun, stand_in_path};

struct SurfaceFixture {
    cleanup: BridgeCleanup,
    smith: Fixture,
}

impl SurfaceFixture {
    fn new(codex: bool) -> Self {
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
delegation = true
[providers.local]
kind = "fake"
[harness.{kind}]
executable = {executable}
"#,
            executable = json!(stand_in_path())
        );
        let smith = Fixture::with_private_user_config(&config);
        Self {
            cleanup: BridgeCleanup::from_record(smith.project.path().join("surface-record.json")),
            smith,
        }
    }

    fn run(&self, format: &str, script: Value, approve: bool) -> StandInRun {
        self.cleanup.capture();
        let script_path = self.smith.project.path().join("surface-script.json");
        let record_path = self.smith.project.path().join("surface-record.json");
        std::fs::write(
            &script_path,
            serde_json::to_vec(&script).expect("script JSON"),
        )
        .expect("script");
        let mut command = self.smith.command();
        command
            .args(["-p", "bridge surfaces", "--output-format", format])
            .env("CLI_STAND_IN_SCRIPT", script_path)
            .env("CLI_STAND_IN_RECORD", &record_path);
        if approve {
            command.args(["--approval", "allow-all"]);
        }
        let output = Process::spawn(&mut command, Deadline::after(Duration::from_secs(30)))
            .finish(Some(&record_path));
        self.cleanup.capture();
        let record = serde_json::from_slice(
            &std::fs::read(&record_path)
                .unwrap_or_else(|error| panic!("record: {error}; {output:?}")),
        )
        .expect("record JSON");
        let lines = if format == "text" {
            Vec::new()
        } else {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| serde_json::from_str(line).expect("machine output JSON"))
                .collect()
        };
        StandInRun {
            output,
            lines,
            record,
        }
    }
}

fn mixed_script() -> Value {
    serde_json::from_str(include_str!(
        "../../../smith-tui/tests/fixtures/bridge-surfaces-script.json"
    ))
    .expect("shared stand-in script")
}

fn result(run: &StandInRun) -> &Value {
    let results = run
        .lines
        .iter()
        .filter(|line| line["type"] == "result")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1, "one terminal result: {:?}", run.output);
    results[0]
}

fn payloads(run: &StandInRun) -> Vec<Value> {
    run.lines
        .iter()
        .filter(|line| line["type"] == "runtime_event")
        .map(|line| line["event"]["payload"].clone())
        .collect()
}

fn compare_golden(name: &str, value: &Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/headless")
        .join(name);
    let bytes = std::fs::read(path).expect("surface golden");
    let expected: Value = if name.ends_with(".jsonl") {
        Value::Array(
            String::from_utf8(bytes)
                .expect("golden UTF-8")
                .lines()
                .map(|line| serde_json::from_str(line).expect("golden JSONL"))
                .collect(),
        )
    } else {
        serde_json::from_slice(&bytes).expect("golden JSON")
    };
    assert_eq!(*value, expected);
}

fn result_projection(result: &Value) -> Value {
    json!({"type":result["type"],"status":result["status"],"output":result["output"]})
}

// Runtime identities and fingerprints vary between processes, while the
// complete surface payloads, owner distinction, and ordering must agree.
fn surface_projection(payloads: &[Value]) -> Vec<Value> {
    payloads
        .iter()
        .filter(|payload| {
            matches!(
                payload["event"].as_str(),
                Some(
                    "turn_started"
                        | "external_session_started"
                        | "external_text"
                        | "external_tool_invoked"
                        | "external_tool_completed"
                        | "tool_call_requested"
                        | "tool_call_completed"
                        | "turn_completed"
                )
            )
        })
        .map(|payload| {
            let mut payload = payload.clone();
            if payload.get("id").is_some() {
                payload["id"] = json!("cli-call");
            }
            if payload.get("call").is_some() {
                payload["call"] = json!("smith-call");
            }
            if payload.get("session").is_some() {
                payload["session"] = json!("cli-session");
            }
            if payload.get("argument_fingerprint").is_some() {
                payload["argument_fingerprint"] = json!("<FINGERPRINT>");
            }
            payload
        })
        .collect()
}

#[test]
fn stream_json_bridged_agent_is_an_ordinary_pair_beside_one_cli_pair() {
    for codex in [false, true] {
        let run = SurfaceFixture::new(codex).run("stream-json", mixed_script(), true);
        assert!(run.output.status.success(), "{:?}", run.output);
        assert_eq!(result(&run)["status"], "ok");
        assert_eq!(result(&run)["output"], "after");
        let events = payloads(&run);
        let requests = events
            .iter()
            .filter(|p| p["event"] == "tool_call_requested")
            .collect::<Vec<_>>();
        let completions = events
            .iter()
            .filter(|p| p["event"] == "tool_call_completed")
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 1, "{events:?}");
        assert_eq!(completions.len(), 1, "{events:?}");
        assert_eq!(requests[0]["name"], "agent");
        assert_eq!(completions[0]["name"], "agent");
        assert_eq!(requests[0]["call"], completions[0]["call"]);
        assert_eq!(completions[0]["is_error"], false);
        let own = events
            .iter()
            .filter(|p| p["event"] == "external_tool_invoked")
            .collect::<Vec<_>>();
        let own_results = events
            .iter()
            .filter(|p| p["event"] == "external_tool_completed")
            .collect::<Vec<_>>();
        assert_eq!(own.len(), 1, "{events:?}");
        assert_eq!(own_results.len(), 1, "{events:?}");
        assert_eq!(own[0]["id"], own_results[0]["id"]);
        assert_eq!(own_results[0]["ok"], true);
        assert!(!own[0]["name"].as_str().expect("own name").contains("smith"));
        assert!(
            !events
                .iter()
                .any(|p| p["name"] == "mcp__smith__agent" || p["name"] == "smith.agent")
        );
        if !codex {
            let mut golden = surface_projection(&events)
                .into_iter()
                .map(|payload| json!({"type":"runtime_event","event":{"payload":payload}}))
                .collect::<Vec<_>>();
            golden.push(result_projection(result(&run)));
            compare_golden("bridge-mixed.jsonl", &Value::Array(golden));
        }
    }
}

#[test]
fn json_bridged_agent_has_one_successful_result_and_final_text() {
    for codex in [false, true] {
        let run = SurfaceFixture::new(codex).run("json", mixed_script(), true);
        assert!(run.output.status.success(), "{:?}", run.output);
        assert_eq!(run.lines.len(), 1);
        compare_golden("bridge-mixed.json", &result_projection(result(&run)));
        assert!(!String::from_utf8_lossy(&run.output.stdout).contains("mcp__smith__agent"));
    }
}

#[test]
fn text_bridged_agent_prints_the_answer_and_exits_zero() {
    for codex in [false, true] {
        let run = SurfaceFixture::new(codex).run("text", mixed_script(), true);
        assert!(run.output.status.success(), "{:?}", run.output);
        assert_eq!(run.output.stdout, b"after\n");
    }
}

fn agent_response(run: &StandInRun) -> &Value {
    let wire = run.record["wire"].as_array().expect("wire");
    let call = wire
        .iter()
        .find(|entry| {
            entry["direction"] == "sent"
                && entry["frame"]["method"] == "tools/call"
                && entry["frame"]["params"]["name"] == "agent"
        })
        .expect("agent wire call");
    &wire
        .iter()
        .find(|entry| {
            entry["direction"] == "received" && entry["frame"]["id"] == call["frame"]["id"]
        })
        .expect("agent response")["frame"]
}

#[test]
fn bridged_spawn_fails_closed_with_the_native_headless_approval_result() {
    for format in ["json", "stream-json", "text"] {
        let run = SurfaceFixture::new(false).run(format, json!([
            {"call":{"tool":"agent","arguments":{"action":"spawn","task":"Do not execute without approval","tools":"all","workspace":"shared"}}},
            {"say":"refusal survived"}
        ]), false);
        assert_eq!(run.output.status.code(), Some(4), "{:?}", run.output);
        let response = agent_response(&run);
        assert!(
            response.get("error").is_some() || response["result"]["isError"] == true,
            "{response}"
        );
        assert!(response.to_string().contains("approval"), "{response}");
        if format == "text" {
            assert!(run.output.stdout.is_empty());
            assert!(
                String::from_utf8_lossy(&run.output.stderr)
                    .contains("approval required for tool `delegation.spawn`")
            );
        } else {
            let result = result(&run);
            assert_eq!(result["status"], "approval_required");
            let approval = &result["approval_required"];
            assert_eq!(approval["tool"], "delegation.spawn");
            assert_eq!(approval["requires_authorization"], true);
            assert_eq!(approval["mutates"], false);
            assert!(approval["authority_warnings"].is_array());
            assert_eq!(approval["permissions"], json!(["agent.delegate"]));
            assert_eq!(approval["resource"]["resource_kind"], "other");
            assert_eq!(approval["resource"]["kind"], "child-agent");
            assert!(approval["call_id"].is_string());
            assert!(approval["argument_keys"].is_array());
            assert!(approval["preparation_fingerprint"].is_string());
            assert!(result.get("error").is_none());
            assert!(result["lifecycle"].get("children").is_none());
            assert!(!payloads(&run).iter().any(|p| p["event"] == "child_spawned"));
        }
    }
}

#[test]
fn claude_never_connected_diagnostic_survives_all_formats_without_failure() {
    for format in ["text", "json", "stream-json"] {
        let run = SurfaceFixture::new(false).run(
            format,
            json!([
                {"server_never_starts":true}, {"say":"answer without bridge"}
            ]),
            true,
        );
        assert!(run.output.status.success(), "{:?}", run.output);
        assert_eq!(run.record["server_spawned"], false);
        let expected = "Claude Code did not load Smith's tools";
        if format == "text" {
            assert_eq!(run.output.stdout, b"answer without bridge\n");
            let stderr = String::from_utf8_lossy(&run.output.stderr);
            assert!(
                stderr
                    .lines()
                    .any(|line| line.starts_with("smith: ") && line.contains(expected)),
                "{stderr}"
            );
        } else {
            let result = result(&run);
            assert_eq!(result["status"], "ok");
            assert_eq!(result["output"], "answer without bridge");
            assert!(result.get("error").is_none());
            let diagnostics = result["diagnostics"].as_array().expect("diagnostics array");
            assert_eq!(diagnostics.len(), 1);
            assert!(
                diagnostics[0]
                    .as_str()
                    .expect("diagnostic text")
                    .contains(expected)
            );
            if format == "stream-json" {
                let events = payloads(&run);
                let errors = events
                    .iter()
                    .filter(|p| p["event"] == "error")
                    .collect::<Vec<_>>();
                assert_eq!(errors.len(), 1);
                assert!(
                    errors[0]["error"]["message"]
                        .as_str()
                        .expect("runtime diagnostic")
                        .contains(expected)
                );
                assert!(events.iter().any(
                    |p| p["event"] == "turn_completed" && p["finish"]["outcome"] == "completed"
                ));
            }
        }
    }
}
