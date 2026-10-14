//! Failure injection observes dispatch effects and socket lifetime.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use agent_runtime_core::{
    content::{ContentPart, ToolCall},
    error::ErrorKind,
    ids::ToolCallId,
    tool::{ToolEffects, ToolSpec},
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use super::endpoint::{BridgeEndpoint, shortest_base, socket_path_limit};
use super::endpoint_tests::{
    Client, Event, FakeDispatch, bind_fake, deadline, recorded_set, result,
};
use super::transport::MAX_FRAME_BYTES;
use super::{BridgedSet, instructions};

async fn ended(client: &mut Client) {
    let mut bytes = vec![];
    let outcome = client.read.read_to_end(&mut bytes).await;
    assert!(
        outcome.is_ok()
            || outcome.is_err_and(|error| matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
            ))
    );
    assert!(bytes.is_empty(), "refused connection replied: {bytes:?}");
}

#[tokio::test]
async fn unknown_tool_is_refused_without_dispatch() {
    deadline(async {
        let (dispatch, _events) = FakeDispatch::new();
        let endpoint = bind_fake(dispatch.clone());
        let mut client = Client::connect(&endpoint).await;
        client.initialize().await;
        client.call(2, "unknown", json!({})).await;
        let response = client.response(json!(2)).await;
        assert_eq!(response["error"]["code"], -32602);
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap()
                .contains("unknown")
        );
        assert!(dispatch.calls.lock().unwrap().is_empty());
        assert_eq!(dispatch.effects.load(Ordering::SeqCst), 0);
        assert!(client.list(3).await["result"]["tools"].is_array());
        endpoint.close().await;
    })
    .await;
}

#[tokio::test]
async fn protocol_refusal_releases_its_place_in_the_call_queue() {
    deadline(async {
        let (dispatch, _events) = FakeDispatch::new();
        let endpoint = bind_fake(dispatch.clone());
        let mut client = Client::connect(&endpoint).await;
        client.initialize().await;
        client
            .send(
                &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                    "name":"fast_echo","arguments":{"text":"must not run"},
                    "_meta":{"io.modelcontextprotocol/protocolVersion":"1900-01-01"}
                }}),
            )
            .await;
        assert!(client.response(json!(2)).await["error"]["message"].is_string());
        assert!(dispatch.calls.lock().unwrap().is_empty());
        client
            .call(3, "fast_echo", json!({"text":"after protocol refusal"}))
            .await;
        assert_eq!(client.response(json!(3)).await["result"]["isError"], false);
        assert_eq!(
            dispatch.calls.lock().unwrap().as_slice(),
            &[("fast_echo".into(), json!({"text":"after protocol refusal"}))]
        );
        endpoint.close().await;
    })
    .await;
}

#[tokio::test]
async fn dispatcher_denial_and_schema_failure_are_tool_errors() {
    deadline(async {
        let (dispatch, _events) = FakeDispatch::new();
        let endpoint = bind_fake(dispatch.clone());
        let mut client = Client::connect(&endpoint).await;
        client.initialize().await;
        for (id, arguments, message) in [
            (2, json!({"text":"fail"}), "Smith denied this call"),
            (3, json!({"text":42}), "schema: text must be a string"),
        ] {
            client.call(id, "fast_echo", arguments.clone()).await;
            let response = client.response(json!(id)).await;
            assert_eq!(response["result"]["isError"], true);
            assert_eq!(
                response["result"]["content"],
                json!([{"type":"text","text":message}])
            );
            assert_eq!(
                dispatch.calls.lock().unwrap().last(),
                Some(&("fast_echo".into(), arguments))
            );
        }
        assert_eq!(dispatch.effects.load(Ordering::SeqCst), 0);
        client
            .call(4, "fast_echo", json!({"text":"still usable"}))
            .await;
        assert_eq!(client.response(json!(4)).await["result"]["isError"], false);
        assert_eq!(dispatch.calls.lock().unwrap().len(), 3);
        assert_eq!(dispatch.effects.load(Ordering::SeqCst), 1);
        endpoint.close().await;
    })
    .await;
}

#[tokio::test]
async fn peer_closing_mid_call_drops_future_before_its_effect() {
    deadline(async {
        let (dispatch, mut events) = FakeDispatch::new();
        let endpoint = bind_fake(dispatch.clone());
        let mut client = Client::connect(&endpoint).await;
        client.initialize().await;
        client.call(2, "fast_echo", json!({"text":"block"})).await;
        assert_eq!(events.recv().await.unwrap(), Event::Started("block".into()));
        drop(client);
        assert_eq!(events.recv().await.unwrap(), Event::Dropped("block".into()));
        dispatch.gate.add_permits(1);
        let mut other = Client::connect(&endpoint).await;
        other.initialize().await;
        other
            .call(2, "fast_echo", json!({"text":"next connection"}))
            .await;
        assert_eq!(other.response(json!(2)).await["result"]["isError"], false);
        assert_eq!(dispatch.effects.load(Ordering::SeqCst), 1);
        assert_eq!(dispatch.calls.lock().unwrap().len(), 2);
        endpoint.close().await;
    })
    .await;
}

