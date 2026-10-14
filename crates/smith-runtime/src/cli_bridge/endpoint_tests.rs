//! Socket conformance and recorded CLI request replay.

use std::future::Future;
use std::path::Path;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use agent_runtime_core::{
    content::{ContentPart, ToolResultBlock},
    error::RuntimeError,
    ids::ToolCallId,
    tool::{ToolEffects, ToolSpec},
};
use async_trait::async_trait;
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{
    UnixStream,
    unix::{OwnedReadHalf, OwnedWriteHalf},
};
use tokio::sync::{Semaphore, mpsc};

use super::{
    BridgedSet,
    endpoint::{BridgeDispatch, BridgeEndpoint},
};

pub(super) const CLAUDE: &str = include_str!(
    "../../tests/fixtures/cli_bridge/claude-code-2.1.296/wire/handshake-and-call.jsonl"
);
pub(super) const CODEX: &str =
    include_str!("../../tests/fixtures/cli_bridge/codex-0.162.1/wire/handshake-and-call.jsonl");
const CANCELLED: &str = include_str!(
    "../../tests/fixtures/cli_bridge/claude-code-2.1.296/wire/call-cancelled-on-timeout.jsonl"
);
pub(super) const GUIDANCE: &str = "SMITH endpoint test guidance.";

pub(super) async fn deadline(future: impl Future<Output = ()>) {
    tokio::time::timeout(Duration::from_secs(15), future)
        .await
        .expect("bridge test deadline");
}

pub(super) fn recorded_in(recording: &str) -> Vec<Value> {
    recording
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|frame| frame["dir"] == "in")
        .map(|frame| frame["msg"].clone())
        .collect()
}

pub(super) fn recorded_set() -> BridgedSet {
    let frame = CLAUDE
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|frame| frame["dir"] == "out" && frame["msg"]["result"]["tools"].is_array())
        .unwrap();
    BridgedSet::new(
        frame["msg"]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| {
                ToolSpec::new(
                    tool["name"].as_str().unwrap(),
                    tool["description"].as_str().unwrap(),
                    tool["inputSchema"].clone(),
                    ToolEffects::read_only(),
                )
            })
            .collect(),
        true,
    )
}

pub(super) fn result(name: &str, text: &str) -> ToolResultBlock {
    ToolResultBlock {
        call_id: ToolCallId::new("fake"),
        name: name.into(),
        content: vec![ContentPart::text(text)],
        is_error: false,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Event {
    Started(String),
    Finished(String),
    Dropped(String),
}

pub(super) struct FakeDispatch {
    pub(super) calls: Mutex<Vec<(String, Value)>>,
    pub(super) effects: AtomicUsize,
    pub(super) gate: Semaphore,
    events: mpsc::UnboundedSender<Event>,
}

impl FakeDispatch {
    pub(super) fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<Event>) {
        let (events, receiver) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                calls: Mutex::new(vec![]),
                effects: AtomicUsize::new(0),
                gate: Semaphore::new(0),
                events,
            }),
            receiver,
        )
    }
}

struct AbandonGuard<'a> {
    name: String,
    events: &'a mpsc::UnboundedSender<Event>,
    finished: bool,
}

impl Drop for AbandonGuard<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.events.send(Event::Dropped(self.name.clone()));
        }
    }
}

#[async_trait]
impl BridgeDispatch for FakeDispatch {
    async fn call(&self, name: String, arguments: Value) -> Result<ToolResultBlock, RuntimeError> {
        self.calls
            .lock()
            .unwrap()
            .push((name.clone(), arguments.clone()));
        let label = arguments["text"].as_str().unwrap_or(&name).to_owned();
        self.events.send(Event::Started(label.clone())).unwrap();
        let mut guard = AbandonGuard {
            name: label.clone(),
            events: &self.events,
            finished: false,
        };
        if name == "slow_wait" || label == "block" {
            self.gate.acquire().await.unwrap().forget();
        }
        let outcome = if label == "fail" {
            Err(RuntimeError::approval("Smith denied this call"))
        } else if name == "fast_echo" && !arguments["text"].is_string() {
            Err(RuntimeError::tool("schema: text must be a string"))
        } else {
            self.effects.fetch_add(1, Ordering::SeqCst);
            Ok(result(
                &name,
                &format!("ECHO:{}", arguments["text"].as_str().unwrap_or("DONE")),
            ))
        };
        guard.finished = true;
        self.events.send(Event::Finished(label)).unwrap();
        outcome
    }
}

pub(super) fn bind_fake(dispatch: Arc<FakeDispatch>) -> BridgeEndpoint {
    BridgeEndpoint::bind_test(
        recorded_set(),
        dispatch,
        Path::new("/tmp"),
        Arc::new(|| rustix::process::getuid().as_raw()),
        Some(GUIDANCE.into()),
    )
    .unwrap()
}

pub(super) struct Client {
    pub(super) read: BufReader<OwnedReadHalf>,
    pub(super) write: OwnedWriteHalf,
}

impl Client {
    pub(super) async fn connect(endpoint: &BridgeEndpoint) -> Self {
        let (read, write) = UnixStream::connect(endpoint.socket_path())
            .await
            .unwrap()
            .into_split();
        Self {
            read: BufReader::new(read),
            write,
        }
    }

