use super::*;
use agent_runtime::runtime::SessionHandle;
use smith_runtime::delegation::DelegationLifecycle;
use std::time::Duration;

#[derive(Debug)]
struct WaitProvider {
    requests: Mutex<Vec<String>>,
    child_release: tokio::sync::Semaphore,
    user_release: tokio::sync::Semaphore,
    /// A child first calls `list`, then streams a draft until released.
    tool_step: bool,
    draft_release: Arc<tokio::sync::Semaphore>,
}

impl Default for WaitProvider {
    fn default() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            child_release: tokio::sync::Semaphore::new(0),
            user_release: tokio::sync::Semaphore::new(0),
            tool_step: false,
            draft_release: Arc::new(tokio::sync::Semaphore::new(0)),
        }
    }
}

const DRAFT: &str = "Drafting the answer";

#[async_trait]
impl Provider for WaitProvider {
    fn describe(&self) -> Vec<ModelDescriptor> {
        Vec::new()
    }
    fn capabilities(&self, _: &agent_runtime_core::provider::ModelId) -> Option<Capabilities> {
        Some(Capabilities::basic_streaming())
    }
    async fn stream(
        &self,
        request: ProviderRequest,
        _: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        let wire = serde_json::to_string(&request.messages).unwrap();
        let child = wire.contains("controlled child") && !wire.contains("parent boundary");
        let busy = wire.contains("busy parent") && !wire.contains("delegation.");
        self.requests.lock().unwrap().push(wire.clone());
        if child && self.tool_step {
            if !wire.contains("progress-list") {
                return Ok(Box::pin(futures_util::stream::iter(vec![
                    ProviderStreamEvent::ToolCallDelta {
                        index: 0,
                        id: Some("progress-list".into()),
                        name: Some("list".into()),
                        arguments_fragment: r#"{"path":"."}"#.into(),
                    },
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::ToolCalls,
                    },
                ])));
            }
            let release = self.draft_release.clone();
            return Ok(Box::pin(
                futures_util::stream::iter(vec![ProviderStreamEvent::TextDelta {
                    text: DRAFT.into(),
                }])
                .chain(futures_util::stream::once(async move {
                    release.acquire().await.unwrap().forget();
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    }
                })),
            ));
        }
        if child {
            self.child_release.acquire().await.unwrap().forget();
        } else if busy {
            self.user_release.acquire().await.unwrap().forget();
        }
        Ok(Box::pin(futures_util::stream::iter(vec![
            ProviderStreamEvent::TextDelta {
                text: if child {
                    "private child result"
                } else {
                    "parent done"
                }
                .into(),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])))
    }
}

struct WaitFixture {
    _fixture: Fixture,
    runtime: factory::SmithRuntime,
    session: SessionHandle,
    lifecycle: DelegationLifecycle,
    tool: AgentTool,
    provider: Arc<WaitProvider>,
}

impl WaitFixture {
    async fn new() -> Self {
        Self::with_provider(WaitProvider::default()).await
    }

    async fn with_tool_step() -> Self {
        Self::with_provider(WaitProvider {
            tool_step: true,
            ..WaitProvider::default()
        })
        .await
    }

    async fn with_provider(provider: WaitProvider) -> Self {
        let fixture = Fixture::new();
        let provider = Arc::new(provider);
        let runtime = factory::build_request(request(&fixture, provider.clone()))
            .await
            .unwrap();
        let session = runtime
            .runtime()
            .start_session(StartSession::new())
            .await
            .unwrap();
        let delegation = runtime.delegation().unwrap();
        let lifecycle = wire_delegation(&session, delegation).await.unwrap();
        let slot = Arc::new(OnceLock::new());
        slot.set(delegation.coordinator().unwrap().clone()).unwrap();
        let tool = AgentTool::new(slot).with_wait_timers(delegation.wait_timers().clone());
        // Establish a real parent boundary so parking has the same starting
        // state as a user turn that spawned a child.
        session
            .run(UserInput::text("parent boundary"))
            .await
            .unwrap();
        Self {
            _fixture: fixture,
            runtime,
            session,
            lifecycle,
            tool,
            provider,
        }
    }

    async fn call(&self, arguments: serde_json::Value) -> serde_json::Value {
        let ctx = InvocationContext {
            session: self.session.id().clone(),
            turn: None,
            call_id: ToolCallId::new("wait-test"),
            request: RequestId::new("wait-test"),
            workspace: Arc::new(LexicalWorkspace::new("/repo")),
            clock: Arc::new(SystemClock),
            cancel: Cancellation::new(),
            deadline: Deadline::never(),
            output_limit: 100_000,
        };
        invoke_agent(&self.tool, arguments, &ctx)
            .await
            .unwrap()
            .value
    }

