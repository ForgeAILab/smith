//! Provider and harness children reached through the real bridge.
use super::support::{Process, recorded_run, script_files};
use serde_json::{Value, json};

use super::bridge::{
    BridgeFixture, cleaned, events, history_parts, no_tool_event, ordinary_calls, response_text,
    response_value, responses, result, successful,
};

// Durable children require both stores; never rely on the machine's keychain.
const CHILD: &str = r#"
[persistence]
checkpoint_key = "5151515151515151515151515151515151515151515151515151515151515151"
[profiles.worker]
provider = "local"
model = "child-model"
posture = "review"
use = ["child"]

[models."local/child-model"]
context_tokens = 64000
max_input_tokens = 60000
max_output_tokens = 2048
"#;
const REPLY: &str = "This session is running Smith's deterministic fake provider; configure a real provider to talk to a model.";

fn spawn(task: &str) -> Value {
    json!({"call":{"tool":"agent","arguments":{"action":"spawn","profile":"worker","task":task}}})
}
fn action(action: &str, pointer: &str) -> Value {
    json!({"call":{"tool":"agent","arguments":{"action":action,"child_id":{"$last":pointer}}}})
}

#[test]
fn provider_child_wait_is_acknowledged_and_all_actions_round_trip() {
    for codex in [false, true] {
        let fixture = BridgeFixture::configured(codex, "delegation = true", CHILD);
        let run = fixture.run(
            json!([
                {"list_tools":true}, spawn("review the first task"), action("wait", "/spawned"),
                {"say_last":true}, {"call":{"tool":"agent","arguments":{"action":"list"}}},
                action("result", "/children/0/child_id"),
                {"call":{"tool":"agent","arguments":{"action":"follow_up",
                    "child_id":{"$last":"/child_id"},"task":"review the second task"}}},
                action("wait", "/follow_up_sent"), action("stop", "/child_id"), {"say":"done"}
            ]),
            None,
        );
        successful(&run);
        assert_eq!(run.record["server"]["name"], "smith");
        assert_eq!(
            run.invocations().len(),
            1,
            "wait must consume both outcomes"
        );
        assert!(
            run.record["instructions"]
                .as_str()
                .unwrap()
                .contains("Delegate only a bounded, concrete subtask")
        );
        assert!(
            run.record["tool_names"]
                .as_array()
                .unwrap()
                .contains(&json!("agent"))
        );
        let calls = responses(&run.record, "agent")
            .iter()
            .map(response_value)
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 7);
        let child = calls[0]["spawned"].as_str().expect("spawned child");
        assert_eq!(calls[1]["child_id"], child);
        assert_eq!(calls[1]["state"], "idle");
        assert_eq!(calls[1]["timed_out"], false);
        assert_eq!(calls[1]["result"], REPLY);
        assert_eq!(calls[2]["children"].as_array().unwrap().len(), 1);
        assert_eq!(calls[2]["children"][0]["child_id"], child);
        assert_eq!(calls[3]["result"]["text"], REPLY);
        assert_eq!(calls[4]["follow_up_sent"], child);
        // FakeProvider consumes its single scripted reply, then emits only
        // Finish::Stop. The second turn commits no new text; agent's result
        // projection therefore still reads the last assistant reply.
        assert_eq!(calls[5]["result"], REPLY);
        assert_eq!(calls[5]["state"], "idle");
        assert_eq!(calls[5]["timed_out"], false);
        assert_eq!(calls[5]["turns_used"], 2);
        assert_eq!(calls[6]["child_id"], child);
        // Stop on an idle durable child returns the native status shape.
        assert!(calls[6]["state"].as_str().unwrap().starts_with("stopped"));
        let observed = events(&run);
        assert!(
            observed
                .iter()
                .all(|event| event["payload"]["event"] != "internal_turn_started")
        );
        assert_eq!(
            observed
                .iter()
                .filter(|event| event["payload"]["event"] == "external_session_started")
                .count(),
            1
        );
        // Read the child's real durable history, proving the selected provider
        // executed both tasks rather than merely returning a spawn identifier.
        let child_session = calls[1]["child_session_id"].as_str().unwrap();
        let snapshot = fixture.snapshot(child_session);
        let parts = history_parts(&snapshot);
        assert_eq!(calls[1]["durability"], "durable");
        assert_provider_turns(&snapshot, 2);
        assert!(observed.iter().any(|event| {
            event["payload"]["event"] == "external_text"
                && event["payload"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains(REPLY))
        }));
        for task in ["review the first task", "review the second task"] {
            assert!(
                parts
                    .iter()
                    .any(|part| part["type"] == "text" && part["text"] == task)
            );
        }
        assert_eq!(
            parts
                .iter()
                .filter(|part| part["type"] == "text" && part["text"] == REPLY)
                .count(),
            1
        );
        let second_task = parts
            .iter()
            .position(|part| part["type"] == "text" && part["text"] == "review the second task")
            .unwrap();
        assert!(
            parts[second_task + 1..]
                .iter()
                .all(|part| part["type"] != "text"
                    || part["text"].as_str().is_none_or(str::is_empty)),
            "exhausted fake must commit no second answer: {snapshot}"
        );
        ordinary_calls(&fixture, &run, &["agent"; 7]);
    }
}

