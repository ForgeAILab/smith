use super::*;

/// The full root path: spawn through the coordinator and receive the protected
/// final outcome in an attributed internal parent turn even when a one-slot
/// presentation stream cannot retain the lifecycle burst.
#[tokio::test]
async fn a_spawned_child_completes_and_its_result_reaches_the_parent_model() {
    let fixture = Fixture::new();
    let provider = scripted(3, "the child's findings");
    let mut runtime_request = request(&fixture, provider.clone());
    let project_instructions =
        ProjectInstructionsSnapshot::from_body("SHARED_PARENT_CHILD_INSTRUCTIONS")
            .expect("bounded project instructions");
    runtime_request.project_instructions = Some(project_instructions.clone());
    runtime_request.event_buffer = 1;
    let smith = factory::build_request(runtime_request)
        .await
        .expect("a runtime with a one-event presentation buffer");
    assert_eq!(
        smith.policy().project_instructions.as_ref(),
        Some(&project_instructions.identity())
    );

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a root delegation surface");
    let _lifecycle = wire_delegation(&session, delegation)
        .await
        .expect("delegation wires once");
    let coordinator = delegation.coordinator().expect("a coordinator");

    let outcome = coordinator
        .spawn(ChildSpec {
            task: UserInput::text("inspect and explain the Rust source files in this repository"),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(2),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("a spawn");
    let child = match outcome {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };

    let outcome = coordinator
        .wait_task_outcome(&child)
        .await
        .expect("a protected child outcome");
    assert!(matches!(
        &outcome,
        ChildTaskOutcome::Completed { child: id, result }
            if id == &child
                && result.text == "the child's findings"
                && result.artifacts.is_empty()
    ));
    assert_eq!(
        coordinator.status(&child).expect("a status").state,
        ChildState::Idle
    );

    // The child keeps live descriptor routing after the coordinator narrows
    // the executable view: at least one relevant read tool plus the protected
    // discovery bootstrap, and never a mutation/delegation/question surface.
    let child_request = &provider.requests()[0];
    let child_wire = serde_json::to_string(&child_request.messages).expect("child messages");
    assert!(
        child_wire.contains("SHARED_PARENT_CHILD_INSTRUCTIONS"),
        "{child_wire}"
    );
    let names: Vec<&str> = child_request
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    assert!(
        names
            .iter()
            .any(|name| matches!(*name, "read" | "list" | "search")),
        "{names:?}"
    );
    assert!(
        names
            .iter()
            .all(|name| matches!(*name, "read" | "list" | "search" | "registry.search")),
        "a narrowed child exposed a broader or orphaned descriptor: {names:?}"
    );

    wait_for_provider_requests(&provider, 2).await;
    let parent_request = provider.requests()[1].clone();
    let parent_wire = serde_json::to_string(&parent_request.messages).expect("parent messages");
    assert!(
        parent_wire.contains("SHARED_PARENT_CHILD_INSTRUCTIONS"),
        "{parent_wire}"
    );
    assert!(
        parent_wire.contains("delegation.child-completion"),
        "{parent_wire}"
    );
    assert!(parent_wire.contains(child.as_str()), "{parent_wire}");
    assert!(
        parent_wire.contains("the child's findings"),
        "{parent_wire}"
    );

    session
        .run(UserInput::text("continue after consuming the child result"))
        .await
        .expect("the later parent turn runs");
    let later_wire =
        serde_json::to_string(&provider.requests()[2].messages).expect("later parent messages");
    assert!(
        !later_wire.contains("delegation.child-completion")
            && !later_wire.contains("Protected delegated child outcomes"),
        "the ephemeral child-completion input must not enter canonical history: {later_wire}"
    );
    assert_eq!(
        coordinator
            .task_outcome(&child)
            .expect("known child")
            .expect("retained exact outcome"),
        outcome,
        "automatic delivery consumed the idempotent status outcome"
    );

    session.shutdown().await.expect("a clean shutdown");
}

/// Spawns a child from inside the parent's own turn and then finishes that
/// turn without reading the result, which is how a delegating model actually
/// leaves a ready outcome behind.
#[derive(Debug)]
struct SpawnThenFinishProvider {
    requests: Mutex<Vec<ProviderRequest>>,
}

impl SpawnThenFinishProvider {
    fn new() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ProviderRequest> {
        self.requests
            .lock()
            .expect("provider requests poisoned")
            .clone()
    }
}

#[async_trait]
impl Provider for SpawnThenFinishProvider {
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
        // The child inherits the parent's provider; its own turn is the one
        // carrying the delegated task text without an `agent` call of its own.
        let is_child = wire.contains(CHILD_TASK_MARKER) && !wire.contains(AGENT_TOOL_NAME);
        if is_child {
            return Ok(Box::pin(futures_util::stream::iter(vec![
                ProviderStreamEvent::TextDelta {
                    text: "the child's finding".to_owned(),
                },
                usage_event(5, 2),
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])));
        }
        if !wire.contains(CHILD_TASK_MARKER) {
            let mut events = tool_call_fragments(
                0,
                "spawn-one-child",
                AGENT_TOOL_NAME,
                &serde_json::json!({"action": "spawn", "task": CHILD_TASK_MARKER}).to_string(),
            );
            events.push(ProviderStreamEvent::Finish {
                reason: FinishReason::ToolCalls,
            });
            return Ok(Box::pin(futures_util::stream::iter(events)));
        }
        Ok(Box::pin(futures_util::stream::iter(vec![
            ProviderStreamEvent::TextDelta {
                text: "spawned and finished".to_owned(),
            },
            usage_event(5, 2),
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])))
    }
}

const CHILD_TASK_MARKER: &str = "inspect-the-repository-for-the-parent";

/// A parent that spawns a child inside its own turn and never reads the
/// result must still receive it, with no further user turn, tool call, or
/// runtime event to wake the admission worker.
///
/// This asserts the end state the hang violated. It does not by itself
/// reproduce the refusal race that caused it: behind an in-process fake
/// provider the parent's turn boundary frees before the worker's first
/// attempt, so that attempt is accepted. `next_admission_retry_delay` covers
/// the retry policy the race depends on.
#[tokio::test]
async fn a_child_spawned_inside_a_parent_turn_is_delivered_without_another_user_turn() {
    let fixture = Fixture::new();
    let provider = Arc::new(SpawnThenFinishProvider::new());
    let smith = factory::build_request(request(&fixture, provider.clone()))
        .await
        .expect("a root runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a delegation surface");
    let _lifecycle = wire_delegation(&session, delegation)
        .await
        .expect("delegation wires once");

    session
        .run(UserInput::text("delegate one look at the repository"))
        .await
        .expect("the parent turn completes while its child result stays undelivered");

    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if provider.requests().iter().any(|request| {
                serde_json::to_string(&request.messages)
                    .expect("provider messages")
                    .contains("delegation.child-completion")
            }) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the ready child outcome reaches the parent on its own");

    session.shutdown().await.expect("a clean shutdown");
}

const PONG_TASK_MARKER: &str = "reply-pong-to-the-parent";

const PONG_PARENT_INPUT: &str = "ask a sub-agent to reply pong";

/// Spawns a child, waits for it with the `agent` tool, optionally fetches the
/// result too, and answers in the same turn. This is the turn that used to
/// get a second, automatic delivery of the result it had already read.
#[derive(Debug)]
struct SpawnReadAnswerProvider {
    /// `"wait"` answers from the wait status; `"result"` also calls result.
    read: &'static str,
    parent_calls: Mutex<u32>,
    requests: Mutex<Vec<ProviderRequest>>,
}

impl SpawnReadAnswerProvider {
    fn new(read: &'static str) -> Self {
        Self {
            read,
            parent_calls: Mutex::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ProviderRequest> {
        self.requests
            .lock()
            .expect("provider requests poisoned")
            .clone()
    }

    fn parent_calls(&self) -> u32 {
        *self.parent_calls.lock().expect("parent calls poisoned")
    }
}

#[async_trait]
impl Provider for SpawnReadAnswerProvider {
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
        let answer = |text: &str| {
            Ok(Box::pin(futures_util::stream::iter(vec![
                ProviderStreamEvent::TextDelta {
                    text: text.to_owned(),
                },
                usage_event(5, 2),
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])) as ProviderStream)
        };
        // Only the parent's history holds the user's request.
        if !wire.contains(PONG_PARENT_INPUT) {
            return answer("pong");
        }
        let step = {
            let mut calls = self.parent_calls.lock().expect("parent calls poisoned");
            *calls += 1;
            *calls
        };
        let arguments = match (step, self.read) {
            (1, _) => serde_json::json!({"action": "spawn", "task": PONG_TASK_MARKER}),
            (2, _) => serde_json::json!({"action": "wait", "child_id": "child-1"}),
            (3, "result") => serde_json::json!({"action": "result", "child_id": "child-1"}),
            _ => return answer("pong"),
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

/// A parent that reads its child's result with `wait` or `result` and
/// answers in the same turn gets no second, automatic delivery of it.
async fn assert_a_read_child_result_is_answered_once(read: &'static str, parent_calls: u32) {
    let fixture = Fixture::new();
    let provider = Arc::new(SpawnReadAnswerProvider::new(read));
    let smith = factory::build_request(request(&fixture, provider.clone()))
        .await
        .expect("a root runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a delegation surface");
    let _lifecycle = wire_delegation(&session, delegation)
        .await
        .expect("delegation wires once");

    session
        .run(UserInput::text(PONG_PARENT_INPUT))
        .await
        .expect("the parent turn reads the result and answers");
    let coordinator = delegation.coordinator().expect("a coordinator");
    assert!(
        coordinator.take_ready_task_outcomes().is_empty(),
        "the result the model read is no longer pending automatic delivery"
    );
    assert!(
        coordinator
            .task_outcome(&agent_runtime_core::ids::ChildId::new("child-1"))
            .expect("a known child")
            .is_some(),
        "the result stays readable"
    );

    // Before the fix the admission worker started its delivery turn within
    // milliseconds of the parent turn ending.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(
        provider.parent_calls(),
        parent_calls,
        "the parent model answers once"
    );
    assert!(
        !provider.requests().iter().any(|request| {
            serde_json::to_string(&request.messages)
                .expect("provider messages")
                .contains("delegation.child-completion")
        }),
        "no automatic turn delivers the result again"
    );

    session.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn a_child_result_read_with_wait_is_not_delivered_again() {
    assert_a_read_child_result_is_answered_once("wait", 3).await;
}

#[tokio::test]
async fn a_child_result_read_with_result_is_not_delivered_again() {
    assert_a_read_child_result_is_answered_once("result", 4).await;
}

/// Runtime answers `Busy` when the parent turn boundary is still occupied.
/// The admission worker is otherwise woken only by runtime events, so a
/// refusal arriving after the last event of a run must schedule its own retry
/// — and a parent that stays occupied must back off rather than spin.
#[test]
fn a_repeated_transient_admission_refusal_backs_off_to_a_bounded_delay() {
    let first = smith_runtime::delegation::next_admission_retry_delay(None);
    assert_eq!(first, std::time::Duration::from_millis(20));

    let mut delay = first;
    let mut doublings = 0;
    while delay < std::time::Duration::from_millis(500) {
        let next = smith_runtime::delegation::next_admission_retry_delay(Some(delay));
        assert!(next > delay, "each refusal waits longer than the last");
        delay = next;
        doublings += 1;
        assert!(doublings < 16, "the backoff must reach its ceiling quickly");
    }
    assert_eq!(delay, std::time::Duration::from_millis(500));
    assert_eq!(
        smith_runtime::delegation::next_admission_retry_delay(Some(delay)),
        std::time::Duration::from_millis(500),
        "the ceiling holds instead of growing without bound"
    );
}
