use super::*;

use agent_runtime::delegation::DelegationCoordinator;
use agent_runtime_core::ids::ChildId;

const LONG_TASK_MARKER: &str = "write-a-long-report-for-the-parent";
const SHORT_TASK_MARKER: &str = "write-a-short-note-for-the-parent";
const LONG_REPLY_HEAD: &str = "LONG-REPLY-HEAD";
const SHORT_REPLY: &str = "the short child's note";

/// A child's final answer longer than one Runtime internal turn (4,096
/// characters), which is an ordinary size for a review or research report.
fn long_reply() -> String {
    format!(
        "{LONG_REPLY_HEAD}\n{}",
        "one finding the parent asked for, with its evidence\n".repeat(120)
    )
}

/// Answers each child by the marker in its task and every parent request with
/// a fixed acknowledgement, so the order in which children and deliveries
/// reach the provider does not matter.
#[derive(Debug)]
struct ReplyByTaskProvider {
    replies: Vec<(&'static str, String)>,
    requests: Mutex<Vec<ProviderRequest>>,
}

impl ReplyByTaskProvider {
    fn new(replies: Vec<(&'static str, String)>) -> Self {
        Self {
            replies,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn wires(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("provider requests poisoned")
            .iter()
            .map(|request| serde_json::to_string(&request.messages).expect("provider messages"))
            .collect()
    }
}

#[async_trait]
impl Provider for ReplyByTaskProvider {
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
        let text = self
            .replies
            .iter()
            .find(|(marker, _)| wire.contains(marker))
            .map(|(_, reply)| reply.clone())
            .unwrap_or_else(|| "the parent integrated its children's results".to_owned());
        Ok(Box::pin(futures_util::stream::iter(vec![
            ProviderStreamEvent::TextDelta { text },
            usage_event(5, 2),
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])))
    }
}

async fn spawn_read_only(coordinator: &DelegationCoordinator, task: &str) -> ChildId {
    let spawned = coordinator
        .spawn(ChildSpec {
            task: UserInput::text(task),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(1),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the child spawns");
    match spawned {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    }
}

/// The first automatic child-completion request that names `child`.
async fn wait_for_delivery_naming(provider: &ReplyByTaskProvider, child: &ChildId) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(wire) = provider.wires().into_iter().find(|wire| {
                wire.contains("delegation.child-completion") && wire.contains(child.as_str())
            }) {
                return wire;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "no child-completion delivery named child {child} within 10 s; \
             the provider saw {} requests",
            provider.wires().len()
        )
    })
}

/// A parent that never reads its child's result must still be handed a long
/// one. Today the batch exceeds Runtime's internal-turn limit, admission is
/// refused as a conflict, and nothing retries it.
#[tokio::test]
async fn a_child_reply_longer_than_an_internal_turn_still_reaches_the_parent() {
    let fixture = Fixture::new();
    let provider = Arc::new(ReplyByTaskProvider::new(vec![(
        LONG_TASK_MARKER,
        long_reply(),
    )]));
    let smith = Box::pin(factory::build_request(request(&fixture, provider.clone())))
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
    let coordinator = delegation.coordinator().expect("a coordinator");

    let child = spawn_read_only(coordinator, LONG_TASK_MARKER).await;
    let outcome = coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the child completes");
    assert!(
        matches!(&outcome, ChildTaskOutcome::Completed { result, .. } if result.text == long_reply()),
        "{outcome:?}"
    );

    let delivery = wait_for_delivery_naming(&provider, &child).await;
    assert!(delivery.contains(LONG_REPLY_HEAD), "{delivery}");

    session.shutdown().await.expect("a clean shutdown");
}

/// A reply larger than the resolved inline budget arrives as its opening
/// excerpt plus a parent-owned reference, and the parent can read the exact
/// reply back through that reference.
#[tokio::test]
async fn a_reply_over_the_inline_budget_is_delivered_as_an_excerpt_and_a_readable_copy() {
    let fixture = Fixture::new();
    let provider = Arc::new(ReplyByTaskProvider::new(vec![(
        LONG_TASK_MARKER,
        long_reply(),
    )]));
    let paths = smith_runtime::host::paths(&fixture.config(), fixture.project.path())
        .expect("protected Smith paths");
    let store = Arc::new(SmithArtifactStore::new(paths));
    let mut runtime_request = request(&fixture, provider.clone());
    runtime_request.artifact_store = Some(store.clone());
    runtime_request
        .config
        .context
        .tool_output_inline_bytes
        .value = 2_048;
    let smith = Box::pin(factory::build_request(runtime_request))
        .await
        .expect("a root runtime with an artifact store");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let delegation = smith.delegation().expect("a delegation surface");
    let _lifecycle = wire_delegation(&session, delegation)
        .await
        .expect("delegation wires once");
    let coordinator = delegation.coordinator().expect("a coordinator");

    let child = spawn_read_only(coordinator, LONG_TASK_MARKER).await;
    coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the child completes");
    let delivery = wait_for_delivery_naming(&provider, &child).await;
    assert!(delivery.contains(LONG_REPLY_HEAD), "{delivery}");
    assert!(
        !delivery.contains(&long_reply()),
        "the whole reply must not be inlined over the budget: {delivery}"
    );
    let marker = "is stored as artifact ";
    let start = delivery
        .find(marker)
        .expect("the delivery names the stored reply")
        + marker.len();
    let id = delivery[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>();

    let page = store
        .read(ArtifactRead {
            session: session.id().clone(),
            id: agent_runtime_core::artifact::ArtifactId::new(id.clone()).expect("an artifact id"),
            offset: 0,
            limit: MAX_ARTIFACT_READ_BYTES,
        })
        .await
        .expect("the parent reads its own copy of the reply");
    assert_eq!(String::from_utf8(page.bytes).unwrap(), long_reply());
    assert_eq!(page.next_offset, None);

    // agent result cites the same stored copy, so a model that lost the
    // delivery can recover the id it named.
    let slot = Arc::new(OnceLock::new());
    slot.set(coordinator.clone()).unwrap();
    let ctx = InvocationContext {
        session: session.id().clone(),
        turn: None,
        call_id: ToolCallId::new("result-after-delivery"),
        request: RequestId::new("result-after-delivery"),
        workspace: Arc::new(LexicalWorkspace::new("/repo")),
        clock: Arc::new(SystemClock),
        cancel: Cancellation::new(),
        deadline: Deadline::never(),
        output_limit: 1_000_000,
    };
    let result = invoke_agent(
        &AgentTool::new(slot.clone()),
        serde_json::json!({"action": "result", "child_id": child.as_str()}),
        &ctx,
    )
    .await
    .expect("a result outcome")
    .value;
    assert_eq!(result["result"]["reply_artifact"], id.as_str(), "{result}");

    // Runtime names the child on its stored reply, so listing by child finds
    // it without the delivery.
    let tools = smith_runtime::tool_output::ToolOutputContextPolicy {
        inline_bytes: 8192,
        artifact_page_bytes: 2048,
    }
    .artifact_tools(
        store.clone(),
        Some(store.clone() as Arc<dyn smith_runtime::artifact::ArtifactCatalog>),
        Some(slot as Arc<dyn smith_runtime::tool_output::ChildDirectory>),
    );
    let lister = tools
        .iter()
        .find(|tool| tool.spec().name == smith_runtime::tool_output::ARTIFACT_LIST_TOOL_NAME)
        .expect("a lister beside the reader");
    let listed = invoke_agent(
        lister.as_ref(),
        serde_json::json!({"child_id": child.as_str(), "purpose": "delegation.child-reply"}),
        &ctx,
    )
    .await
    .expect("a listing")
    .value;
    assert_eq!(listed["total"], 1, "{listed}");
    assert_eq!(listed["artifacts"][0]["id"], id.as_str(), "{listed}");
    assert_eq!(
        listed["artifacts"][0]["child_id"],
        child.as_str(),
        "{listed}"
    );

    session.shutdown().await.expect("a clean shutdown");
}

/// Every ready outcome is admitted as one batch, so a refused long reply must
/// not hold back a later child whose short reply would fit on its own.
#[tokio::test]
async fn a_long_child_reply_does_not_block_a_later_childs_delivery() {
    let fixture = Fixture::new();
    let provider = Arc::new(ReplyByTaskProvider::new(vec![
        (LONG_TASK_MARKER, long_reply()),
        (SHORT_TASK_MARKER, SHORT_REPLY.to_owned()),
    ]));
    let smith = Box::pin(factory::build_request(request(&fixture, provider.clone())))
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
    let coordinator = delegation.coordinator().expect("a coordinator");

    let long = spawn_read_only(coordinator, LONG_TASK_MARKER).await;
    coordinator
        .wait_task_outcome(&long)
        .await
        .expect("the long child completes");
    let short = spawn_read_only(coordinator, SHORT_TASK_MARKER).await;
    coordinator
        .wait_task_outcome(&short)
        .await
        .expect("the short child completes");

    let delivery = wait_for_delivery_naming(&provider, &short).await;
    assert!(delivery.contains(SHORT_REPLY), "{delivery}");

    session.shutdown().await.expect("a clean shutdown");
}
