//! Root questionnaire contract and durable bounded output through artifact.read.
use super::bridge::{
    BridgeFixture, call_response, cleaned, history_parts, log_lines, response_text, response_value,
    result, socket, successful,
};
use serde_json::{Value, json};

#[test]
fn headless_run_offers_no_questionnaire_and_refuses_a_call_to_it() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        let native = fixture.native("agent", json!({"action":"list"}), "allow-all");
        successful(&native);
        let requests = native.record["requests"]
            .as_array()
            .expect("native requests");
        assert!(!requests.is_empty(), "{:?}", native.output);
        for request in requests {
            assert!(
                request["request"]["tools"]
                    .as_array()
                    .expect("native tool view")
                    .iter()
                    .all(|tool| tool["name"] != "ask_user"),
                "headless native provider must not see ask_user: {request}"
            );
        }
        let run = fixture.run(json!([
            {"list_tools":true},
            {"call":{"tool":"ask_user","arguments":{
                "questions":[{"id":"choice","header":"Choice","prompt":"Choose the bridge contract",
                    "choices":[{"id":"alpha","label":"Alpha"}],"allow_free_form":true}]
            }}},
            {"say":"continued without an answer"}
        ]), None);
        successful(&run);
        let envelope = result(&run);
        assert!(envelope.get("interaction_required").is_none(), "{envelope}");
        assert!(
            run.record["tool_names"]
                .as_array()
                .expect("bridge tool view")
                .iter()
                .all(|name| name != "ask_user"),
            "{}",
            run.record
        );
        let reply = call_response(&run, "ask_user");
        assert_eq!(reply["error"]["code"], -32602, "{reply}");
        assert_eq!(
            reply["error"]["message"], "unknown bridge tool: ask_user",
            "{reply}"
        );
        assert!(reply.get("result").is_none(), "no invented answer: {reply}");
        assert_eq!(envelope["output"], "continued without an answer");
        super::bridge::no_tool_event(&super::bridge::events(&run), "ask_user");
        let session = envelope["session_id"]
            .as_str()
            .expect("questionnaire session");
        let snapshot = fixture.snapshot(session);
        assert!(
            !snapshot.to_string().contains("\"selected\":\"alpha\""),
            "{snapshot}"
        );
        cleaned(&socket(&run));
        // This is the decisive pending-interaction check: a deferred native
        // interaction makes --resume inert rather than admitting new work.
        let resumed = fixture.run(
            json!([
                {"call":{"tool":"agent", "arguments":{"action":"list"}}},
                {"say":"normal resumed turn"}
            ]),
            Some(session),
        );
        successful(&resumed);
        assert_eq!(result(&resumed)["session_id"], session);
        assert_ne!(result(&resumed)["turn_id"], envelope["turn_id"]);
        assert_eq!(result(&resumed)["output"], "normal resumed turn");
        assert!(
            result(&resumed).get("interaction_required").is_none(),
            "{:?}",
            resumed.output
        );
        assert_eq!(
            response_value(call_response(&resumed, "agent"))["children"],
            json!([])
        );
        cleaned(&socket(&resumed));
    }
}

#[test]
fn bounded_remote_output_can_be_read_exactly_from_durable_artifact() {
    for codex in [false, true] {
        let fixture = BridgeFixture::new(codex, false);
        fixture.append_config("\n[context]\ntool_output_inline_bytes = 1024\n\n[limits]\ntool_output_limit_bytes = 65536\n");
        let log = fixture.counting(true, false);
        let text = format!(
            "BEGIN_EXACT_EVIDENCE\n{}\nEND_EXACT_EVIDENCE",
            "bridge-evidence\n".repeat(300)
        );
        let first = fixture.run(json!([
            {"call":{"tool":"mcp__docs__count","arguments":{"output":text}}}, {"say":"output archived"}
        ]), None);
        successful(&first);
        assert_eq!(log_lines(&log), [json!({"output":text})]);
        let bounded = response_text(call_response(&first, "mcp__docs__count"));
        assert!(bounded.len() < text.len(), "must be bounded");
        assert!(
            bounded.contains("artifact.read") && bounded.contains("artifact id="),
            "{bounded}"
        );
        let artifact = &result(&first)["artifacts"][0];
        let id = artifact["id"].as_str().expect("durable output pointer");
        assert!(bounded.contains(id), "wire pointer matches saved reference");
        let bytes = artifact["byte_length"].as_u64().unwrap();
        assert!(bytes > 1024);
        let steps = (0..bytes)
            .step_by(256)
            .map(|offset| {
                json!({"call":{
                    "tool":"artifact.read","arguments":{"id":id,"offset":offset,"limit":256}
                }})
            })
            .chain(std::iter::once(json!({"say":"exact output recovered"})))
            .collect::<Vec<_>>();
        let session = result(&first)["session_id"].as_str().unwrap();
        let second = fixture.run(json!(steps), Some(session));
        successful(&second);
        assert_ne!(socket(&first), socket(&second));
        let pages = super::bridge::responses(&second.record, "artifact.read");
        let mut recovered = String::new();
        for (index, response) in pages.iter().enumerate() {
            let page = response_value(response);
            assert_eq!(page["artifact"], id);
            assert_eq!(page["offset"], (index * 256) as u64);
            let content = page["content"].as_str().unwrap();
            assert!(content.len() <= 256);
            recovered.push_str(content);
            if index + 1 == pages.len() {
                assert!(page["next_offset"].is_null());
            } else {
                assert_eq!(page["next_offset"], ((index + 1) * 256) as u64);
            }
        }
        assert_eq!(recovered.len() as u64, bytes);
        let exact: Value = serde_json::from_str(&recovered).expect("original archived ToolOutcome");
        assert_eq!(exact["content"][0]["text"], text);
        assert_eq!(exact["is_error"], false);
        assert_eq!(
            log_lines(&log).len(),
            1,
            "artifact reads never rerun the remote tool"
        );
        let snapshot = fixture.snapshot(session);
        let parts = history_parts(&snapshot);
        assert_eq!(
            parts
                .iter()
                .filter(|part| part["type"] == "tool_call" && part["name"] == "artifact.read")
                .count(),
            pages.len()
        );
        assert_eq!(
            fixture
                .journal(session)
                .iter()
                .filter(|event| event["payload"]["event"] == "tool_call_completed"
                    && event["payload"]["name"] == "artifact.read")
                .count(),
            pages.len()
        );
        cleaned(&socket(&first));
        cleaned(&socket(&second));
    }
}