    async fn spawn(&self) -> String {
        self.call(serde_json::json!({"action":"spawn", "task":"controlled child"}))
            .await["spawned"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn wait(&self, child: &str, timeout: u64) -> serde_json::Value {
        self.call(serde_json::json!({"action":"wait", "child_id":child, "timeout_ms":timeout}))
            .await
    }

    fn wakes(&self, kind: &str) -> Vec<String> {
        self.provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|wire| wire.contains(kind))
            .cloned()
            .collect()
    }

    async fn settle(&self) {
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
    }

    async fn advance(&self, millis: u64) {
        tokio::time::advance(Duration::from_millis(millis)).await;
        self.settle().await;
    }

    /// Polls until the child's draft is streaming, after its `list` step.
    async fn streaming_status(&self, child: &str) -> serde_json::Value {
        for _ in 0..200 {
            let status = self.wait(child, 0).await;
            if status["activity"]["streaming"]["text_chars"] == DRAFT.len() {
                return status;
            }
            self.advance(10).await;
        }
        panic!(
            "child never streamed its draft: {}",
            self.wait(child, 0).await
        );
    }

    async fn close(self) {
        self.lifecycle.shutdown().await;
        self.session.shutdown().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn wait_returns_without_yielding_and_clamps_without_a_minimum() {
    let f = WaitFixture::new().await;
    let child = f.spawn().await;
    let before = tokio::time::Instant::now();
    let result = f.wait(&child, u64::MAX).await;
    assert_eq!(before, tokio::time::Instant::now());
    assert_eq!(result["state"], "running");
    assert_eq!(result["armed"], true);
    assert_eq!(result["wake_after_ms"], 300_000);
    assert_eq!(result["clamped"], true);
    assert!(result["deadline"].as_u64().is_some());
    assert_eq!(f.wait(&child, 1).await["wake_after_ms"], 1);
    f.close().await;
}

#[tokio::test(start_paused = true)]
async fn zero_is_a_plain_status_check() {
    let f = WaitFixture::new().await;
    let child = f.spawn().await;
    assert_eq!(f.wait(&child, 0).await["armed"], false);
    assert!(
        f.runtime
            .delegation()
            .unwrap()
            .wait_timers()
            .snapshot()
            .is_empty()
    );
    f.advance(300_001).await;
    assert!(f.wakes("delegation.wait-timeout").is_empty());
    let armed = f.wait(&child, 50).await;
    let status = f.wait(&child, 0).await;
    assert_eq!(
        status["armed"], true,
        "a status check reports the earlier timer"
    );
    assert_eq!(status["deadline"], armed["deadline"]);
    assert_eq!(status["wake_after_ms"], 50);
    assert_eq!(
        f.runtime
            .delegation()
            .unwrap()
            .wait_timers()
            .snapshot()
            .len(),
        1
    );
    f.advance(51).await;
    assert_eq!(f.wakes("delegation.wait-timeout").len(), 1);
    f.close().await;
}

#[tokio::test(start_paused = true)]
async fn expiry_wakes_once_with_status_and_no_child_output() {
    let f = WaitFixture::new().await;
    let child = f.spawn().await;
    f.wait(&child, 50).await;
    f.advance(51).await;
    let wakes = f.wakes("delegation.wait-timeout");
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    for expected in [
        &child,
        "dev",
        "running",
        "elapsed_ms",
        "wake_after_ms",
        "steps_this_task",
    ] {
        assert!(wakes[0].contains(expected), "{}", wakes[0]);
    }
    assert!(!wakes[0].contains("private child result"));
    assert!(
        f.runtime
            .delegation()
            .unwrap()
            .wait_timers()
            .snapshot()
            .is_empty()
    );
    assert!(
        f.session
            .with_history(|history| history.iter().all(|message| !message
                .joined_text()
                .contains("single-use agent wait deadline")))
    );
    f.advance(300_001).await;
    assert_eq!(f.wakes("delegation.wait-timeout").len(), 1);
    f.close().await;
}

#[tokio::test(start_paused = true)]
async fn completion_cancels_the_timer() {
    let f = WaitFixture::new().await;
    let child = f.spawn().await;
    f.wait(&child, 50).await;
    f.provider.child_release.add_permits(1);
    f.settle().await;
    f.advance(51).await;
    assert_eq!(f.wakes("delegation.child-completion").len(), 1);
    assert!(f.wakes("delegation.wait-timeout").is_empty());
    assert!(
        f.runtime
            .delegation()
            .unwrap()
            .wait_timers()
            .snapshot()
            .is_empty()
    );
    f.close().await;
}

#[tokio::test(start_paused = true)]
async fn a_running_follow_up_reports_no_earlier_result() {
    let f = WaitFixture::new().await;
    let child = f.spawn().await;
    f.provider.child_release.add_permits(1);
    f.settle().await;
    f.advance(1).await;
    let finished = f.wait(&child, 0).await;
    assert_eq!(finished["state"], "idle", "{finished}");
    assert_eq!(finished["result"], "private child result", "{finished}");
    assert!(finished.get("activity").is_none(), "{finished}");

    let sent = f
        .call(serde_json::json!({"action":"follow_up", "child_id":child, "task":"second task"}))
        .await;
    assert_eq!(sent["follow_up_sent"], child.as_str(), "{sent}");
    assert!(sent["note"].as_str().is_some(), "{sent}");
    let listed = f.call(serde_json::json!({"action":"list"})).await;
    assert_eq!(listed["children"][0]["state"], "running", "{listed}");
    assert!(listed["children"][0]["result"].is_null(), "{listed}");
    let waited = f.wait(&child, 0).await;
    assert_eq!(waited["state"], "running", "{waited}");
    assert!(waited.get("result").is_none(), "{waited}");
    f.close().await;
}

#[tokio::test(start_paused = true)]
async fn expiry_during_a_user_turn_waits_for_its_boundary() {
    let f = WaitFixture::new().await;
    let child = f.spawn().await;
    f.wait(&child, 50).await;
    let user = f.session.send(UserInput::text("busy parent")).unwrap();
    f.settle().await;
    f.advance(51).await;
    assert!(f.wakes("delegation.wait-timeout").is_empty());
    f.provider.user_release.add_permits(1);
    user.completed().await;
    f.settle().await;
    assert_eq!(f.wakes("delegation.wait-timeout").len(), 1);
    f.close().await;
}

#[tokio::test(start_paused = true)]
async fn pending_expiry_loses_to_completion_at_the_same_boundary() {
    let f = WaitFixture::new().await;
    let child = f.spawn().await;
    f.wait(&child, 50).await;
    let user = f.session.send(UserInput::text("busy parent")).unwrap();
    f.settle().await;
    f.advance(50).await;
    f.provider.child_release.add_permits(1);
    f.settle().await;
    f.provider.user_release.add_permits(1);
    user.completed().await;
    f.settle().await;
    assert_eq!(f.wakes("delegation.child-completion").len(), 1);
    assert!(f.wakes("delegation.wait-timeout").is_empty());
    f.close().await;
}

#[tokio::test(start_paused = true)]
async fn rearm_replaces_the_deadline_and_expiries_batch_in_child_order() {
    let f = WaitFixture::new().await;
    let first = f.spawn().await;
    let second = f.spawn().await;
    f.wait(&first, 50).await;
    f.advance(25).await;
    f.wait(&second, 100).await;
    f.wait(&first, 100).await;
    f.advance(26).await;
    assert!(f.wakes("delegation.wait-timeout").is_empty());
    f.advance(75).await;
    let wakes = f.wakes("delegation.wait-timeout");
    assert_eq!(wakes.len(), 1);
    assert!(
        wakes[0].find(&first) < wakes[0].find(&second),
        "{}",
        wakes[0]
    );
    f.close().await;
}

#[tokio::test(start_paused = true)]
async fn shutdown_cancels_armed_timers_without_waking() {
    let f = WaitFixture::new().await;
    let child = f.spawn().await;
    f.wait(&child, 50).await;
    f.lifecycle.shutdown().await;
    f.advance(51).await;
    assert!(f.wakes("delegation.wait-timeout").is_empty());
    assert!(
        f.runtime
            .delegation()
            .unwrap()
            .wait_timers()
            .snapshot()
            .is_empty()
    );
    assert_eq!(f.wait(&child, 50).await["armed"], false);
    f.close().await;
}

#[tokio::test(start_paused = true)]
async fn running_status_reports_committed_steps_and_draft_size_only() {
    let f = WaitFixture::with_tool_step().await;
    let child = f.spawn().await;
    f.advance(1_000).await;
    let status = f.streaming_status(&child).await;
    let activity = &status["activity"];
    assert_eq!(activity["steps_this_task"], 2, "{status}");
    assert_eq!(activity["last_step"]["kind"], "tool_result", "{status}");
    assert_eq!(activity["last_step"]["tool"], "list", "{status}");
    assert_eq!(activity["pending_tools"], serde_json::json!([]), "{status}");
    assert_eq!(activity["streaming"]["attempt"], 1, "{status}");
    assert!(activity["streaming"]["for_ms"].is_u64(), "{status}");
    assert!(
        activity["elapsed_ms"].as_u64().unwrap() >= 1_000,
        "{status}"
    );
    assert!(!status.to_string().contains(DRAFT), "{status}");

    let listed = f.call(serde_json::json!({"action":"list"})).await;
    assert_eq!(
        listed["children"][0]["activity"]["steps_this_task"], 2,
        "{listed}"
    );

    f.provider.draft_release.add_permits(1);
    f.settle().await;
    f.advance(1).await;
    let finished = f.wait(&child, 0).await;
    assert_eq!(finished["state"], "idle", "{finished}");
    assert!(finished.get("activity").is_none(), "{finished}");
    f.close().await;
}

#[tokio::test(start_paused = true)]
async fn expiry_wake_carries_activity_without_streamed_text() {
    let f = WaitFixture::with_tool_step().await;
    let child = f.spawn().await;
    f.streaming_status(&child).await;
    f.wait(&child, 50).await;
    f.advance(51).await;
    let wakes = f.wakes("delegation.wait-timeout");
    assert_eq!(wakes.len(), 1, "{wakes:?}");
    for expected in [
        "steps_this_task",
        "tool_result",
        "text_chars",
        "agent transcript shows the committed steps",
    ] {
        assert!(wakes[0].contains(expected), "{}", wakes[0]);
    }
    assert!(!wakes[0].contains(DRAFT), "{}", wakes[0]);
    f.provider.draft_release.add_permits(1);
    f.settle().await;
    f.close().await;
}