async fn rejected_frame(bytes: Vec<u8>) {
    let (dispatch, _events) = FakeDispatch::new();
    let endpoint = bind_fake(dispatch.clone());
    let mut bad = Client::connect(&endpoint).await;
    let _ = bad.write.write_all(&bytes).await;
    ended(&mut bad).await;
    assert!(dispatch.calls.lock().unwrap().is_empty());
    assert!(!endpoint.connected());
    let mut good = Client::connect(&endpoint).await;
    good.initialize().await;
    good.call(2, "fast_echo", json!({"text":"healthy"})).await;
    assert_eq!(
        good.response(json!(2)).await["result"]["content"][0]["text"],
        "ECHO:healthy"
    );
    assert_eq!(
        dispatch.calls.lock().unwrap().as_slice(),
        &[("fast_echo".into(), json!({"text":"healthy"}))]
    );
    endpoint.close().await;
}

#[tokio::test]
async fn malformed_frame_ends_only_that_connection() {
    deadline(async {
        let mut bytes = b"{not-json}\n".to_vec();
        bytes.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"fast_echo\",\"arguments\":{\"text\":\"must not run\"}}}\n");
        rejected_frame(bytes).await;
    }).await;
}

#[tokio::test]
async fn oversized_frame_ends_only_that_connection() {
    deadline(async {
        let mut bytes = serde_json::to_vec(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"fast_echo","arguments":{"text":"x".repeat(MAX_FRAME_BYTES)}}}))
        .unwrap();
        bytes.push(b'\n');
        rejected_frame(bytes).await;
    })
    .await;
}

#[tokio::test]
async fn peer_uid_mismatch_is_dropped_before_handshake() {
    deadline(async {
        let (dispatch, _events) = FakeDispatch::new();
        let lookups = Arc::new(AtomicUsize::new(0));
        let seen = lookups.clone();
        let endpoint = BridgeEndpoint::bind_test(
            recorded_set(),
            dispatch.clone(),
            Path::new("/tmp"),
            Arc::new(move || {
                seen.fetch_add(1, Ordering::SeqCst);
                rustix::process::getuid().as_raw().wrapping_add(1)
            }),
            None,
        )
        .unwrap();
        let mut client = Client::connect(&endpoint).await;
        let frames = super::endpoint_tests::recorded_in(super::endpoint_tests::CODEX);
        let mut bytes = vec![];
        for frame in frames {
            bytes.extend(serde_json::to_vec(&frame).unwrap());
            bytes.push(b'\n');
        }
        let _ = client.write.write_all(&bytes).await;
        ended(&mut client).await;
        assert_eq!(lookups.load(Ordering::SeqCst), 1);
        assert!(!endpoint.connected());
        assert!(dispatch.calls.lock().unwrap().is_empty());
        assert_eq!(dispatch.effects.load(Ordering::SeqCst), 0);
        endpoint.close().await;
    })
    .await;
}

#[tokio::test]
async fn private_modes_and_close_or_drop_remove_the_endpoint() {
    deadline(async {
        for explicit_close in [true, false] {
            let (dispatch, mut events) = FakeDispatch::new();
            let endpoint = bind_fake(dispatch.clone());
            let socket = endpoint.socket_path().to_owned();
            let directory = socket.parent().unwrap().to_owned();
            assert_eq!(fs::metadata(&directory).unwrap().permissions().mode() & 0o777, 0o700);
            assert_eq!(fs::metadata(&socket).unwrap().permissions().mode() & 0o777, 0o600);
            let mut client = Client::connect(&endpoint).await;
            client.initialize().await;
            client.call(2, "fast_echo", json!({"text":"block"})).await;
            assert_eq!(events.recv().await.unwrap(), Event::Started("block".into()));
            client.call(3, "fast_echo", json!({"text":"queued at close"})).await;
            client.list(4).await;
            if explicit_close { endpoint.close().await; } else { drop(endpoint); }
            assert!(!directory.exists());
            assert!(UnixStream::connect(&socket).await.is_err());
            assert_eq!(events.recv().await.unwrap(), Event::Dropped("block".into()));
            dispatch.gate.add_permits(1);
            // The descriptor predates close. It must be shut too, so a late
            // frame on it cannot reach Smith even if the peer retained it.
            let _ = client.write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"fast_echo\",\"arguments\":{\"text\":\"late\"}}}\n").await;
            ended(&mut client).await;
            assert_eq!(dispatch.calls.lock().unwrap().len(), 1);
            assert_eq!(dispatch.effects.load(Ordering::SeqCst), 0);
        }
    }).await;
}

