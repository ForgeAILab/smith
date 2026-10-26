use super::*;

use smith_module::{CompiledModule, ModuleComposition, ModuleOrigin};

/// The tools a read-only child asks for, the reviewed reads it should get
/// and the writes it must not.
const ACTIVATE: [&str; 6] = [
    "tool:web_fetch",
    "tool:web_search",
    "tool:artifact.read",
    "tool:artifact.list",
    "tool:edit",
    "tool:shell",
];

fn text_reply(text: &str) -> ScriptedStream {
    ScriptedStream::new(vec![
        ProviderStreamEvent::TextDelta { text: text.into() },
        usage_event(5, 2),
        ProviderStreamEvent::Finish {
            reason: FinishReason::Stop,
        },
    ])
}

/// The child activates every tool in [`ACTIVATE`], then answers.
fn activating_child() -> Arc<FakeProvider> {
    let arguments = serde_json::json!({ "ids": ACTIVATE }).to_string();
    let mut activate = tool_call_fragments(0, "activate", "registry.activate", &arguments);
    activate.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(activate),
            text_reply("the child's findings"),
            text_reply("the parent read them"),
            text_reply("the parent read them"),
        ],
    ))
}

/// Spawns one child of `spec`'s shape on a root with the web module and an
/// artifact store, and returns the names of the tools the child held after
/// activating [`ACTIVATE`].
async fn child_tools(
    network_modules: Option<Vec<String>>,
    tools: ToolViewScope,
    workspace: WorkspacePolicy,
) -> std::collections::BTreeSet<String> {
    let fixture = Fixture::new();
    let provider = activating_child();
    let paths = smith_runtime::host::paths(&fixture.config(), fixture.project.path())
        .expect("protected Smith paths");
    let store = Arc::new(SmithArtifactStore::new(paths));
    let mut runtime_request = request(&fixture, provider.clone());
    runtime_request.artifact_store = Some(store.clone());
    runtime_request.artifact_catalog = Some(store);
    runtime_request.modules = ModuleComposition::with_defaults(
        vec![CompiledModule {
            module: Arc::new(smith_module_web::WebModule),
            origin: ModuleOrigin::FirstParty,
        }],
        Vec::new(),
    );
    if let Some(modules) = network_modules {
        runtime_request.config.agent.read_only_network_modules.value = modules;
    }
    let smith = Box::pin(factory::build_request(runtime_request))
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
    let child = match coordinator
        .spawn(ChildSpec {
            task: UserInput::text("research a release"),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(1),
            tools,
            workspace,
        })
        .await
        .expect("the child spawns")
    {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };
    coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the child completes");
    let answered = &provider.requests()[1];
    let names = answered
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    session.shutdown().await.expect("a clean shutdown");
    names
}

/// A read-only child keeps what a read-only root keeps: the web tools the
/// default `read_only_network_modules` admits and its own artifact tools,
/// whether it is read-only by tool scope or by workspace, and never a write.
#[tokio::test]
async fn a_read_only_child_holds_every_tool_that_cannot_write() {
    for (tools, workspace) in [
        (ToolViewScope::ReadOnly, WorkspacePolicy::SharedProject),
        (ToolViewScope::ReadOnly, WorkspacePolicy::ReadOnlyView),
        // A build profile spawned with `tools: "all"` and no workspace.
        (ToolViewScope::All, WorkspacePolicy::ReadOnlyView),
    ] {
        let names = child_tools(None, tools.clone(), workspace.clone()).await;
        for kept in ["web_fetch", "web_search", "artifact.read", "artifact.list"] {
            assert!(
                names.contains(kept),
                "{tools:?}/{workspace:?} child lacks {kept}: {names:?}"
            );
        }
        for withheld in ["edit", "shell"] {
            assert!(
                !names.contains(withheld),
                "{tools:?}/{workspace:?} child holds {withheld}: {names:?}"
            );
        }
    }
}

/// `read_only_network_modules = []` narrows children exactly as it narrows a
/// read-only root; the artifact tools stay.
#[tokio::test]
async fn the_owner_can_still_keep_network_tools_from_read_only_children() {
    let names = child_tools(
        Some(Vec::new()),
        ToolViewScope::ReadOnly,
        WorkspacePolicy::ReadOnlyView,
    )
    .await;
    assert!(!names.contains("web_fetch"), "{names:?}");
    assert!(!names.contains("web_search"), "{names:?}");
    assert!(names.contains("artifact.read"), "{names:?}");
}
