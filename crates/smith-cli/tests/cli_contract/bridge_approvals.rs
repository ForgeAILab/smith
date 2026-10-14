//! Interactive terminal broker, real endpoint and real Smith stdio relay.
//!
//! A stand-in host subprocess embeds the public host API because it resolves
//! current_exe as the relay. Its relay/supervisor subcommands exec real Smith.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use super::bridge::{BridgeFixture, call_response, cleaned, log_lines, response_value};
use super::support::{
    BridgeCleanup, Process, StandInRun, kill_group, recorded_run, script_files, stand_in_path,
};

const COUNT: &str = "mcp__docs__count";

struct HarnessGroups {
    record: PathBuf,
    group: Option<u32>,
}
impl HarnessGroups {
    fn capture(&mut self, cleanup: &BridgeCleanup) {
        cleanup.capture();
        let pid = i32::try_from(record(&self.record)["pid"].as_u64().unwrap()).unwrap();
        let group = nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(pid)))
            .expect("live harness group");
        self.group = Some(u32::try_from(group.as_raw()).unwrap());
    }
}
impl Drop for HarnessGroups {
    fn drop(&mut self) {
        // The host has its own group and its CLI supervisor creates another.
        // Kill that second group even if a failed assertion skips shutdown.
        if let Some(group) = self.group {
            kill_group(group);
            return;
        }
        if let Ok(bytes) = std::fs::read(&self.record)
            && let Ok(record) = serde_json::from_slice::<Value>(&bytes)
            && let Some(pid) = record["pid"].as_u64()
            && let Ok(pid) = i32::try_from(pid)
            && let Ok(group) = nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(pid)))
            && let Ok(group) = u32::try_from(group.as_raw())
        {
            kill_group(group);
        }
    }
}

fn next_type(process: &mut Process, expected: &str) -> Value {
    loop {
        let line = process.next(expected);
        assert_ne!(
            line["type"], "finished",
            "host ended before {expected}: {line}"
        );
        if line["type"] == expected {
            return line;
        }
    }
}

fn record(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("running CLI record")).unwrap()
}

fn no_received_result(record: &Value) {
    no_wire_result(record);
    assert!(record["last_result"].is_null(), "{record}");
}

fn no_wire_result(record: &Value) {
    let wire = record["wire"].as_array().unwrap();
    let request = wire
        .iter()
        .find(|entry| entry["direction"] == "sent" && entry["frame"]["method"] == "tools/call")
        .expect("pending MCP call");
    assert!(
        wire.iter().all(|entry| entry["direction"] != "received"
            || entry["frame"]["id"] != request["frame"]["id"]),
        "{record}"
    );
}

fn host(fixture: &BridgeFixture, script: &Value) -> (Process, PathBuf, HarnessGroups) {
    fixture.append_config("\n[approval]\nmode = \"ask\"\n");
    let (script, recorded) = script_files(fixture.smith.project.path(), script);
    let mut command = Command::new(stand_in_path());
    command
        .args(["--approval-host"])
        .arg(fixture.smith.project.path())
        .arg(fixture.smith.home.path())
        .current_dir(fixture.smith.project.path())
        .env_clear()
        .env("HOME", fixture.smith.home.path())
        .env("CLI_STAND_IN_SMITH_BINARY", env!("CARGO_BIN_EXE_smith"))
        .env("CLI_STAND_IN_SCRIPT", script)
        .env("CLI_STAND_IN_RECORD", &recorded)
        .env("CLI_STAND_IN_DEFAULT_TOOL_TIMEOUT_MS", "2000");
    let groups = HarnessGroups {
        record: recorded.clone(),
        group: None,
    };
    (
        Process::spawn(&mut command, fixture.deadline),
        recorded,
        groups,
    )
}

fn finished(run: &StandInRun, executed: bool) {
    assert!(run.output.status.success(), "{:?}", run.output);
    let done = run
        .lines
        .iter()
        .find(|line| line["type"] == "finished")
        .expect("host history");
    let results = done["history"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().unwrap())
        .filter(|part| {
            part["type"] == "tool_result" && part["name"] == COUNT && part["is_error"] == false
        })
        .collect::<Vec<_>>();
    assert_eq!(
        results.len(),
        usize::from(executed),
        "executed call history: {done}"
    );
    if executed {
        assert!(results[0].to_string().contains("counted:1"));
        assert_eq!(response_value(call_response(run, COUNT)), "counted:1");
    } else {
        no_received_result(&run.record);
    }
    cleaned(Path::new(run.record["server"]["args"][1].as_str().unwrap()));
}

