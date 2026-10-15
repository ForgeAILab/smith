//! Smith owns remote calls, secrets, trust and module authority.
use super::bridge::{
    BridgeFixture, call_response, events, history_parts, log_lines, no_tool_event, ordinary_calls,
    response_text, response_value, result, successful,
};
use super::support::run_scripted_smith;
use serde_json::json;

const SECRET: &str = "bridge-secret-must-not-reach-cli";
const COUNT: &str = "mcp__docs__count";

#[test]
fn trusted_remote_call_executes_once_with_credential_kept_out_of_cli() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        let log = fixture.counting(true, true);
        let arguments = json!({"query":"bridge authority"});
        let mut command = fixture.command(None, "allow-all");

        let run = run_scripted_smith(
            &mut command,
            fixture.smith.project.path(),
            &json!([{"list_tools":true}, {"call":{"tool":COUNT,"arguments":arguments}}, {"say":"remote done"}]),
            fixture.deadline,
        );
        successful(&run);
        assert!(
            !run.record.to_string().contains(SECRET),
            "secret appears nowhere in the CLI record"
        );
        assert!(
            run.record["tool_names"]
                .as_array()
                .unwrap()
                .contains(&json!(COUNT))
        );
        assert_eq!(log_lines(&log), [arguments]);
        let started = log_lines(&log.with_extension("started.jsonl"));
        assert_eq!(started.len(), 1);
        assert_eq!(
            started[0]["authenticated"], true,
            "server received the credential"
        );
        assert_eq!(response_value(call_response(&run, COUNT)), "counted:1");
        for record in run.invocations() {
            let invocation = json!({"argv":record["argv"], "environment":record["environment"], "server":record["server"]}).to_string();
            for withheld in [
                SECRET,
                "--counting-mcp-server",
                log.to_str().unwrap(),
                "BRIDGE_TEST_TOKEN_SOURCE",
            ] {
                assert!(
                    !invocation.contains(withheld),
                    "CLI invocation leaked {withheld}: {invocation}"
                );
            }
            assert_eq!(record["server"]["name"], "smith");
        }
        ordinary_calls(&fixture, &run, &[COUNT]);
        // Headless cannot answer a live prompt. Check the same credentialed
        // declaration at its native ask boundary, then prove no second effect.
        let blocked = fixture.run_policy(json!([
            {"call":{"tool":COUNT,"arguments":{"approval_probe":true}}}, {"say":"approval unavailable"}
        ]), None, "ask");
        assert_eq!(blocked.output.status.code(), Some(4));
        assert_eq!(result(&blocked)["status"], "approval_required");
        assert_eq!(result(&blocked)["approval_required"]["tool"], COUNT);
        assert!(
            result(&blocked)["approval_required"]["resource"]
                .to_string()
                .contains("docs")
        );
        assert_eq!(log_lines(&log).len(), 1);
        assert_eq!(log_lines(&log.with_extension("started.jsonl")).len(), 1);
        assert!(!blocked.record.to_string().contains(SECRET));
        ordinary_calls(&fixture, &blocked, &[COUNT]);
    }
}

#[test]
fn remote_approval_asks_with_server_attribution_and_denial_has_no_effect() {
    for codex in [false, true] {
        for policy in ["ask", "deny"] {
            let fixture = BridgeFixture::new(codex, false);
            let log = fixture.counting(true, false);
            let run = fixture.run_policy(json!([
                {"call":{"tool":COUNT,"arguments":{"must_not_run":true}}}, {"say":"refusal survived"}
            ]), None, policy);
            assert!(log_lines(&log).is_empty());
            assert!(
                !log.with_extension("started.jsonl").exists(),
                "no server invocation entered"
            );
            let response = call_response(&run, COUNT);
            assert_eq!(response["result"]["isError"], true, "{response}");
            if policy == "ask" {
                assert_eq!(run.output.status.code(), Some(4));
                assert_eq!(result(&run)["status"], "approval_required");
                let approval = &result(&run)["approval_required"];
                assert_eq!(approval["tool"], COUNT);
                assert!(
                    approval["resource"].to_string().contains("docs"),
                    "{approval}"
                );
                assert_eq!(approval["argument_keys"], json!(["must_not_run"]));
                assert!(!approval["permissions"].as_array().unwrap().is_empty());
            } else {
                successful(&run);
                assert!(
                    response_text(response)
                        .contains("approval declined: no approval policy configured"),
                    "{response}"
                );
            }
            if policy == "ask" {
                assert!(
                    response_text(response)
                        .contains("approval unavailable: headless approval required"),
                    "{response}"
                );
            }
            assert!(
                events(&run)
                    .iter()
                    .any(|event| event["payload"]["event"] == "external_text"
                        && event["payload"]["text"] == "refusal survived")
            );
            ordinary_calls(&fixture, &run, &[COUNT]);
        }
    }
}

#[test]
fn untrusted_server_is_neither_listed_nor_callable() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        let log = fixture.counting(false, false);
        let run = fixture.run(json!([
            {"list_tools":true}, {"call":{"tool":COUNT,"arguments":{"must_not_run":true}}}, {"say":"trust refused"}
        ]), None);
        successful(&run);
        assert_eq!(
            result(&run)["diagnostics"],
            json!(["`docs` requires interactive trust — run `/mcp trust docs`"])
        );
        assert!(
            run.record["tool_names"]
                .as_array()
                .unwrap()
                .iter()
                .all(|name| !name.as_str().unwrap().starts_with("mcp__docs__"))
        );
        assert!(call_response(&run, COUNT).get("error").is_some());
        assert!(log_lines(&log).is_empty());
        assert!(!log.with_extension("started.jsonl").exists());
        no_tool_event(&events(&run), COUNT);
        no_tool_event(
            &fixture.journal(result(&run)["session_id"].as_str().unwrap()),
            COUNT,
        );
    }
}