#[test]
fn spawn_obeys_native_delegation_approval_and_refusal_creates_no_child() {
    for codex in [false, true] {
        for policy in ["allow-all", "deny", "ask"] {
            let fixture = BridgeFixture::configured(codex, "delegation = true", CHILD);
            let native = fixture.native(
                "agent",
                json!({"action":"spawn", "profile":"worker", "task":"approval parity"}),
                policy,
            );
            let mut script = vec![spawn("approval parity")];
            if policy == "allow-all" {
                script.push(action("wait", "/spawned"));
            } else {
                script.push(json!({"call":{"tool":"agent", "arguments":{"action":"list"}}}));
            }
            script.push(json!({"say":"approval policy survived"}));
            let run = fixture.run_policy(json!(script), None, policy);
            assert_eq!(
                run.output.status.code(),
                native.output.status.code(),
                "bridge: {:?}; native: {:?}",
                run.output,
                native.output
            );
            assert_eq!(result(&run)["status"], result(&native)["status"]);
            let replies = responses(&run.record, "agent");
            let native_snapshot = fixture.snapshot(
                result(&native)["session_id"]
                    .as_str()
                    .expect("native session"),
            );
            let native_parts = history_parts(&native_snapshot);
            let native_result = native_parts
                .iter()
                .find(|part| part["type"] == "tool_result" && part["name"] == "agent")
                .unwrap_or_else(|| panic!("native spawn result missing: {native_snapshot}"));
            let native_text = native_result["content"]
                .as_array()
                .expect("native result parts")
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if policy == "allow-all" {
                successful(&run);
                assert_eq!(replies.len(), 2);
                assert_eq!(response_value(&replies[1])["result"], REPLY);
                assert_eq!(native_result["is_error"], false);
                let native_wait = native_parts
                    .iter()
                    .filter(|part| part["type"] == "tool_result" && part["name"] == "agent")
                    .nth(1)
                    .expect("native provider child wait");
                let native_wait: Value = serde_json::from_str(
                    native_wait["content"][0]["text"]
                        .as_str()
                        .expect("native wait text"),
                )
                .expect("native wait JSON");
                assert_eq!(native_wait["result"], REPLY, "{native_wait}");
                let child = response_value(&replies[0]);
                let child_session = response_value(&replies[1]);
                assert!(child["spawned"].is_string(), "{child}");
                let child_snapshot = fixture.snapshot(
                    child_session["child_session_id"]
                        .as_str()
                        .expect("provider child session"),
                );
                assert!(
                    history_parts(&child_snapshot)
                        .iter()
                        .any(|part| part["type"] == "text" && part["text"] == REPLY)
                );
                assert_eq!(child_session["durability"], "durable");
                assert_provider_turns(&child_snapshot, 1);
            } else {
                assert_eq!(replies.len(), 2);
                assert_eq!(response_value(&replies[1])["children"], json!([]));
                let native_list = native_parts
                    .iter()
                    .filter(|part| part["type"] == "tool_result" && part["name"] == "agent")
                    .nth(1)
                    .expect("native list result");
                let text = native_list["content"][0]["text"]
                    .as_str()
                    .expect("native list text");
                assert_eq!(
                    serde_json::from_str::<Value>(text).expect("native list JSON")["children"],
                    json!([])
                );
                assert_eq!(replies[0]["result"]["isError"], true, "{}", replies[0]);
                assert_eq!(native_result["is_error"], true, "{native_result}");
                let text = response_text(&replies[0]);
                assert_eq!(
                    text, native_text,
                    "bridge refusal: {}; native refusal: {native_text}",
                    replies[0]
                );
                assert!(text.contains(if policy == "ask" {
                    "delegation approval denied: approval unavailable: headless approval required"
                } else {
                    "delegation approval denied: no approval policy configured"
                }), "{text}");
                for refused in [&native, &run] {
                    let observed = events(refused);
                    assert!(
                        !observed.iter().any(|event| matches!(
                            event["payload"]["event"].as_str(),
                            Some("child_spawned" | "child_completed")
                        )),
                        "{observed:?}"
                    );
                }
                if policy == "ask" {
                    assert_eq!(run.output.status.code(), Some(4));
                    assert_eq!(result(&run)["status"], "approval_required");
                    let approval = &result(&run)["approval_required"];
                    let native_approval = &result(&native)["approval_required"];
                    for field in [
                        "tool",
                        "permissions",
                        "argument_keys",
                        "mutates",
                        "requires_authorization",
                    ] {
                        assert_eq!(
                            approval[field], native_approval[field],
                            "{field}: bridge {approval}; native {native_approval}"
                        );
                    }
                    assert_eq!(approval["tool"], "delegation.spawn");
                    assert_eq!(approval["resource"]["kind"], "child-agent");
                    for field in ["kind", "resource_kind"] {
                        assert_eq!(
                            approval["resource"][field],
                            native_approval["resource"][field]
                        );
                    }
                    for compared in [&native, &run] {
                        let envelope = result(compared);
                        assert_eq!(
                            envelope["approval_required"]["resource"]["id"], envelope["session_id"],
                            "{envelope}"
                        );
                        assert_eq!(
                            envelope["approval_required"]["call_id"],
                            format!(
                                "delegation.spawn@{}",
                                envelope["session_id"].as_str().expect("approval parent")
                            ),
                            "{envelope}"
                        );
                    }
                    assert_eq!(approval["permissions"], json!(["agent.delegate"]));
                    assert!(
                        approval["preparation_fingerprint"]
                            .as_str()
                            .is_some_and(|text| !text.is_empty()),
                        "{approval}"
                    );
                } else {
                    successful(&run);
                }
            }
            assert_eq!(run.invocations().len(), 1);
            ordinary_calls(&fixture, &run, &["agent", "agent"]);
        }
    }
}

