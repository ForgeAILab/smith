//! Registry discovery and hash-bound skill activation on external turns.
use super::bridge::{
    BridgeFixture, call_response, ordinary_calls, response_text, response_value, successful,
};
use super::support::{Process, recorded_run, script_files};
use serde_json::{Value, json};

const BODY: &str = "BRIDGE_SKILL_INSTRUCTION: inspect ownership before changing cancellation.";
fn skill(fixture: &BridgeFixture) -> std::path::PathBuf {
    let path = fixture
        .smith
        .project
        .path()
        .join(".smith/skills/bridge-review/SKILL.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        format!("---\ndescription: Bridge review ownership cancellation\n---\n{BODY}\n"),
    )
    .unwrap();
    let executable = smith_config::trust::Executable::from_file(
        fixture.smith.project.path(),
        smith_config::trust::ExecutableKind::Skill,
        &path,
    )
    .unwrap();
    smith_config::trust::TrustStore::open(fixture.smith.home.path().join(".smith"))
        .unwrap()
        .record(
            fixture.smith.project.path(),
            &executable,
            smith_config::trust::TrustDecision::Allow,
        )
        .unwrap();
    path
}
fn activation() -> Value {
    json!({"call":{"tool":"registry.activate","arguments":{"ids":["skill:bridge-review"]}}})
}

#[test]
fn trusted_skill_is_discovered_and_its_instructions_return_over_mcp() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        skill(&fixture);
        let run = fixture.run(json!([
            {"list_tools":true},
            {"call":{"tool":"registry.search","arguments":{"query":"Bridge review ownership cancellation"}}},
            activation(), {"say":"skill received"}
        ]), None);
        successful(&run);
        let names = run.record["tool_names"].as_array().unwrap();
        for name in ["registry.search", "registry.activate"] {
            assert!(names.contains(&json!(name)));
        }
        let discovery = response_value(call_response(&run, "registry.search"));
        assert!(
            discovery["cards"]
                .as_array()
                .expect("discovery cards")
                .iter()
                .any(|card| card["id"] == json!({"domain":"skill", "name":"bridge-review"})),
            "{discovery}"
        );
        assert_eq!(discovery["available_on"], "registry.activate");
        let activation = response_value(call_response(&run, "registry.activate"));
        assert_eq!(activation["available_on"], "this_result", "{activation}");
        let entry = activation["activated"]
            .as_array()
            .expect("activated skills")
            .iter()
            .find(|entry| entry["id"] == "skill:bridge-review")
            .unwrap_or_else(|| panic!("installed skill not activated: {activation}"));
        assert!(
            entry["instructions"]
                .as_str()
                .expect("skill instructions")
                .contains(BODY),
            "{activation}"
        );
        assert_eq!(activation["rejected"], json!([]), "{activation}");
        assert!(activation["already_available"].is_array(), "{activation}");
        ordinary_calls(&fixture, &run, &["registry.search", "registry.activate"]);
    }
}

#[test]
fn edited_skill_fails_pinned_activation_and_returns_no_instruction_body() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        let path = skill(&fixture);
        // Synchronize on the CLI's text event after it has loaded the bridge.
        // Mutate *after* discovery pins the trusted bytes; changing before
        // startup only proves exclusion, not activation's content check.
        let (script, record) = script_files(
            fixture.smith.project.path(),
            &json!([
                {"list_tools":true}, {"say":"skill index pinned"}, {"wait_file":fixture.smith.project.path().join("skill-edited")},
                activation(), {"say":"activation refused"}
            ]),
        );
        let mut command = fixture.command(None, "allow-all");
        command
            .env("CLI_STAND_IN_SCRIPT", script)
            .env("CLI_STAND_IN_RECORD", &record);
        let mut process = Process::spawn(&mut command, fixture.deadline);
        loop {
            let line = process.next("skill discovery barrier");
            fixture.cleanup.capture();
            if line["event"]["payload"]["event"] == "external_text"
                && line["event"]["payload"]["text"] == "skill index pinned"
            {
                break;
            }
        }
        std::fs::write(path, "---\ndescription: Bridge review ownership cancellation\n---\nCHANGED_UNTRUSTED_INSTRUCTIONS").unwrap();
        std::fs::write(fixture.smith.project.path().join("skill-edited"), "ready").unwrap();
        let run = recorded_run(process.finish(Some(&record)), &record);
        successful(&run);
        let response = call_response(&run, "registry.activate");
        let text = response_text(response);
        let activation = response_value(response);
        assert_eq!(activation["available_on"], "this_result", "{activation}");
        assert!(
            activation["activated"]
                .as_array()
                .expect("activated skills")
                .iter()
                .all(|entry| entry["id"] != "skill:bridge-review"),
            "{activation}"
        );
        assert!(
            activation["rejected"]
                .as_array()
                .expect("rejected skills")
                .iter()
                .any(|entry| entry["id"] == "skill:bridge-review"
                    && entry["reason"]
                        .as_str()
                        .is_some_and(|reason| reason.contains("skill content digest changed"))),
            "{activation}"
        );
        // Registry activation can return per-id failures in an ordinary result.
        assert!(
            text.contains("skill content digest changed"),
            "existing trust error: {response}"
        );
        assert!(!text.contains(BODY) && !text.contains("CHANGED_UNTRUSTED_INSTRUCTIONS"));
        ordinary_calls(&fixture, &run, &["registry.activate"]);
    }
}

#[test]
fn denying_all_skills_omits_discovery_tools_but_keeps_agent() {
    for codex in [false, true] {
        let fixture = BridgeFixture::configured(
            codex,
            "delegation = true\n[profiles.dev.capabilities]\ndeny = [\"skill:*\"]",
            "",
        );
        let run = fixture.run(json!([{"list_tools":true},{"say":"no skills"}]), None);
        successful(&run);
        let names = run.record["tool_names"].as_array().unwrap();
        assert!(!names.contains(&json!("registry.search")));
        assert!(!names.contains(&json!("registry.activate")));
        assert!(names.contains(&json!("agent")), "{names:?}");
        assert!(!fixture.smith.project.path().join(".smith/skills").exists());
        assert!(!fixture.smith.home.path().join(".smith/skills").exists());
    }
}
