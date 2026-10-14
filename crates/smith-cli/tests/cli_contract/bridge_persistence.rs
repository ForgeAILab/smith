//! Ordinary saved tool rounds survive resume without persisting the endpoint.
use super::bridge::{BridgeFixture, cleaned, events, history_parts, result, socket, successful};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn saved_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(saved_files(&path));
        } else {
            files.push(path);
        }
    }
    files
}
fn assert_history_order(snapshot: &Value, rounds: usize) {
    let history = snapshot["snapshot"]["history"].as_array().unwrap();
    let mut found = 0;
    for (index, message) in history.iter().enumerate() {
        for call in message["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|part| part["type"] == "tool_call")
        {
            assert_eq!(message["role"], "assistant");
            assert_eq!(call["name"], "agent");
            assert_eq!(call["arguments"], json!({"action":"list"}));
            assert!(
                message["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|part| part["type"] == "text" && part["text"] == "before saved call")
            );
            let next = &history[index + 1];
            assert_eq!(next["role"], "tool");
            let results = next["content"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|part| part["type"] == "tool_result" && part["call_id"] == call["id"])
                .collect::<Vec<_>>();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0]["is_error"], false);
            assert!(results[0]["content"].to_string().contains("children"));
            assert!(
                history[index + 2]["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|part| part["type"] == "text" && part["text"] == "after saved call")
            );
            found += 1;
        }
    }
    assert_eq!(found, rounds);
}

#[test]
fn saved_rounds_journal_listing_and_resume_use_a_fresh_working_endpoint() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        let script = json!([
            {"say":"before saved call"}, {"list_tools":true},
            {"call":{"tool":"agent","arguments":{"action":"list"}}}, {"say":"after saved call"}
        ]);
        let first = fixture.run(script.clone(), None);
        successful(&first);
        let session = result(&first)["session_id"].as_str().unwrap();
        let first_snapshot = fixture.snapshot(session);
        assert_history_order(&first_snapshot, 1);
        let listing = fixture.cli(&["sessions", "list"]);
        assert!(listing.status.success(), "{listing:?}");
        assert!(
            String::from_utf8_lossy(&listing.stdout).contains(session),
            "{listing:?}"
        );
        let second = fixture.run(script, Some(session));
        successful(&second);
        assert_eq!(result(&second)["session_id"], session);
        assert_eq!(second.record["resume"], first.record["session_id"]);
        assert_ne!(result(&second)["turn_id"], result(&first)["turn_id"]);
        let old = socket(&first);
        let new = socket(&second);
        assert_ne!(old, new);
        cleaned(&old);
        cleaned(&new);
        let snapshot = fixture.snapshot(session);
        assert_history_order(&snapshot, 2);
        let parts = history_parts(&snapshot);
        assert_eq!(
            parts
                .iter()
                .filter(|part| part["type"] == "text" && part["text"] == "after saved call")
                .count(),
            2
        );
        let journal = fixture.journal(session);
        let rounds = journal
            .iter()
            .filter(|event| {
                matches!(
                    event["payload"]["event"].as_str(),
                    Some("tool_call_requested" | "tool_call_completed")
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(rounds.len(), 4);
        for (index, pair) in rounds.chunks_exact(2).enumerate() {
            assert_eq!(pair[0]["payload"]["event"], "tool_call_requested");
            assert_eq!(pair[1]["payload"]["event"], "tool_call_completed");
            assert_eq!(pair[0]["payload"]["call"], pair[1]["payload"]["call"]);
            assert_eq!(
                pair[0]["turn"],
                if index == 0 {
                    result(&first)["turn_id"].clone()
                } else {
                    result(&second)["turn_id"].clone()
                }
            );
        }
        assert_eq!(
            events(&second)
                .iter()
                .filter(|event| event["payload"]["event"] == "tool_call_requested")
                .count(),
            1,
            "resume does not rerun the committed first round"
        );
        let first_usage = result(&first)["usage"]["session"]["input_uncached"]
            .as_u64()
            .unwrap();
        let second_usage = result(&second)["usage"]["session"]["input_uncached"]
            .as_u64()
            .unwrap();
        assert!(second_usage > first_usage, "usage survives resume");
        let files = saved_files(&fixture.smith.home.path().join(".smith/sessions"));
        assert!(
            files
                .iter()
                .any(|path| path.to_string_lossy().ends_with(".snapshot.json"))
        );
        assert!(
            files
                .iter()
                .any(|path| path.to_string_lossy().ends_with(".jsonl"))
        );
        for path in files {
            let bytes = std::fs::read(&path).unwrap();
            for endpoint in [&old, &new] {
                let needle = endpoint.to_str().unwrap().as_bytes();
                assert!(
                    !bytes.windows(needle.len()).any(|window| window == needle),
                    "persisted turn-scoped endpoint in {}",
                    path.display()
                );
                let directory = endpoint.parent().unwrap().to_str().unwrap().as_bytes();
                assert!(
                    !bytes
                        .windows(directory.len())
                        .any(|window| window == directory),
                    "persisted private endpoint directory in {}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn project_cannot_enable_owner_controlled_cli_tools() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        let kind = if codex { "codex" } else { "claude-code" };
        let directory = fixture.smith.project.path().join(".smith");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("config.toml"),
            format!("[harness.{kind}]\nallow_own_tools = true\n"),
        )
        .unwrap();
        let output = fixture.cli(&["-p", "project authority", "--output-format", "json"]);
        assert!(!output.status.success());
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains(&format!("harness.{kind}.allow_own_tools"))
                && diagnostic.contains("project"),
            "{diagnostic}"
        );
        assert!(!fixture.smith.project.path().join("record.json").exists());
        assert!(
            !fixture.smith.home.path().join(".smith/sessions").exists(),
            "refusal precedes session/CLI creation"
        );
    }
}