#[test]
fn unread_provider_outcome_gets_one_internal_resumed_harness_turn() {
    for codex in [false, true] {
        let fixture = BridgeFixture::configured(codex, "delegation = true", CHILD);
        let run = fixture.run(
            json!({
                "initial":[spawn("review unread task"), {"say":"parent yielded"}],
                "resumed":[{"list_tools":true}, {"say":"completion consumed"}]
            }),
            None,
        );
        successful(&run);
        assert_eq!(
            run.invocations().len(),
            2,
            "one root and one completion invocation"
        );
        let first = &run.invocations()[0];
        let completion = &run.invocations()[1];
        assert!(first["resume"].is_null());
        assert_eq!(completion["resume"], first["session_id"]);
        let prompt = completion["prompt"].as_str().unwrap();
        assert!(
            prompt.starts_with("[Delivered by the host: delegation.child-completion]\n"),
            "{prompt}"
        );
        assert!(prompt.contains(REPLY), "{prompt}");
        let observed = events(&run);
        let internal = observed
            .iter()
            .filter(|event| event["payload"]["event"] == "internal_turn_started")
            .collect::<Vec<_>>();
        assert_eq!(internal.len(), 1);
        assert!(
            internal[0]["payload"]["source"]
                .to_string()
                .contains("delegation.child-completion")
        );
        assert!(
            observed
                .iter()
                .all(|event| event["payload"]["event"] != "provider_attempt_started")
        );
        let session = result(&run)["session_id"].as_str().unwrap();
        let snapshot = fixture.snapshot(session);
        assert!(
            snapshot["snapshot"]["history"]
                .as_array()
                .unwrap()
                .iter()
                .all(|message| message["role"] != "user"
                    || !message.to_string().contains("Delivered by the host"))
        );
        assert_eq!(
            fixture
                .journal(session)
                .iter()
                .filter(|event| event["payload"]["event"] == "internal_turn_started")
                .count(),
            1
        );
        ordinary_calls(&fixture, &run, &["agent"]);
        // Replay and explicit user work must not schedule the consumed outcome.
        let resumed = fixture.run(
            json!([{"list_tools":true},{"say":"real new user turn"}]),
            Some(session),
        );
        successful(&resumed);
        assert_eq!(resumed.invocations().len(), 1);
        assert_eq!(resumed.record["prompt"], "bridge contract");
        assert!(
            events(&resumed)
                .iter()
                .all(|event| event["payload"]["event"] != "internal_turn_started")
        );
    }
}