fn approval_round_trip(codex: bool, hold_ms: u64) {
    let fixture = BridgeFixture::new(codex, false);
    let log = fixture.counting(true, false);
    let release = fixture.smith.project.path().join("finish-turn");
    let (mut process, recorded, mut groups) = host(
        &fixture,
        &json!([
            {"call":{"tool":COUNT, "arguments":{"approval":"controlled"}}},
            {"wait_file":release}, {"say":"approved result received"}
        ]),
    );
    let prompt = next_type(&mut process, "approval");
    groups.capture(&fixture.cleanup);
    assert_eq!(prompt["tool"], COUNT);
    assert!(prompt["resource"].to_string().contains("docs"), "{prompt}");
    assert!(log_lines(&log).is_empty());
    assert!(!log.with_extension("started.jsonl").exists());
    no_received_result(&record(&recorded));
    if hold_ms > 0 {
        process.send(&json!({"settle_ms":hold_ms}), "hold interactive approval");
        next_type(&mut process, "settled");
        let waiting = record(&recorded);
        assert_ne!(
            waiting["tool_timed_out"], true,
            "CLI default timeout fired: {waiting}"
        );
        no_received_result(&waiting);
        assert!(log_lines(&log).is_empty());
        assert_eq!(waiting["tool_timeout_ms"], 604_800_000_u64);
    }
    process.send(&json!({"allow":true}), "approve remote call");
    next_type(&mut process, "answered");
    // The file gate lets the process finish naturally only after this answer.
    std::fs::write(release, "finish").unwrap();
    let run = recorded_run(process.finish(Some(&recorded)), &recorded);
    finished(&run, true);
    assert_eq!(log_lines(&log), [json!({"approval":"controlled"})]);
    assert_eq!(log_lines(&log.with_extension("started.jsonl")).len(), 1);
    assert_eq!(
        run.lines
            .iter()
            .filter(|line| line["type"] == "runtime_event"
                && line["event"]["payload"]["event"] == "tool_call_completed"
                && line["event"]["payload"]["name"] == COUNT)
            .count(),
        1
    );
}

#[test]
fn bridged_remote_call_waits_for_interactive_server_attributed_approval() {
    approval_round_trip(false, 0);
}

#[test]
fn approval_outlasts_default_timeout_in_both_dialects() {
    for codex in [false, true] {
        approval_round_trip(codex, 4_000);
    }
}

fn abandoned_call(how: &str) {
    let fixture = BridgeFixture::new(false, false);
    let log = fixture.counting(true, false);
    let abandon = fixture.smith.project.path().join("abandon-after-prompt");
    let finish = fixture
        .smith
        .project
        .path()
        .join("finish-after-late-approval");
    let (mut process, recorded, mut groups) = host(
        &fixture,
        &json!([
            {"abandon":{"tool":COUNT,"arguments":{"must_not_run":true},
                "how":how,"after_ms":0,"started_file":abandon}},
            {"say":"abandonment recorded"}, {"wait_file":finish}, {"say":"abandoned safely"}
        ]),
    );
    let prompt = next_type(&mut process, "approval");
    groups.capture(&fixture.cleanup);
    assert_eq!(prompt["tool"], COUNT);
    assert!(log_lines(&log).is_empty());
    no_received_result(&record(&recorded));
    std::fs::write(abandon, "connection may now abandon").unwrap();
    let mut withdrawn = false;
    let mut recorded_abandonment = false;
    while !withdrawn || !recorded_abandonment {
        let line = process.next("withdrawal and recorded abandonment");
        assert_ne!(line["type"], "finished", "{line}");
        if line["type"] == "withdrawn" {
            assert_eq!(line["tool"], COUNT);
            withdrawn = true;
        }
        if line["event"]["payload"]["event"] == "external_text"
            && line["event"]["payload"]["text"] == "abandonment recorded"
        {
            recorded_abandonment = true;
        }
    }
    let wire = record(&recorded);
    assert!(
        wire["wire"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| if how == "cancel" {
                entry["direction"] == "sent"
                    && entry["frame"]["method"] == "notifications/cancelled"
            } else {
                entry["direction"] == "stdin_closed"
            }),
        "abandonment evidence: {wire}"
    );
    process.send(&json!({"allow":true}), "answer withdrawn prompt");
    next_type(&mut process, "answered");
    // An explicit settle period is required to observe a deferred side effect.
    // The helper keeps polling the session throughout this timer.
    process.send(&json!({"settle_ms":100}), "settle late approval");
    next_type(&mut process, "settled");
    assert!(
        log_lines(&log).is_empty(),
        "late approval dispatched an abandoned call"
    );
    assert!(!log.with_extension("started.jsonl").exists());
    std::fs::write(finish, "finish").unwrap();
    let run = recorded_run(process.finish(Some(&recorded)), &recorded);
    finished(&run, false);
    assert!(log_lines(&log).is_empty());
}

