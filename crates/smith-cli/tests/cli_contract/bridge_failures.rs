//! Boundary failures use an independent counting-server effect log.
use super::bridge::{
    BridgeFixture, cleaned, events, history_parts, log_lines, ordinary_calls, result, socket,
    successful,
};
use super::support::{Deadline, Process, kill_group, recorded_run, script_files};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;
const SLOW: &str = "mcp__docs__slow_count";
const COUNT: &str = "mcp__docs__count";

#[test]
fn abandoning_an_executing_call_keeps_one_effect_and_committed_result() {
    for codex in [false, true] {
        for how in ["cancel", "close"] {
            let fixture = BridgeFixture::new(codex, false);
            let log = fixture.counting(true, false);
            let arguments = json!({"seconds":0.4,"marker":how});
            let run = fixture.run(
                json!([
                    {"abandon":{"tool":SLOW,"arguments":arguments,"after_ms":0,"how":how,
                        "started_file":log.with_extension("started.jsonl")}},
                    {"sleep_ms":800}, {"say":"turn continued after abandonment"}
                ]),
                None,
            );
            successful(&run);
            assert_eq!(log_lines(&log.with_extension("started.jsonl")).len(), 1);
            assert_eq!(log_lines(&log), [arguments]);
            let wire = run.record["wire"].as_array().unwrap();
            let call = wire
                .iter()
                .find(|entry| entry["frame"]["method"] == "tools/call")
                .unwrap();
            assert!(
                wire.iter().all(|entry| entry["direction"] != "received"
                    || entry["frame"]["id"] != call["frame"]["id"]),
                "CLI never read abandoned result"
            );
            if how == "cancel" {
                assert!(wire.iter().any(|entry| entry["frame"]["method"]
                    == "notifications/cancelled"
                    && entry["frame"]["params"]["requestId"] == call["frame"]["id"]));
            } else {
                assert!(
                    wire.iter()
                        .any(|entry| entry["direction"] == "stdin_closed")
                );
            }
            let snapshot = fixture.snapshot(result(&run)["session_id"].as_str().unwrap());
            let parts = history_parts(&snapshot);
            let results = parts
                .iter()
                .filter(|part| part["type"] == "tool_result" && part["name"] == SLOW)
                .collect::<Vec<_>>();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0]["is_error"], false);
            assert!(
                results[0]["content"].to_string().contains("counted:1"),
                "{snapshot}"
            );
            ordinary_calls(&fixture, &run, &[SLOW]);
        }
    }
}

#[test]
fn queued_call_cancelled_before_dispatch_has_no_effect() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        let log = fixture.counting(true, false);
        let first = json!({"seconds":0.6,"order":1});
        let run = fixture.run(
            json!([
                {"together_cancel":{"calls":[{"tool":SLOW,"arguments":first},
                    {"tool":COUNT,"arguments":{"order":2}}],"cancel_index":1,"after_ms":100}},
                {"say":"queue drained"}
            ]),
            None,
        );
        successful(&run);
        assert_eq!(log_lines(&log), [first]);
        assert_eq!(log_lines(&log.with_extension("started.jsonl")).len(), 1);
        ordinary_calls(&fixture, &run, &[SLOW]);
    }
}

#[test]
fn simultaneous_calls_execute_serially_in_wire_event_and_effect_order() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        let log = fixture.counting(true, false);
        let first = json!({"seconds":0.3,"order":1});
        let second = json!({"order":2});
        let run = fixture.run(json!([
            {"call_together":[{"tool":SLOW,"arguments":first},{"tool":COUNT,"arguments":second}]},
            {"say":"serialized"}
        ]), None);
        successful(&run);
        assert_eq!(log_lines(&log), [first, second]);
        let observed = events(&run);
        let sequence = observed
            .iter()
            .filter(|event| {
                matches!(
                    event["payload"]["event"].as_str(),
                    Some("tool_call_requested" | "tool_call_completed")
                )
            })
            .map(|event| {
                (
                    event["payload"]["event"].as_str().unwrap(),
                    event["payload"]["name"].as_str().unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            sequence,
            [
                ("tool_call_requested", SLOW),
                ("tool_call_completed", SLOW),
                ("tool_call_requested", COUNT),
                ("tool_call_completed", COUNT)
            ]
        );
        let wire = run.record["wire"].as_array().unwrap();
        let sent = wire
            .iter()
            .filter(|entry| {
                entry["direction"] == "sent" && entry["frame"]["method"] == "tools/call"
            })
            .collect::<Vec<_>>();
        assert_eq!(sent.len(), 2);
        let first_reply = wire
            .iter()
            .find(|entry| {
                entry["direction"] == "received" && entry["frame"]["id"] == sent[0]["frame"]["id"]
            })
            .unwrap();
        assert!(
            sent[1]["sequence"].as_u64().unwrap() < first_reply["sequence"].as_u64().unwrap(),
            "both requests really entered before waiting on the first response"
        );
        ordinary_calls(&fixture, &run, &[SLOW, COUNT]);
    }
}