#[test]
fn delegation_disabled_removes_tool_and_guidance() {
    for codex in [false, true] {
        let fixture = BridgeFixture::configured(codex, "delegation = false", "");
        let run = fixture.run(
            json!([{"list_tools":true}, {"call":{"tool":"agent","arguments":{"action":"list"}}}]),
            None,
        );
        successful(&run);
        assert!(
            !run.record["tool_names"]
                .as_array()
                .unwrap()
                .contains(&json!("agent"))
        );
        let guidance = run.record["instructions"].as_str().unwrap_or("");
        assert!(!guidance.contains("`agent`"), "{guidance}");
        assert!(
            !guidance.contains("agent") && !guidance.to_lowercase().contains("delegat"),
            "{guidance}"
        );
        assert!(responses(&run.record, "agent")[0].get("error").is_some());
        no_tool_event(&events(&run), "agent");
    }
}

#[test]
fn harness_child_receives_neither_agent_nor_advisor() {
    for codex in [false, true] {
        let child_model = if codex {
            "cli/codex/gpt-6-sol"
        } else {
            "cli/claude-code/sonnet"
        };
        // Give the child a trusted skill so its support tools keep an endpoint
        // alive; otherwise the correct empty child set has no list to inspect.
        let child_config = format!(
            "{}\n[models.\"local/advisor-model\"]\ncontext_tokens = 32000\nmax_input_tokens = 30000\nmax_output_tokens = 512\n",
            CHILD.replace("child-model", child_model).replace(
                "use = [\"child\"]",
                "use = [\"child\"]\ndelegation = true\nadvisor = \"local/advisor-model\""
            )
        );
        let fixture = BridgeFixture::configured(
            codex,
            "delegation = true\nadvisor = \"local/advisor-model\"",
            &child_config,
        );
        let root = fixture.smith.home.path().join(".smith/skills/child-probe");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("SKILL.md"),
            "---\ndescription: inspect the child bridge\n---\nChild guidance",
        )
        .unwrap();
        let run = fixture.run(json!({
            "initial":[spawn("inspect harness child"), action("wait", "/spawned"), {"say":"read child"}],
            "by_prompt":{"inspect harness child":[{"list_tools":true},{"say":"harness child ran"}]}
        }), None);
        successful(&run);
        assert_eq!(run.invocations().len(), 2);
        let child = run
            .invocations()
            .iter()
            .find(|record| record["prompt"] == "inspect harness child")
            .unwrap();
        let names = child["tool_names"].as_array().unwrap();
        assert!(!names.contains(&json!("agent")));
        assert!(!names.contains(&json!("advisor")));
        assert!(names.contains(&json!("registry.search")));
        assert!(
            run.invocations()[0]["tool_names"]
                .as_array()
                .unwrap()
                .contains(&json!("advisor"))
        );
        let calls = responses(&run.invocations()[0], "agent");
        assert_eq!(response_value(&calls[1])["result"], "harness child ran");
        for record in run.invocations() {
            cleaned(std::path::Path::new(
                record["server"]["args"][1].as_str().unwrap(),
            ));
        }
    }
}

#[test]
fn result_without_wait_acknowledges_the_terminal_outcome() {
    for codex in [false, true] {
        let fixture = BridgeFixture::configured(codex, "delegation = true", CHILD);
        let release = fixture.smith.project.path().join("child-completed");
        let (script, record) = script_files(
            fixture.smith.project.path(),
            &json!([
                spawn("read result without foreground wait"), {"wait_file":release},
                action("result", "/spawned"), {"say_last":true}
            ]),
        );
        let mut command = fixture.command(None, "allow-all");
        command
            .env("CLI_STAND_IN_SCRIPT", script)
            .env("CLI_STAND_IN_RECORD", &record);
        let mut process = Process::spawn(&mut command, fixture.deadline);
        loop {
            let line = process.next("child completion before agent result");
            fixture.cleanup.capture();
            if line["event"]["payload"]["event"] == "child_completed" {
                assert_eq!(line["event"]["payload"]["result"], REPLY);
                break;
            }
        }
        std::fs::write(release, "continue").unwrap();
        let run = recorded_run(process.finish(Some(&record)), &record);
        successful(&run);
        let replies = responses(&run.record, "agent");
        assert_eq!(response_value(&replies[1])["result"]["text"], REPLY);
        assert_eq!(run.invocations().len(), 1);
        assert!(
            events(&run)
                .iter()
                .all(|event| event["payload"]["event"] != "internal_turn_started")
        );
        ordinary_calls(&fixture, &run, &["agent", "agent"]);
    }
}