#[cfg(feature = "wasm-modules")]
#[test]
fn installed_module_tool_runs_under_confirmed_workspace_grants() {
    for codex in [false, true] {
        for grant_write in [false, true] {
            let fixture = BridgeFixture::new(codex, false);
            let source = fixture.smith.project.path().join("module-source");
            std::fs::create_dir_all(source.join(".smith-plugin")).unwrap();
            std::fs::write(source.join(".smith-plugin/plugin.json"), json!({
                "name":"bridge-module", "version":"1", "modules":[{
                    "wasm":"./tools.wasm", "capabilities":{"workspace_read":true,"workspace_write":grant_write}
                }]
            }).to_string()).unwrap();
            std::fs::write(
                source.join("tools.wasm"),
                include_bytes!("../../../smith-wasm-host/tests/fixtures/tools.wasm"),
            )
            .unwrap();
            let installed = fixture.cli(&["plugin", "add", source.to_str().unwrap(), "--yes"]);
            assert!(installed.status.success(), "{installed:?}");
            let inventory = smith_plugin::inspect(&source, "bridge-module").unwrap();
            assert_eq!(
                inventory.modules.len(),
                1,
                "module admission: {:?}",
                inventory.not_loaded
            );
            assert!(
                String::from_utf8_lossy(&installed.stdout).contains("Installed and trusted"),
                "{installed:?}"
            );
            let store = smith_plugin::PluginStore::new(fixture.smith.home.path().join(".smith"));
            let installed = store.list().unwrap().remove(0);
            assert_eq!(
                installed.module_grants[&inventory.modules[0].id].workspace_write,
                grant_write
            );
            // Installation confirms the bundle digest and grants. User-owned
            // switches explicitly enable the plugin and its scanned module id.
            fixture.append_config(&format!(
                "\n[plugins.bridge-module]\nenabled = true\n[modules.\"{}\"]\nenabled = true\n",
                inventory.modules[0].id
            ));
            std::fs::write(fixture.smith.project.path().join("fixture.txt"), "original").unwrap();
            // Namespace uses the component descriptor id, as the host projects it.
            let tool = "wasm_13_bridge-module_guest__probe";
            let native = fixture.native(tool, json!({"action":"fs-write"}), "allow-all");
            successful(&native);
            let native_names = native.record["requests"]
                .as_array()
                .expect("native provider requests")
                .iter()
                .flat_map(|request| {
                    request["request"]["tools"]
                        .as_array()
                        .expect("native tool view")
                })
                .map(|tool| tool["name"].clone())
                .collect::<Vec<_>>();
            assert!(
                native_names.contains(&json!(tool)),
                "module was not registered natively: {:?}; provider tools: {native_names:?}",
                native.output
            );
            let native_snapshot = fixture.snapshot(
                result(&native)["session_id"]
                    .as_str()
                    .expect("native module session"),
            );
            let native_parts = history_parts(&native_snapshot);
            let native_result = native_parts
                .iter()
                .find(|part| part["type"] == "tool_result" && part["name"] == tool)
                .expect("native module tool result");
            let native_text = native_result["content"].to_string();
            assert_eq!(native_result["is_error"], !grant_write, "{native_result}");
            assert!(
                native_text.contains(if grant_write {
                    "Ok(())"
                } else {
                    "capability denied: WorkspaceWrite"
                }),
                "{native_result}"
            );
            assert_eq!(
                std::fs::read_to_string(fixture.smith.project.path().join("fixture.txt")).unwrap(),
                if grant_write { "written" } else { "original" }
            );
            std::fs::write(fixture.smith.project.path().join("fixture.txt"), "original").unwrap();
            let run = fixture.run(json!([
                {"list_tools":true}, {"call":{"tool":tool,"arguments":{"action":"fs-write"}}}, {"say":"module done"}
            ]), None);
            successful(&run);
            // If native registration above succeeds but this list omits the
            // module, retain this failure as a bridge product defect. The
            // message records both genuine tool views rather than substituting
            // a weaker plugin-installation assertion.
            assert!(
                run.record["tool_names"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(tool)),
                "a natively registered module must be bridged: native tools {native_names:?}; bridge tools {}; native result {native_result}",
                run.record["tool_names"]
            );
            let reply = call_response(&run, tool);
            assert_eq!(reply["result"]["isError"], !grant_write, "{reply}");
            let response = response_text(reply);
            if grant_write {
                assert!(response.contains("Ok(())"), "{response}");
                assert_eq!(
                    std::fs::read_to_string(fixture.smith.project.path().join("fixture.txt"))
                        .unwrap(),
                    "written"
                );
            } else {
                assert!(
                    response.contains("capability denied: WorkspaceWrite"),
                    "{response}"
                );
                assert_eq!(
                    std::fs::read_to_string(fixture.smith.project.path().join("fixture.txt"))
                        .unwrap(),
                    "original"
                );
            }
            ordinary_calls(&fixture, &run, &[tool]);
        }
    }
}