#[test]
fn cli_exit_during_execution_fails_cleans_endpoint_and_does_not_replay_call() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        let log = fixture.counting(true, false);
        let run = fixture.run(json!([
            {"call_then_exit":{"tool":SLOW,"arguments":{"seconds":0.6,"exit_probe":true},"code":7,
                "started_file":log.with_extension("started.jsonl")}}
        ]), None);
        assert!(!run.output.status.success());
        assert_eq!(result(&run)["status"], "failed");
        assert!(
            result(&run)["error"]
                .as_str()
                .unwrap()
                .contains("exited before reporting a result")
        );
        assert_eq!(
            log_lines(&log.with_extension("started.jsonl")).len(),
            1,
            "call entered before process exit"
        );
        cleaned(&socket(&run));
        let before = log_lines(&log);
        let session = result(&run)["session_id"].as_str().unwrap();
        let resumed = fixture.run(
            json!([{"list_tools":true},{"say":"recovered without rerunning"}]),
            Some(session),
        );
        successful(&resumed);
        assert_eq!(log_lines(&log), before);
        assert_eq!(log_lines(&log.with_extension("started.jsonl")).len(), 1);
        assert!(
            events(&resumed)
                .iter()
                .all(|event| event["payload"]["event"] != "tool_call_requested")
        );
    }
}

#[test]
fn claude_without_bridge_connection_completes_with_named_diagnostic() {
    let fixture = BridgeFixture::new(false, false);
    let run = fixture.run(
        json!([{"server_never_starts":true},{"list_tools":true},{"say":"without host tools"}]),
        None,
    );
    successful(&run);
    assert_eq!(run.record["server_spawned"], false);
    assert_eq!(run.record["tool_names"], json!([]));
    assert!(run.record["wire"].as_array().unwrap().is_empty());
    let observed = events(&run);
    let diagnostic = observed
        .iter()
        .find(|event| event["payload"]["event"] == "error")
        .expect("bridge diagnostic");
    let text = diagnostic["payload"]["error"]["message"].as_str().unwrap();
    assert!(
        text.contains("Claude Code") && text.contains("did not"),
        "{text}"
    );
    assert!(
        observed
            .iter()
            .all(|event| event["payload"]["event"] != "tool_call_requested")
    );
    cleaned(&socket(&run));
}

fn wait_json(path: &Path, deadline: Deadline) -> Value {
    loop {
        deadline.remaining("published fixture record");
        if let Ok(bytes) = std::fs::read(path)
            && let Ok(value) = serde_json::from_slice(&bytes)
        {
            return value;
        }
        thread::sleep(Duration::from_millis(5));
    }
}
fn live(pid: u32, deadline: Deadline) -> bool {
    let output = deadline.work("process liveness", move || {
        super::support::run_command(
            Command::new("/bin/ps").args(["-o", "stat=", "-p", &pid.to_string()]),
            deadline,
        )
    });
    let status = String::from_utf8(output.stdout).unwrap();
    !status.trim().is_empty() && !status.trim().starts_with('Z')
}
struct HarnessCleanup {
    pid: u32,
}
impl HarnessCleanup {
    fn new(harness: u32) -> Self {
        // The supervisor leads the group; the recorded stand-in is a member.
        let group = nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(
            i32::try_from(harness).expect("harness pid"),
        )))
        .expect("live harness process group");
        Self {
            pid: u32::try_from(group.as_raw()).expect("positive group id"),
        }
    }
}
impl Drop for HarnessCleanup {
    fn drop(&mut self) {
        kill_group(self.pid);
    }
}