#[test]
fn resume_of_a_stopped_child_returns_native_refusal_without_starting_a_turn() {
    for codex in [false, true] {
        let fixture = BridgeFixture::configured(codex, "delegation = true", CHILD);
        let run = fixture.run(json!([
            spawn("complete then stop"), action("wait", "/spawned"), action("stop", "/child_id"),
            action("resume", "/child_id"), {"say":"resume refused safely"}
        ]), None);
        successful(&run);
        let replies = responses(&run.record, "agent");
        let waited = response_value(&replies[1]);
        assert_eq!(waited["result"], REPLY);
        assert_eq!(response_value(&replies[2])["resumable"], false);
        assert_eq!(replies[3]["result"]["isError"], true);
        assert!(
            response_text(&replies[3]).contains("has no compatible interrupted checkpoint"),
            "native stopped-child refusal: {}",
            response_text(&replies[3])
        );
        let child_session = waited["child_session_id"].as_str().unwrap();
        assert_eq!(waited["durability"], "durable");
        assert_provider_turns(&fixture.snapshot(child_session), 1);
        assert_eq!(run.invocations().len(), 1);
        ordinary_calls(&fixture, &run, &["agent"; 4]);
    }
}

/// Child snapshots share the parent's project directory; only the root has
/// a host journal. Manifests and counters prove the provider turns actually ran.
fn assert_provider_turns(snapshot: &Value, turns: u64) {
    let child = &snapshot["snapshot"];
    assert_eq!(child["identity"]["turn"], turns, "{snapshot}");
    assert_eq!(child["identity"]["attempt"], turns, "{snapshot}");
    let manifests = child["manifests"].as_array().expect("child turn manifests");
    assert_eq!(manifests.len() as u64, turns, "{snapshot}");
    for manifest in manifests {
        assert_eq!(
            manifest["manifest"]["model"]["provider"], "local",
            "{manifest}"
        );
        assert_eq!(
            manifest["manifest"]["model"]["model"], "child-model",
            "{manifest}"
        );
    }
}

#[test]
fn denied_agent_is_unlisted_and_refused_without_dispatch_or_child() {
    for codex in [false, true] {
        let fixture = BridgeFixture::configured(
            codex,
            "delegation = true\n[profiles.dev.capabilities]\ndeny = [\"tool:agent\"]",
            CHILD,
        );
        let log = fixture.counting(true, false);
        let run = fixture.run(
            json!([
                {"list_tools":true}, spawn("denied child must never exist"),
                {"call":{"tool":"mcp__docs__count", "arguments":{"allowed":true}}}
            ]),
            None,
        );
        successful(&run);
        let names = run.record["tool_names"].as_array().unwrap();
        assert!(!names.contains(&json!("agent")));
        assert!(names.contains(&json!("mcp__docs__count")));
        let refused = &responses(&run.record, "agent")[0];
        assert_eq!(refused["error"]["code"], -32602, "{refused}");
        assert_eq!(refused["error"]["message"], "unknown bridge tool: agent");
        no_tool_event(&events(&run), "agent");
        let session = result(&run)["session_id"].as_str().unwrap();
        assert!(fixture.journal(session).iter().all(|event| {
            !event["payload"]["event"]
                .as_str()
                .unwrap_or("")
                .starts_with("child_")
        }));
        assert_eq!(super::bridge::log_lines(&log), [json!({"allowed":true})]);
        ordinary_calls(&fixture, &run, &["mcp__docs__count"]);
        let root =
            super::support::session_file(fixture.smith.home.path(), session, ".snapshot.json");
        let snapshots = std::fs::read_dir(root.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with(".snapshot.json")
            })
            .collect::<Vec<_>>();
        assert_eq!(snapshots, vec![root], "no durable child was created");
    }
}