#[test]
fn cli_cancellation_withdraws_approval_and_late_allow_runs_nothing() {
    abandoned_call("cancel");
}

#[test]
fn connection_abandonment_is_observed_before_late_approval_runs_nothing() {
    abandoned_call("close");
}

fn next_external_text(process: &mut Process, expected: &str) {
    loop {
        let line = process.next(expected);
        assert_ne!(
            line["type"], "finished",
            "host ended before {expected}: {line}"
        );
        if line["event"]["payload"]["event"] == "external_text"
            && line["event"]["payload"]["text"] == expected
        {
            return;
        }
    }
}

fn question_history(run: &StandInRun, prompt: &Value, arguments: &Value, cancelled: bool) {
    assert!(run.output.status.success(), "{:?}", run.output);
    let done = run
        .lines
        .iter()
        .find(|line| line["type"] == "finished")
        .expect("host history and interaction state");
    assert_eq!(
        done["question_count"], 1,
        "exactly one broker presentation: {done}"
    );
    assert_eq!(
        done["pending_interaction"], false,
        "live session left a question pending: {done}"
    );
    let parts = done["history"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().unwrap())
        .filter(|part| part["name"] == "ask_user")
        .collect::<Vec<_>>();
    let calls = parts
        .iter()
        .filter(|part| part["type"] == "tool_call")
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1, "ordinary Smith call: {done}");
    assert_eq!(calls[0]["id"], prompt["call"]);
    assert_eq!(calls[0]["arguments"], *arguments);
    let results = parts
        .iter()
        .filter(|part| part["type"] == "tool_result")
        .collect::<Vec<_>>();
    for result in &results {
        assert_eq!(result["call_id"], calls[0]["id"]);
    }
    let events = run
        .lines
        .iter()
        .filter(|line| line["type"] == "runtime_event")
        .map(|line| &line["event"]["payload"])
        .collect::<Vec<_>>();
    let requested = events
        .iter()
        .filter(|event| event["event"] == "tool_call_requested" && event["name"] == "ask_user")
        .collect::<Vec<_>>();
    assert_eq!(requested.len(), 1, "ordinary Smith dispatch: {events:?}");
    assert_eq!(requested[0]["call"], calls[0]["id"]);
    let resolved = events
        .iter()
        .filter(|event| event["event"] == "interaction_resolved")
        .collect::<Vec<_>>();
    assert_eq!(
        resolved.len(),
        1,
        "one terminal interaction outcome: {events:?}"
    );
    assert_eq!(resolved[0]["request"], prompt["request_id"]);
    assert_eq!(resolved[0]["call"], calls[0]["id"]);
    assert_eq!(
        resolved[0]["outcome"],
        if cancelled { "cancelled" } else { "answered" }
    );
    if cancelled {
        // A cancellation error may be committed, but a late answer must not
        // become an ordinary successful questionnaire result.
        assert!(
            results.iter().all(|result| result["is_error"] == true),
            "{done}"
        );
        no_wire_result(&run.record);
        assert!(response_value(call_response(run, "agent"))["children"].is_array());
    } else {
        let answer = response_value(call_response(run, "ask_user"));
        assert_eq!(answer["outcome"], "answered");
        assert_eq!(answer["request_id"], prompt["request_id"]);
        assert_eq!(
            answer["answers"],
            json!([{"question_id":"direction", "choice_id":"small"}])
        );
        assert_eq!(run.record["last_result"], answer);
        assert_eq!(results.len(), 1, "ordinary Smith result: {done}");
        assert_eq!(results[0]["is_error"], false);
        let text = results[0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            serde_json::from_str::<Value>(&text).expect("canonical questionnaire result"),
            answer
        );
        let completed = events
            .iter()
            .filter(|event| {
                event["event"] == "tool_call_completed" && event["call"] == calls[0]["id"]
            })
            .collect::<Vec<_>>();
        assert_eq!(completed.len(), 1, "{events:?}");
        assert_eq!(completed[0]["name"], "ask_user");
        assert_eq!(completed[0]["is_error"], false);
    }
    cleaned(Path::new(run.record["server"]["args"][1].as_str().unwrap()));
}

