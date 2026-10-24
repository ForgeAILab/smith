use super::*;

const LIMIT_TASK_MARKER: &str = "write-the-whole-document-in-one-go";

const LIMIT_PARENT_INPUT: &str = "ask a sub-agent to write the document";

/// The text of the child's response that runs over its output limit. The
/// runtime discards that response, so none of it may reach the parent.
const CUT_OFF_TEXT: &str = "# The document, cut off mid-";

/// A parent that spawns one child and, with `read`, waits for it with the
/// `agent` tool before answering. Its child's only response runs over the
/// output limit.
#[derive(Debug)]
struct SpawnLimitedChildProvider {
    read: bool,
    parent_calls: Mutex<u32>,
    requests: Mutex<Vec<ProviderRequest>>,
    coordinator: OnceLock<agent_runtime::delegation::DelegationCoordinator>,
}

impl SpawnLimitedChildProvider {
    fn new(read: bool) -> Self {
        Self {
            read,
            parent_calls: Mutex::new(0),
            requests: Mutex::new(Vec::new()),
            coordinator: OnceLock::new(),
        }
    }

    fn parent_wires(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("provider requests poisoned")
            .iter()
            .map(|request| serde_json::to_string(&request.messages).expect("provider messages"))
            .filter(|wire| wire.contains(LIMIT_PARENT_INPUT))
            .collect()
    }
}

#[async_trait]
impl Provider for SpawnLimitedChildProvider {
    fn describe(&self) -> Vec<ModelDescriptor> {
        Vec::new()
    }

    fn capabilities(&self, _model: &agent_runtime_core::provider::ModelId) -> Option<Capabilities> {
        Some(Capabilities::basic_streaming())
    }

    async fn stream(
        &self,
        request: ProviderRequest,
        _ctx: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        let wire = serde_json::to_string(&request.messages).expect("provider messages");
        self.requests
            .lock()
            .expect("provider requests poisoned")
            .push(request);
        let answer = |text: &str, reason: FinishReason| {
            Ok(Box::pin(futures_util::stream::iter(vec![
                ProviderStreamEvent::TextDelta {
                    text: text.to_owned(),
                },
                usage_event(5, 2),
                ProviderStreamEvent::Finish { reason },
            ])) as ProviderStream)
        };
        // Only the parent's history holds the user's request.
        if !wire.contains(LIMIT_PARENT_INPUT) {
            return answer(CUT_OFF_TEXT, FinishReason::Length);
        }
        let step = {
            let mut calls = self.parent_calls.lock().expect("parent calls poisoned");
            *calls += 1;
            *calls
        };
        if step == 2 && self.read {
            while self
                .coordinator
                .get()
                .unwrap()
                .status(&agent_runtime_core::ids::ChildId::new("child-1"))
                .unwrap()
                .state
                == ChildState::Running
            {
                tokio::task::yield_now().await;
            }
        }
        let arguments = match (step, self.read) {
            (1, _) => serde_json::json!({"action": "spawn", "task": LIMIT_TASK_MARKER}),
            (2, true) => serde_json::json!({"action": "wait", "child_id": "child-1"}),
            _ => return answer("noted", FinishReason::Stop),
        };
        let mut events = tool_call_fragments(
            0,
            &format!("parent-call-{step}"),
            AGENT_TOOL_NAME,
            &arguments.to_string(),
        );
        events.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

async fn start(
    fixture: &Fixture,
    provider: Arc<SpawnLimitedChildProvider>,
) -> (
    agent_runtime::runtime::SessionHandle,
    impl Sized,
    agent_runtime::delegation::DelegationCoordinator,
) {
    let smith = Box::pin(factory::build_request(request(fixture, provider.clone())))
        .await
        .expect("a root runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a delegation surface");
    let lifecycle = wire_delegation(&session, delegation)
        .await
        .expect("delegation wires once");
    let coordinator = delegation.coordinator().expect("a coordinator").clone();
    provider.coordinator.set(coordinator.clone()).unwrap();
    (session, (smith, lifecycle), coordinator)
}

/// A child whose task stops at its output limit reaches the parent as that
/// limit, on its own, and never as a completion carrying the child's text.
#[tokio::test]
async fn a_limit_stopped_child_is_delivered_to_the_parent_as_its_limit() {
    let fixture = Fixture::new();
    let provider = Arc::new(SpawnLimitedChildProvider::new(false));
    let (session, _keep, coordinator) = start(&fixture, provider.clone()).await;

    session
        .run(UserInput::text(LIMIT_PARENT_INPUT))
        .await
        .expect("the parent turn spawns its child and ends");

    let delivery = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(wire) = provider
                .parent_wires()
                .into_iter()
                .find(|wire| wire.contains("delegation.child-completion"))
            {
                return wire;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the limit outcome reaches the parent on its own");

    assert!(
        delivery.contains("stopped at its output limit"),
        "{delivery}"
    );
    assert!(delivery.contains("gave no final answer"), "{delivery}");
    assert!(
        !delivery.contains(CUT_OFF_TEXT),
        "the discarded response must not reach the parent: {delivery}"
    );

    let status = coordinator
        .status(&agent_runtime_core::ids::ChildId::new("child-1"))
        .expect("a known child");
    assert_eq!(status.state, ChildState::Idle, "the child stays available");
    assert_eq!(status.last_result, None);
    assert_eq!(
        smith_runtime::delegation::child_status_limit(&status),
        Some("output")
    );

    // An idle limit-stopped child reports its limit, not running activity.
    let slot = Arc::new(OnceLock::new());
    slot.set(coordinator.clone()).unwrap();
    let ctx = InvocationContext {
        session: session.id().clone(),
        turn: None,
        call_id: ToolCallId::new("limit-list"),
        request: RequestId::new("limit-list"),
        workspace: Arc::new(LexicalWorkspace::new("/repo")),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 100_000,
    };
    let listed = invoke_agent(
        &AgentTool::new(slot),
        serde_json::json!({"action": "list"}),
        &ctx,
    )
    .await
    .unwrap()
    .value;
    let child = &listed["children"][0];
    assert_eq!(child["stopped_at_limit"], "output", "{listed}");
    assert!(child.get("activity").is_none(), "{listed}");

    session.shutdown().await.expect("a clean shutdown");
}

/// `wait` on a limit-stopped child reports the limit, no result, and how to
/// continue; reading it there consumes the automatic delivery.
#[tokio::test]
async fn waiting_on_a_limit_stopped_child_reports_the_limit_instead_of_a_result() {
    let fixture = Fixture::new();
    let provider = Arc::new(SpawnLimitedChildProvider::new(true));
    let (session, _keep, coordinator) = start(&fixture, provider.clone()).await;

    session
        .run(UserInput::text(LIMIT_PARENT_INPUT))
        .await
        .expect("the parent turn waits on its child and answers");

    let wires = provider.parent_wires();
    let answer = wires.last().expect("the parent answered after its wait");
    assert!(
        answer.contains(r#"\"stopped_at_limit\":\"output\""#),
        "{answer}"
    );
    assert!(answer.contains(r#"\"result\":null"#), "{answer}");
    assert!(
        answer.contains("send a follow_up to continue the work in smaller steps"),
        "{answer}"
    );
    assert!(!answer.contains(CUT_OFF_TEXT), "{answer}");
    assert!(!answer.contains(r#"\"activity\""#), "{answer}");
    assert!(
        coordinator.take_ready_task_outcomes().is_empty(),
        "the limit the model read is no longer pending automatic delivery"
    );

    session.shutdown().await.expect("a clean shutdown");
}