#[tokio::test]
async fn long_base_fails_with_config_and_leaves_no_private_directory() {
    deadline(async {
        let root = tempfile::Builder::new()
            .prefix("sb-test-")
            .tempdir_in("/tmp")
            .unwrap();
        let base = root.path().join("x".repeat(socket_path_limit()));
        fs::create_dir(&base).unwrap();
        let (dispatch, _events) = FakeDispatch::new();
        let outcome = BridgeEndpoint::bind_test(
            recorded_set(),
            dispatch.clone(),
            &base,
            Arc::new(|| rustix::process::getuid().as_raw()),
            None,
        );
        let error = match outcome {
            Ok(_) => panic!("long path accepted"),
            Err(error) => error,
        };
        assert_eq!(error.kind, ErrorKind::Config);
        assert!(error.message.contains(base.to_str().unwrap()));
        assert!(
            error
                .message
                .contains(&format!("{} bytes", socket_path_limit()))
        );
        assert_eq!(fs::read_dir(&base).unwrap().count(), 0);
        assert!(dispatch.calls.lock().unwrap().is_empty());
    })
    .await;
}

#[test]
fn base_selection_uses_shortest_existing_absolute_candidate() {
    let root = tempfile::Builder::new()
        .prefix("sb-test-")
        .tempdir_in("/tmp")
        .unwrap();
    let short = root.path().join("a");
    let long = root.path().join("a-longer-candidate");
    fs::create_dir(&short).unwrap();
    fs::create_dir(&long).unwrap();
    assert_eq!(
        shortest_base([
            long.clone(),
            PathBuf::from("."),
            root.path().join("absent"),
            short.clone()
        ]),
        Some(short)
    );
    assert_eq!(
        shortest_base([PathBuf::from("relative"), root.path().join("absent")]),
        None
    );
}

#[tokio::test]
async fn removing_directory_keeps_open_connections_working() {
    deadline(async {
        let (dispatch, _events) = FakeDispatch::new();
        let endpoint = bind_fake(dispatch.clone());
        let directory = endpoint.socket_path().parent().unwrap().to_owned();
        let mut client = Client::connect(&endpoint).await;
        client.initialize().await;
        fs::remove_dir_all(&directory).unwrap();
        client
            .call(2, "fast_echo", json!({"text":"after unlink"}))
            .await;
        assert_eq!(
            client.response(json!(2)).await["result"]["content"][0]["text"],
            "ECHO:after unlink"
        );
        assert_eq!(
            dispatch.calls.lock().unwrap().as_slice(),
            &[("fast_echo".into(), json!({"text":"after unlink"}))]
        );
        endpoint.close().await;
        assert!(!directory.exists());
    })
    .await;
}

#[tokio::test]
async fn production_guidance_is_delivered_and_empty_guidance_is_omitted() {
    deadline(async {
        for name in ["agent", "fast_echo"] {
            let set = BridgedSet::new(
                vec![ToolSpec::new(
                    name,
                    "test",
                    json!({"type":"object"}),
                    ToolEffects::read_only(),
                )],
                false,
            );
            let guidance = instructions(&set).text;
            let (dispatch, _events) = FakeDispatch::new();
            let endpoint = BridgeEndpoint::bind(set, dispatch).unwrap();
            let mut client = Client::connect(&endpoint).await;
            let frame = super::endpoint_tests::recorded_in(super::endpoint_tests::CODEX).remove(0);
            client.send(&frame).await;
            let response = client.response(json!(0)).await;
            if guidance.is_empty() {
                assert!(response["result"].get("instructions").is_none());
            } else {
                assert_eq!(response["result"]["instructions"], guidance);
            }
            assert!(!endpoint.connected());
            client.list(1).await;
            assert!(endpoint.connected());
            endpoint.close().await;
        }
    })
    .await;
}

#[test]
fn result_mapping_preserves_text_images_errors_and_names_omissions() {
    let mut block = result("tool", "text survives");
    block.is_error = true;
    block.content.extend([
        ContentPart::Image {
            url: "data:image/png;base64,AQID".into(),
            detail: None,
        },
        ContentPart::Image {
            url: "https://example.test/image.png".into(),
            detail: None,
        },
        ContentPart::Reasoning {
            text: "private reasoning".into(),
            redacted: true,
            signature: None,
            producer: None,
        },
        ContentPart::ToolCall(ToolCall {
            id: ToolCallId::new("nested"),
            name: "nested".into(),
            arguments: json!({}),
        }),
        ContentPart::ToolResult(result("nested", "nested result")),
    ]);
    let wire: Value = serde_json::to_value(super::server::map_result(block)).unwrap();
    assert_eq!(wire["isError"], true);
    assert_eq!(
        wire["content"][0],
        json!({"type":"text","text":"text survives"})
    );
    assert_eq!(
        wire["content"][1],
        json!({"type":"image","data":"AQID","mimeType":"image/png"})
    );
    for (index, omitted) in [
        (2, "image"),
        (3, "reasoning"),
        (4, "tool call"),
        (5, "tool result"),
    ] {
        assert!(
            wire["content"][index]["text"]
                .as_str()
                .unwrap()
                .contains(omitted)
        );
    }
    assert!(!wire.to_string().contains("private reasoning"));
    assert!(!wire.to_string().contains("nested result"));
}
