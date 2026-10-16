//! Advisor requests, answers and usage through the real bridge.
use super::bridge::{
    BridgeFixture, argv, call_response, events, log_lines, ordinary_calls, response_value, result,
    successful,
};
use serde_json::json;

const MODELS: &str = r#"
[profiles.worker]
provider = "local"
model = "child-model"
posture = "review"
use = ["child"]
[models."local/child-model"]
context_tokens = 64000
max_input_tokens = 60000
max_output_tokens = 2048

[models."advice/advisor-model"]
context_tokens = 32000
max_input_tokens = 30000
max_output_tokens = 512
"#;

#[test]
fn recording_advisor_contains_mid_turn_transcript_and_attributed_usage() {
    for codex in [false, true] {
        for own_tools in [false, true] {
            let extra = format!("allow_own_tools = {own_tools}\n{MODELS}");
            let fixture = BridgeFixture::configured(
                codex,
                "delegation = true\nadvisor = \"advice/advisor-model\"",
                &extra,
            );
            let answer = "ADVISOR_RECORDED_ANSWER";
            let call = fixture.smith.project.path().join("advisor-answer.json");
            let requests = fixture.smith.project.path().join("advisor-requests.jsonl");
            std::fs::write(&call, json!({"answer":answer}).to_string()).unwrap();
            std::fs::write(&requests, "").unwrap();
            fixture.append_config(&format!(
                r#"
[providers.advice]
kind = "command-jsonl"
[providers.advice.command]
executable = {}
args = {}
cwd = "workspace"
"#,
                json!(super::support::stand_in_path()),
                json!(["--native-contract-provider", call, requests])
            ));
            let evidence = fixture.smith.project.path().join("evidence.txt");
            std::fs::write(&evidence, "FILE_READ_BY_CLI").unwrap();
            let mut steps = vec![json!({"list_tools":true}), json!({"say":"observed work"})];
            if own_tools {
                steps.push(json!({"own_read":evidence}));
            }
            steps.extend([
                json!({"call":{"tool":"agent","arguments":{"action":"spawn", "profile":"worker", "task":"review evidence"}}}),
                json!({"call":{"tool":"agent","arguments":{"action":"wait", "child_id":{"$last":"/spawned"}}}}),
                json!({"call":{"tool":"advisor","arguments":{}}}),
                json!({"say":"advice received"}),
            ]);
            let run = fixture.run(json!(steps), None);
            successful(&run);
            assert!(
                run.record["tool_names"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("advisor"))
            );
            assert_eq!(response_value(call_response(&run, "advisor")), answer);
            let observed = events(&run);
            let session = result(&run)["session_id"].as_str().unwrap();
            // Advisor usage is committed by its hook under the advisor purpose,
            // independently of the external harness usage.
            let journal = fixture.journal(session);
            let usage = journal
                .iter()
                .filter(|event| {
                    event["payload"]["event"] == "usage"
                        && event["payload"]["record"]["provenance"]["purpose"] == "advisor"
                })
                .collect::<Vec<_>>();
            assert_eq!(usage.len(), 1, "{journal:?}");
            assert_eq!(usage[0]["payload"]["record"]["delta"]["input_uncached"], 6);
            assert_eq!(usage[0]["payload"]["record"]["delta"]["output"], 3);
            assert_eq!(
                observed
                    .iter()
                    .filter(|event| event["payload"]["event"] == "external_tool_invoked")
                    .count(),
                usize::from(own_tools)
            );
            assert!(
                result(&run)["usage"]["session"]["input_uncached"]
                    .as_u64()
                    .unwrap()
                    > 6
            );
            ordinary_calls(&fixture, &run, &["agent", "agent", "advisor"]);
            let recorded = log_lines(&requests);
            assert_eq!(recorded.len(), 1, "one advisor request: {recorded:?}");
            let transcript = recorded[0]["request"]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|message| message["content"].as_array().unwrap())
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            for expected in [
                "bridge contract",
                "observed work",
                "Tool call: agent",
                "spawn",
                "Tool result: agent",
                "spawned",
            ] {
                assert!(
                    transcript.contains(expected),
                    "missing {expected}: {transcript}"
                );
            }
            let spawn = super::bridge::responses(&run.record, "agent");
            let spawn = response_value(&spawn[0]);
            assert!(
                transcript.contains(spawn["spawned"].as_str().unwrap()),
                "advisor must receive the actual Smith spawn result: {transcript}"
            );
            let label = "[tool run by the CLI itself, not dispatched by Smith]";
            assert_eq!(transcript.contains(label), own_tools, "{transcript}");
            if own_tools {
                assert!(transcript.contains("cat "), "{transcript}");
                assert!(transcript.contains("FILE_READ_BY_CLI"), "{transcript}");
                assert!(
                    transcript.contains(&format!(
                        "{label} {}",
                        if codex { "command_execution" } else { "Bash" }
                    )),
                    "{transcript}"
                );
            }
        }
    }
}

#[test]
fn no_advisor_omits_tool_and_guidance() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        let run = fixture.run(json!([{"list_tools":true},{"say":"no advisor"}]), None);
        successful(&run);
        assert!(
            !run.record["tool_names"]
                .as_array()
                .unwrap()
                .contains(&json!("advisor"))
        );
        assert!(
            !run.record["instructions"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains("advisor")
        );
        assert!(
            fixture
                .journal(result(&run)["session_id"].as_str().unwrap())
                .iter()
                .all(|event| event["payload"]["record"]["provenance"]["purpose"] != "advisor")
        );
    }
}

#[test]
fn removing_advisor_on_resume_preserves_recorded_cli_instructions() {
    let fixture = BridgeFixture::configured(
        false,
        "delegation = true\nadvisor = \"local/advisor-model\"",
        &MODELS.replace("advice/advisor-model", "local/advisor-model"),
    );
    let first = fixture.run(json!([{"list_tools":true},{"say":"first turn"}]), None);
    successful(&first);
    assert!(
        first.record["tool_names"]
            .as_array()
            .unwrap()
            .contains(&json!("advisor"))
    );
    let config = fixture.smith.home.path().join(".smith/config.toml");
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(
        &config,
        text.replace("advisor = \"local/advisor-model\"", ""),
    )
    .unwrap();
    let second = fixture.run(
        json!([{"list_tools":true},{"say":"second turn"}]),
        Some(result(&first)["session_id"].as_str().unwrap()),
    );
    successful(&second);
    assert_eq!(result(&second)["session_id"], result(&first)["session_id"]);
    assert_eq!(second.record["resume"], first.record["session_id"]);
    assert!(
        !second.record["tool_names"]
            .as_array()
            .unwrap()
            .contains(&json!("advisor"))
    );
    // Claude's saved prompt includes first-turn MCP guidance. Refreshing it
    // with --system-prompt-snapshot off would resend the whole CLI prompt.
    assert!(
        argv(&second)
            .iter()
            .all(|arg| !arg.contains("system-prompt"))
    );
}

#[test]
fn installed_advisor_host_inherits_the_profile_only_main_provider_without_a_program() {
    for config in [
        super::CONFIG.replace(
            "default_profile = \"dev\"",
            "default_profile = \"dev\"\nadvisor = \"cli/claude-code/opus\"",
        ),
        super::CONFIG.replace(
            "[profiles.dev]",
            "[profiles.dev]\nadvisor = \"cli/claude-code/opus\"",
        ),
    ] {
        let fixture = super::Fixture::with_config(&config);
        // The isolated process has no PATH or installed agent. Startup must
        // still compose the advisor against `local`, selected only by dev.
        let output = fixture.run(&["-p", "hello", "--output-format", "json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