    pub(super) async fn send(&mut self, frame: &Value) {
        let mut bytes = serde_json::to_vec(frame).unwrap();
        bytes.push(b'\n');
        self.write.write_all(&bytes).await.unwrap();
    }

    pub(super) async fn response(&mut self, id: Value) -> Value {
        let response = self.receive().await;
        assert_eq!(response["id"], id);
        response
    }

    async fn receive(&mut self) -> Value {
        let mut line = String::new();
        assert_ne!(
            self.read.read_line(&mut line).await.unwrap(),
            0,
            "missing JSON-RPC response"
        );
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["jsonrpc"], "2.0");
        assert!(response.get("id").is_some());
        assert!(response.get("result").is_some() ^ response.get("error").is_some());
        response
    }

    pub(super) async fn initialize(&mut self) {
        let frames = recorded_in(CODEX);
        self.send(&frames[0]).await;
        let response = self.response(json!(0)).await;
        assert_eq!(response["result"]["serverInfo"]["name"], "smith");
        self.send(&frames[1]).await;
    }

    pub(super) async fn list(&mut self, id: i64) -> Value {
        self.send(&json!({"jsonrpc":"2.0", "id":id, "method":"tools/list"}))
            .await;
        self.response(json!(id)).await
    }

    pub(super) async fn call(&mut self, id: i64, name: &str, arguments: Value) {
        self.send(&json!({"jsonrpc":"2.0", "id":id, "method":"tools/call", "params":{"name":name,"arguments":arguments}})).await;
    }

    pub(super) async fn cancel(&mut self, id: i64) {
        self.send(&json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"test abandonment"}})).await;
    }
}

async fn replay(recording: &str) {
    let (dispatch, _events) = FakeDispatch::new();
    let endpoint = bind_fake(dispatch.clone());
    assert!(!endpoint.connected());
    let mut client = Client::connect(&endpoint).await;
    let expected = recorded_set();
    let mut calls = vec![];
    let mut delivered = false;
    for frame in recorded_in(recording) {
        client.send(&frame).await;
        if let Some(id) = frame.get("id") {
            let response = client.response(id.clone()).await;
            match frame["method"].as_str().unwrap() {
                "server/discover" => assert_eq!(response["error"]["code"], -32601),
                "initialize" => {
                    assert_eq!(response["result"]["instructions"], GUIDANCE);
                    assert_eq!(response["result"]["serverInfo"]["name"], "smith");
                    assert_eq!(
                        response["result"]["protocolVersion"],
                        frame["params"]["protocolVersion"]
                    );
                    assert_eq!(response["result"]["capabilities"], json!({"tools":{}}));
                    delivered = true;
                }
                "tools/list" => {
                    let tools = response["result"]["tools"].as_array().unwrap();
                    assert_eq!(tools.len(), 4);
                    for (tool, spec) in tools.iter().zip(expected.iter()) {
                        assert_eq!(tool["name"], spec.name);
                        assert_eq!(tool["description"], spec.description);
                        assert_eq!(tool["inputSchema"], spec.input_schema);
                        assert!(tool.get("annotations").is_none());
                    }
                    assert!(endpoint.connected());
                }
                "tools/call" => {
                    calls.push((
                        frame["params"]["name"].as_str().unwrap().into(),
                        frame["params"]["arguments"].clone(),
                    ));
                    assert_eq!(
                        response["result"]["content"],
                        json!([{"type":"text","text":"ECHO:hello"}])
                    );
                    assert_eq!(response["result"]["isError"], false);
                }
                method => panic!("unexpected recorded method {method}"),
            }
        }
    }
    assert!(delivered);
    assert_eq!(*dispatch.calls.lock().unwrap(), calls);
    assert_eq!(calls.len(), 1);
    assert_eq!(dispatch.effects.load(Ordering::SeqCst), 1);
    client.write.shutdown().await.unwrap();
    let mut extra = vec![];
    client.read.read_to_end(&mut extra).await.unwrap();
    assert!(
        extra.is_empty(),
        "duplicate or unsolicited responses: {extra:?}"
    );
    endpoint.close().await;
}

#[tokio::test]
async fn recorded_claude_handshake_and_call() {
    deadline(replay(CLAUDE)).await;
}

#[tokio::test]
async fn recorded_codex_handshake_and_call() {
    deadline(replay(CODEX)).await;
}

#[tokio::test]
async fn recorded_claude_timeout_drops_dispatch_future() {
    deadline(async {
        let (dispatch, mut events) = FakeDispatch::new();
        let endpoint = bind_fake(dispatch.clone());
        let mut client = Client::connect(&endpoint).await;
        for frame in recorded_in(CANCELLED) {
            client.send(&frame).await;
            match frame["method"].as_str().unwrap() {
                "tools/call" => assert_eq!(
                    events.recv().await.unwrap(),
                    Event::Started("slow_wait".into())
                ),
                "notifications/cancelled" => assert_eq!(
                    events.recv().await.unwrap(),
                    Event::Dropped("slow_wait".into())
                ),
                _ => {
                    if let Some(id) = frame.get("id") {
                        client.response(id.clone()).await;
                    }
                }
            }
        }
        dispatch.gate.add_permits(1);
        assert!(client.list(20).await["result"]["tools"].is_array());
        assert_eq!(
            dispatch.calls.lock().unwrap().as_slice(),
            &[("slow_wait".into(), json!({"seconds":25}))]
        );
        assert_eq!(dispatch.effects.load(Ordering::SeqCst), 0);
        endpoint.close().await;
    })
    .await;
}

