//! Advisor precondition: a tool can read the in-flight turn's canonical history.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
use agent_runtime::runtime::{RuntimeBuilder, SessionHandle, StartSession};
use agent_runtime_core::content::{ContentPart, Message, ToolCall, ToolResultBlock, UserInput};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::ToolCallId;
use agent_runtime_core::provider::{Capabilities, FinishReason, ModelId, ProviderStreamEvent};
use agent_runtime_core::tool::{
    InvocationContext, PreparedToolCall, Tool, ToolEffects, ToolOutcome, ToolSpec,
};
use agent_runtime_testkit::MemoryWorkspace;
use agent_runtime_testkit::scenarios::{fake_model_profile, stop_events};
use async_trait::async_trait;
use serde_json::json;

const USER_TEXT: &str = "Run probe_echo, then inspect this turn with probe_history.";
const ECHO_TEXT: &str = "fixed echo result";
const HISTORY_TEXT: &str = "history captured";
const FINAL_TEXT: &str = "done";

#[derive(Debug)]
struct ProbeEcho;

#[async_trait]
impl Tool for ProbeEcho {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "probe_echo",
            "Return a fixed text without external effects.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
            ToolEffects::default(),
        )
    }

    async fn invoke(
        &self,
        _prepared: PreparedToolCall,
        _ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        Ok(ToolOutcome::text(ECHO_TEXT))
    }
}

#[derive(Debug)]
struct ProbeHistory {
    session: Arc<OnceLock<SessionHandle>>,
    snapshot: Arc<OnceLock<Vec<Message>>>,
}

#[async_trait]
impl Tool for ProbeHistory {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "probe_history",
            "Snapshot the current session history without external effects.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
            ToolEffects::default(),
        )
    }

    async fn invoke(
        &self,
        _prepared: PreparedToolCall,
        _ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let session = self.session.get().expect("session wired before the turn");
        self.snapshot
            .set(session.history())
            .expect("probe_history is invoked exactly once");
        Ok(ToolOutcome::text(HISTORY_TEXT))
    }
}

fn tool_step(id: &str, name: &str) -> ScriptedStream {
    let mut events = tool_call_fragments(0, id, name, "{}");
    events.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    ScriptedStream::new(events)
}

#[tokio::test]
async fn mid_turn_history_contains_current_assistant_and_prior_tool_result() {
    let session_slot = Arc::new(OnceLock::new());
    let snapshot = Arc::new(OnceLock::new());
    // Separate provider steps ensure echo's result has committed before the
    // history probe runs; tools in the same parallel batch can run together.
    let provider = Arc::new(FakeProvider::new(
        "fake",
        Capabilities::basic_streaming(),
        vec![
            tool_step("call-echo", "probe_echo"),
            tool_step("call-history", "probe_history"),
            ScriptedStream::new(stop_events(FINAL_TEXT)),
        ],
    ));
    let runtime = RuntimeBuilder::new(ModelId::new("fake"))
        .model_profile(fake_model_profile())
        .provider(provider.clone())
        .workspace(Arc::new(MemoryWorkspace::new("/ws")))
        .tool(Arc::new(ProbeEcho))
        .tool(Arc::new(ProbeHistory {
            session: session_slot.clone(),
            snapshot: snapshot.clone(),
        }))
        .build()
        .expect("a runtime with two effect-free probes");
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");
    // Like AgentTool's coordinator slot, this is filled after session start
    // and before any model-driven tool invocation.
    session_slot
        .set(session.clone())
        .expect("the session slot is empty");

    tokio::time::timeout(
        Duration::from_secs(5),
        session.run(UserInput::text(USER_TEXT)),
    )
    .await
    .expect("the scripted turn does not hang while reading history")
    .expect("the turn is accepted");

    let mut expected = vec![
        Message::user(USER_TEXT),
        Message::assistant(vec![ContentPart::ToolCall(ToolCall {
            id: ToolCallId::new("call-echo"),
            name: "probe_echo".to_owned(),
            arguments: json!({}),
        })]),
        Message::tool_result(ToolResultBlock {
            call_id: ToolCallId::new("call-echo"),
            name: "probe_echo".to_owned(),
            content: vec![ContentPart::text(ECHO_TEXT)],
            is_error: false,
        }),
        Message::assistant(vec![ContentPart::ToolCall(ToolCall {
            id: ToolCallId::new("call-history"),
            name: "probe_history".to_owned(),
            arguments: json!({}),
        })]),
    ];
    // This snapshot was taken inside invoke, before probe_history returned.
    // Exact equality checks roles, order, call IDs, arguments, and result text,
    // and excludes the probe's own result and the later final assistant text.
    assert_eq!(
        snapshot.get().expect("probe_history captured history"),
        &expected,
        "the in-flight history contains the current assistant call and all earlier results",
    );

    expected.extend([
        Message::tool_result(ToolResultBlock {
            call_id: ToolCallId::new("call-history"),
            name: "probe_history".to_owned(),
            content: vec![ContentPart::text(HISTORY_TEXT)],
            is_error: false,
        }),
        Message::assistant(vec![ContentPart::text(FINAL_TEXT)]),
    ]);
    assert_eq!(session.history(), expected, "both probes succeeded");
    assert_eq!(provider.requests().len(), 3, "one turn used three steps");
    session.shutdown().await.expect("a clean shutdown");
}