#[test]
fn bridge_questions_answer_round_trip_and_cancellation_in_both_dialects() {
    for codex in [false, true] {
        for cancelled in [false, true] {
            let fixture = BridgeFixture::new(codex, false);
            let abandon = fixture.smith.project.path().join("cancel-open-question");
            let check = fixture.smith.project.path().join("check-late-answer");
            let finish = fixture.smith.project.path().join("finish-question-turn");
            let arguments = json!({"questions":[{
                "id":"direction", "header":"Direction", "prompt":"Which implementation direction?",
                "choices":[{"id":"small", "label":"Small", "description":"Keep the scope narrow"},
                    {"id":"large", "label":"Large", "description":"Expand the implementation"}],
                "allow_free_form":false
            }], "sensitivity":"public"});
            let script = if cancelled {
                json!([
                    {"list_tools":true},
                    {"abandon":{"tool":"ask_user", "arguments":arguments, "how":"cancel",
                        "after_ms":0, "started_file":abandon}},
                    {"say":"question cancellation sent"}, {"wait_file":check},
                    // Reading a subsequent response also drains any stray
                    // reply to the cancelled request from the real relay.
                    {"call":{"tool":"agent", "arguments":{"action":"list"}}},
                    {"say":"late answer checked"}, {"wait_file":finish}, {"say":"question turn done"}
                ])
            } else {
                json!([
                    {"list_tools":true}, {"call":{"tool":"ask_user", "arguments":arguments}},
                    {"say":"question result received"}, {"wait_file":finish}, {"say":"question turn done"}
                ])
            };
            let (mut process, recorded, mut groups) = host(&fixture, &script);
            let prompt = next_type(&mut process, "question");
            groups.capture(&fixture.cleanup);
            assert_eq!(
                prompt["questions"], arguments["questions"],
                "broker must receive the exact text and options"
            );
            let waiting = record(&recorded);
            assert_eq!(
                waiting["dialect"],
                if codex { "codex" } else { "claude-code" }
            );
            assert_eq!(
                waiting["tool_names"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|name| *name == "ask_user")
                    .count(),
                1,
                "interactive tools/list: {waiting}"
            );
            no_received_result(&waiting);
            // Keep polling the session while the stand-in awaits the answer.
            process.send(&json!({"settle_ms":100}), "hold open questionnaire");
            next_type(&mut process, "settled");
            no_received_result(&record(&recorded));
            if cancelled {
                std::fs::write(abandon, "cancel now").unwrap();
                let mut withdrawn = false;
                let mut sent = false;
                while !withdrawn || !sent {
                    let line = process.next("question withdrawal and CLI cancellation");
                    assert_ne!(line["type"], "finished", "{line}");
                    if line["type"] == "question_withdrawn" {
                        assert_eq!(line["request_id"], prompt["request_id"]);
                        assert_eq!(line["outcome"], "cancelled");
                        withdrawn = true;
                    }
                    sent |= line["event"]["payload"]["event"] == "external_text"
                        && line["event"]["payload"]["text"] == "question cancellation sent";
                }
                let abandoned = record(&recorded);
                let wire = abandoned["wire"].as_array().unwrap();
                let call = wire
                    .iter()
                    .find(|entry| {
                        entry["direction"] == "sent" && entry["frame"]["method"] == "tools/call"
                    })
                    .unwrap();
                assert!(
                    wire.iter().any(|entry| entry["direction"] == "sent"
                        && entry["frame"]["method"] == "notifications/cancelled"
                        && entry["frame"]["params"]["requestId"] == call["frame"]["id"]),
                    "{abandoned}"
                );
            }
            process.send(
                &json!({"answers":[{"question_id":"direction", "choice_id":"small"}]}),
                "answer questionnaire (deliberately late after cancellation)",
            );
            next_type(&mut process, "question_answered");
            if cancelled {
                process.send(
                    &json!({"settle_ms":100}),
                    "settle late questionnaire answer",
                );
                next_type(&mut process, "settled");
                std::fs::write(check, "read subsequent reply").unwrap();
                next_external_text(&mut process, "late answer checked");
                no_wire_result(&record(&recorded));
            } else {
                next_external_text(&mut process, "question result received");
            }
            std::fs::write(finish, "finish").unwrap();
            let run = recorded_run(process.finish(Some(&recorded)), &recorded);
            question_history(&run, &prompt, &arguments, cancelled);
        }
    }
}