#[tokio::test]
async fn typed_rmcp_client_lists_and_calls() {
    deadline(async {
        let (dispatch, _events) = FakeDispatch::new();
        let endpoint = bind_fake(dispatch.clone());
        let stream = UnixStream::connect(endpoint.socket_path()).await.unwrap();
        let mut client = ().serve(stream.into_split()).await.unwrap();
        assert_eq!(
            client.peer_info().unwrap().instructions.as_deref(),
            Some(GUIDANCE)
        );
        let tools = client.list_all_tools().await.unwrap();
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.name.as_ref())
                .collect::<Vec<_>>(),
            recorded_set()
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>()
        );
        let response = client
            .call_tool(
                CallToolRequestParams::new("fast_echo")
                    .with_arguments(json!({"text":"typed"}).as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_eq!(response.content[0].as_text().unwrap().text, "ECHO:typed");
        assert_eq!(
            dispatch.calls.lock().unwrap().as_slice(),
            &[("fast_echo".into(), json!({"text":"typed"}))]
        );
        assert!(endpoint.connected());
        client.close().await.unwrap();
        endpoint.close().await;
    })
    .await;
}

#[tokio::test]
async fn calls_serialize_in_arrival_order_across_connections() {
    deadline(async {
        let (dispatch, mut events) = FakeDispatch::new();
        let endpoint = bind_fake(dispatch.clone());
        let mut first = Client::connect(&endpoint).await;
        let mut other = Client::connect(&endpoint).await;
        first.initialize().await;
        other.initialize().await;
        first.call(2, "fast_echo", json!({"text":"block"})).await;
        first.call(3, "fast_echo", json!({"text":"second"})).await;
        assert_eq!(events.recv().await.unwrap(), Event::Started("block".into()));
        // A later list response proves the read loop has reserved the second
        // call while the first is still running.
        first.list(4).await;
        other.call(2, "fast_echo", json!({"text":"third"})).await;
        other.list(4).await;
        assert_eq!(dispatch.calls.lock().unwrap().len(), 1);
        dispatch.gate.add_permits(1);
        // Calls execute in order; rmcp may schedule response writes in either
        // order after both handlers have completed.
        let mut replies = [first.receive().await, first.receive().await];
        replies.sort_by_key(|response| response["id"].as_i64().unwrap());
        assert_eq!(replies[0]["id"], 2);
        assert_eq!(replies[1]["id"], 3);
        assert_eq!(replies[0]["result"]["content"][0]["text"], "ECHO:block");
        assert_eq!(replies[1]["result"]["content"][0]["text"], "ECHO:second");
        other.response(json!(2)).await;
        assert_eq!(
            events.recv().await.unwrap(),
            Event::Finished("block".into())
        );
        assert_eq!(
            events.recv().await.unwrap(),
            Event::Started("second".into())
        );
        assert_eq!(
            events.recv().await.unwrap(),
            Event::Finished("second".into())
        );
        assert_eq!(events.recv().await.unwrap(), Event::Started("third".into()));
        assert_eq!(
            events.recv().await.unwrap(),
            Event::Finished("third".into())
        );
        assert_eq!(
            dispatch
                .calls
                .lock()
                .unwrap()
                .iter()
                .map(|(_, args)| args["text"].clone())
                .collect::<Vec<_>>(),
            vec![json!("block"), json!("second"), json!("third")]
        );
        endpoint.close().await;
    })
    .await;
}

#[tokio::test]
async fn abandoned_queued_call_never_reaches_dispatcher() {
    deadline(async {
        let (dispatch, mut events) = FakeDispatch::new();
        let endpoint = bind_fake(dispatch.clone());
        let mut first = Client::connect(&endpoint).await;
        let mut other = Client::connect(&endpoint).await;
        first.initialize().await;
        other.initialize().await;
        first.call(2, "fast_echo", json!({"text":"block"})).await;
        assert_eq!(events.recv().await.unwrap(), Event::Started("block".into()));
        other
            .call(2, "fast_echo", json!({"text":"abandoned"}))
            .await;
        other.cancel(2).await;
        other.list(4).await;
        other.call(5, "fast_echo", json!({"text":"survivor"})).await;
        dispatch.gate.add_permits(1);
        first.response(json!(2)).await;
        other.response(json!(5)).await;
        assert_eq!(
            dispatch
                .calls
                .lock()
                .unwrap()
                .iter()
                .map(|(_, args)| args["text"].clone())
                .collect::<Vec<_>>(),
            vec![json!("block"), json!("survivor")]
        );
        endpoint.close().await;
    })
    .await;
}