struct McpCleanup {
    pid: u32,
}

impl Drop for McpCleanup {
    fn drop(&mut self) {
        let pid = nix::unistd::Pid::from_raw(i32::try_from(self.pid).expect("MCP pid"));
        let _ = nix::sys::signal::killpg(pid, nix::sys::signal::Signal::SIGKILL);
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
    }
}

fn wait_counting_pid(log: &Path, deadline: Deadline) -> u32 {
    loop {
        deadline.remaining("counting server entered its tool");
        if let Ok(text) = std::fs::read_to_string(log.with_extension("started.jsonl"))
            && let Some(pid) = text
                .lines()
                .find_map(|line| serde_json::from_str::<Value>(line).ok()?["pid"].as_u64())
        {
            return u32::try_from(pid).expect("counting server pid");
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn declared_stdio_shutdown(signals: &[nix::sys::signal::Signal]) {
    let interrupt = !signals.is_empty();
    let fixture = BridgeFixture::new(false, false);
    let log = fixture.counting(true, false);
    let arguments = json!({"seconds":if interrupt {60.0} else {0.1}});
    let (script, record) = script_files(
        fixture.smith.project.path(),
        &json!([
            {"call":{"tool":SLOW,"arguments":arguments}},
            {"say":"counted before shutdown"}
        ]),
    );
    let mut command = fixture.command(None, "allow-all");
    command
        .env("CLI_STAND_IN_SCRIPT", script)
        .env("CLI_STAND_IN_RECORD", &record);
    let process = Process::spawn(&mut command, fixture.deadline);
    let pid = wait_counting_pid(&log, fixture.deadline);
    let _cleanup = McpCleanup { pid };
    if interrupt {
        assert!(live(pid, fixture.deadline), "slow server is executing");
        let server_pid = nix::unistd::Pid::from_raw(i32::try_from(pid).expect("MCP pid"));
        let smith_pid = nix::unistd::Pid::from_raw(i32::try_from(process.id()).expect("Smith pid"));
        assert_eq!(
            nix::unistd::getpgid(Some(server_pid)).expect("MCP process group"),
            nix::unistd::getpgid(Some(smith_pid)).expect("Smith process group"),
            "the declared server shares Smith's process group, so group signals still reach it"
        );
        for signal in signals {
            nix::sys::signal::kill(smith_pid, *signal).expect("interrupt sleeping MCP call");
        }
    }
    // Process::finish kills Smith's fixture group. Assert the server is gone
    // before that teardown, which otherwise masks the original stdio leak.
    let stopped = Deadline::after(Duration::from_secs(5));
    while live(process.id(), stopped) {
        stopped.remaining("Smith exit after MCP call");
        thread::sleep(Duration::from_millis(5));
    }
    while live(pid, stopped) {
        stopped.remaining("declared stdio MCP server exit");
        thread::sleep(Duration::from_millis(5));
    }
    let run = recorded_run(process.finish(Some(&record)), &record);
    assert_eq!(
        run.output.status.code(),
        Some(i32::from(interrupt)),
        "{:?}",
        run.output
    );
    if signals.len() < 2 {
        assert_eq!(
            result(&run)["status"],
            if interrupt { "cancelled" } else { "ok" }
        );
    }
    if interrupt {
        assert!(
            log_lines(&log).is_empty(),
            "interrupted tool never finished"
        );
    } else {
        assert_eq!(log_lines(&log), [arguments]);
    }
}

#[test]
fn headless_interrupt_stops_declared_stdio_mcp_server() {
    declared_stdio_shutdown(&[nix::sys::signal::Signal::SIGINT]);
}

#[test]
fn headless_termination_signals_stop_declared_stdio_mcp_server() {
    for signal in [
        nix::sys::signal::Signal::SIGTERM,
        nix::sys::signal::Signal::SIGHUP,
    ] {
        declared_stdio_shutdown(&[signal]);
    }
}

#[test]
fn headless_second_signal_stops_declared_stdio_mcp_server() {
    declared_stdio_shutdown(&[
        nix::sys::signal::Signal::SIGINT,
        nix::sys::signal::Signal::SIGTERM,
    ]);
}

#[test]
fn headless_completion_stops_declared_stdio_mcp_server() {
    declared_stdio_shutdown(&[]);
}

#[test]
fn headless_interrupt_cancels_call_and_stops_harness_process_group() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        let log = fixture.counting(true, false);
        let pids = fixture.smith.project.path().join("pids.json");
        let (script, record) = script_files(
            fixture.smith.project.path(),
            &json!([
                {"spawn_child":{"seconds":60,"pid_file":pids}},
                {"call":{"tool":SLOW,"arguments":{"seconds":60,"interrupted":true}}},
                {"call":{"tool":COUNT,"arguments":{"must_not_run_after_interrupt":true}}}
            ]),
        );
        let mut command = fixture.command(None, "allow-all");
        command
            .env("CLI_STAND_IN_SCRIPT", script)
            .env("CLI_STAND_IN_RECORD", &record);
        let process = Process::spawn(&mut command, fixture.deadline);
        let pid_values = wait_json(&pids, fixture.deadline);
        fixture.cleanup.capture();
        let endpoint_record = wait_json(&record, fixture.deadline);
        let endpoint =
            std::path::PathBuf::from(endpoint_record["server"]["args"][1].as_str().unwrap());
        let parent = u32::try_from(pid_values["stand_in_pid"].as_u64().unwrap()).unwrap();
        let child = u32::try_from(pid_values["child_pid"].as_u64().unwrap()).unwrap();
        let cleanup = HarnessCleanup::new(parent);
        while !log.with_extension("started.jsonl").exists() {
            fixture.deadline.remaining("remote call executing");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(live(parent, fixture.deadline) && live(child, fixture.deadline));
        assert!(endpoint.exists());
        let smith_pid = process.id();
        let signalled = fixture.deadline.work("send graceful interrupt", move || {
            Command::new("/bin/kill")
                .args(["-INT", &smith_pid.to_string()])
                .status()
                .unwrap()
        });
        assert!(signalled.success());
        // Check liveness before draining inherited pipes to fail promptly and
        // let the cleanup guard stop orphans even when the assertion fails.
        let stopped = Deadline::after(Duration::from_secs(5));
        loop {
            stopped.remaining("graceful interrupt must reap the harness group");
            if !live(parent, stopped) && !live(child, stopped) {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        kill_group(cleanup.pid);
        let run = recorded_run(process.finish(Some(&record)), &record);
        assert_eq!(run.output.status.code(), Some(1), "{:?}", run.output);
        assert_eq!(result(&run)["status"], "cancelled", "{:?}", run.output);
        cleaned(&endpoint);
        assert!(log_lines(&log).is_empty());
        assert_eq!(log_lines(&log.with_extension("started.jsonl")).len(), 1);
        let observed = events(&run);
        let describe = |events: &[Value]| {
            events
                .iter()
                .map(|event| format!("{}:{}", event["payload"]["event"], event["payload"]["name"]))
                .collect::<Vec<_>>()
                .join(" -> ")
        };
        let diagnostics = format!(
            "events in order: {}; stderr: {}",
            describe(&observed),
            String::from_utf8_lossy(&run.output.stderr)
        );
        let snapshot = fixture.snapshot(result(&run)["session_id"].as_str().unwrap());
        let journal = fixture.journal(result(&run)["session_id"].as_str().unwrap());
        let saved_diagnostics = format!(
            "{diagnostics}; journal events in order: {}; snapshot: {snapshot}",
            describe(&journal)
        );
        for (source, events) in [("streamed", &observed), ("journal", &journal)] {
            assert!(
                events.iter().all(|event| {
                    !(matches!(
                        event["payload"]["event"].as_str(),
                        Some("tool_call_requested" | "tool_call_completed")
                    ) && event["payload"]["name"] == COUNT)
                }),
                "counting tool was dispatched after interruption ({source}); {saved_diagnostics}"
            );
        }
        let wire = run.record["wire"].as_array().unwrap();
        let calls = wire
            .iter()
            .filter(|entry| {
                entry["direction"] == "sent" && entry["frame"]["method"] == "tools/call"
            })
            .collect::<Vec<_>>();
        assert_eq!(
            calls.first().expect("slow wire call")["frame"]["params"]["name"],
            SLOW,
            "the first call is the interrupted slow tool; {saved_diagnostics}"
        );
        assert!(
            calls.len() <= 2,
            "at most the two scripted calls are sent; {saved_diagnostics}"
        );
        if let Some(call) = calls.get(1) {
            assert_eq!(
                call["frame"]["params"]["name"], COUNT,
                "the second call is the counting tool; {saved_diagnostics}"
            );
            assert!(
                wire.iter()
                    .filter(|entry| {
                        entry["direction"] == "received"
                            && entry["frame"]["id"] == call["frame"]["id"]
                    })
                    .all(|entry| {
                        entry["frame"].get("error").is_some()
                            || entry["frame"]["result"]["isError"] == true
                    }),
                "any reply to the counting call must be an error; {saved_diagnostics}"
            );
        }
        let completed = observed.iter().find(|event| {
            event["payload"]["event"] == "tool_call_completed" && event["payload"]["name"] == SLOW
        });
        assert!(
            completed.is_some(),
            "native cancellation result missing; {saved_diagnostics}"
        );
        assert_eq!(
            completed.unwrap()["payload"]["is_error"],
            true,
            "{saved_diagnostics}"
        );
        let terminals = journal
            .iter()
            .filter(|event| {
                event["turn"] == result(&run)["turn_id"]
                    && event["payload"]["event"] == "turn_completed"
            })
            .collect::<Vec<_>>();
        assert_eq!(
            terminals.len(),
            1,
            "saved terminal events: {saved_diagnostics}"
        );
        assert_eq!(
            terminals[0]["payload"]["finish"]["outcome"], "cancelled",
            "{saved_diagnostics}"
        );
        assert!(
            history_parts(&snapshot).iter().any(|part| {
                part["type"] == "tool_result"
                    && part["name"] == SLOW
                    && part["is_error"] == true
                    && part["content"]
                        .to_string()
                        .to_lowercase()
                        .contains("cancel")
            }),
            "cancelled result missing from history; {saved_diagnostics}"
        );
        drop(cleanup);
    }
}

#[test]
fn killing_smith_mid_turn_stops_harness_and_child_without_graceful_shutdown() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        let log = fixture.counting(true, false);
        let pids = fixture.smith.project.path().join("pids.json");
        let (script, record) = script_files(
            fixture.smith.project.path(),
            &json!([
                {"spawn_child":{"seconds":60,"pid_file":pids}},
                {"call":{"tool":SLOW,"arguments":{"seconds":60,"killed":true}}}
            ]),
        );
        let mut command = fixture.command(None, "allow-all");
        command
            .env("CLI_STAND_IN_SCRIPT", script)
            .env("CLI_STAND_IN_RECORD", &record);
        let process = Process::spawn(&mut command, fixture.deadline);
        let values = wait_json(&pids, fixture.deadline);
        fixture.cleanup.capture();
        let endpoint_record = wait_json(&record, fixture.deadline);
        let endpoint =
            std::path::PathBuf::from(endpoint_record["server"]["args"][1].as_str().unwrap());
        let parent = u32::try_from(values["stand_in_pid"].as_u64().unwrap()).unwrap();
        let child = u32::try_from(values["child_pid"].as_u64().unwrap()).unwrap();
        let _cleanup = HarnessCleanup::new(parent);
        while !log.with_extension("started.jsonl").exists() {
            fixture.deadline.remaining("call in flight before SIGKILL");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(live(parent, fixture.deadline) && live(child, fixture.deadline));
        let smith_pid = process.id();
        assert!(
            fixture
                .deadline
                .work("kill Smith", move || Command::new("/bin/kill")
                    .args(["-KILL", &smith_pid.to_string()])
                    .status()
                    .unwrap())
                .success()
        );
        let stopped = Deadline::after(Duration::from_secs(5));
        loop {
            stopped.remaining("SIGKILL supervisor must stop the harness and its child");
            if !live(parent, stopped) && !live(child, stopped) {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        drop(_cleanup);
        let output = process.finish(Some(&record));
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(output.status.signal(), Some(9), "{output:?}");
        // SIGKILL cannot run Smith's socket-directory cleanup. The fixture's
        // endpoint guard removes this test's directory after these assertions.
        assert!(
            endpoint.parent().unwrap().is_dir(),
            "SIGKILL leaves the bridge directory"
        );
    }
}
