use std::sync::atomic::{AtomicUsize, Ordering};

use agent_runtime::harness::{
    ComponentDescriptor, ComponentPhase, ContextContributor, ContextPatch, ContextView,
    HistoryProjection, HistoryProjector, HistoryView, ModelInterceptor, ModelRequestPatch,
    ModelView, ToolOutputPatch, ToolOutputProcessor, ToolOutputView, ToolViewContext,
    ToolViewPatch, ToolViewResolver, TurnCommitHook, TurnCommitPatch, TurnCommitView,
};
use agent_runtime::runtime::StartSession;
use agent_runtime_core::content::UserInput;
use agent_runtime_core::event::EventEnvelope;
use agent_runtime_core::observer::EventObserver;
use agent_runtime_core::tool::ToolOutcome;
use smith_module::{
    CompiledModule, Module, ModuleComposition, ModuleContext, ModuleContribution, ModuleError,
    ModuleOrigin, ModuleState, Mounted, PipelineComponent, SlashCommand, StatusItem,
    StatusSeverity, StatusSource,
};

use super::*;

#[derive(Debug, Default)]
struct Hooks {
    phases: [AtomicUsize; 6],
    events: AtomicUsize,
}

impl Hooks {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new("fixture.module", RegistryRevision::new("1"))
    }
}

#[async_trait::async_trait]
impl HistoryProjector for Hooks {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor()
    }
    async fn project(&self, _: &HistoryView) -> Result<HistoryProjection, RuntimeError> {
        self.phases[0].fetch_add(1, Ordering::Relaxed);
        Ok(HistoryProjection::default())
    }
}

#[async_trait::async_trait]
impl ContextContributor for Hooks {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor()
    }
    async fn contribute(&self, _: &ContextView) -> Result<ContextPatch, RuntimeError> {
        self.phases[1].fetch_add(1, Ordering::Relaxed);
        Ok(ContextPatch::default())
    }
}

#[async_trait::async_trait]
impl ToolViewResolver for Hooks {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor()
    }
    async fn resolve(&self, _: &ToolViewContext) -> Result<ToolViewPatch, RuntimeError> {
        self.phases[2].fetch_add(1, Ordering::Relaxed);
        Ok(ToolViewPatch::default())
    }
}

#[async_trait::async_trait]
impl ModelInterceptor for Hooks {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor()
    }
    async fn before_model(&self, _: &ModelView) -> Result<ModelRequestPatch, RuntimeError> {
        self.phases[3].fetch_add(1, Ordering::Relaxed);
        Ok(ModelRequestPatch::default())
    }
}

#[async_trait::async_trait]
impl ToolOutputProcessor for Hooks {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor()
    }
    async fn process(
        &self,
        _: &ToolOutputView,
        outcome: ToolOutcome,
    ) -> Result<ToolOutputPatch, RuntimeError> {
        self.phases[4].fetch_add(1, Ordering::Relaxed);
        Ok(ToolOutputPatch {
            outcome,
            state: None,
            events: vec![],
        })
    }
}

#[async_trait::async_trait]
impl TurnCommitHook for Hooks {
    fn descriptor(&self) -> ComponentDescriptor {
        self.descriptor()
    }
    async fn after_commit(&self, _: &TurnCommitView) -> Result<TurnCommitPatch, RuntimeError> {
        self.phases[5].fetch_add(1, Ordering::Relaxed);
        Ok(TurnCommitPatch::default())
    }
}

impl StatusSource for Hooks {
    fn current(&self) -> Option<StatusItem> {
        Some(StatusItem::new(
            "pressure",
            "context pressure",
            Some(StatusSeverity::Warning),
        ))
    }
}

impl EventObserver for Hooks {
    fn observe(&self, _: &EventEnvelope) {
        self.events.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct FixtureModule {
    id: &'static str,
    hooks: Arc<Hooks>,
    fail: bool,
}

impl Module for FixtureModule {
    fn id(&self) -> &str {
        self.id
    }
    fn revision(&self) -> &str {
        "fixture-v1"
    }
    fn description(&self) -> &str {
        "module fixture"
    }
    fn default_enabled(&self) -> bool {
        true
    }
    fn mount(&self, context: &ModuleContext) -> Result<Mounted, ModuleError> {
        assert_eq!(context.max_input_tokens, 24_000);
        if self.fail {
            return Err(ModuleError("fixture mount failure".into()));
        }
        let hooks = &self.hooks;
        Ok(Mounted::Contributions(vec![
            ModuleContribution::Tool(Arc::new(smith_tools::ReadTool::default())),
            ModuleContribution::Pipeline(PipelineComponent::HistoryProjector(hooks.clone())),
            ModuleContribution::Pipeline(PipelineComponent::ContextContributor(hooks.clone())),
            ModuleContribution::Pipeline(PipelineComponent::ToolViewResolver(hooks.clone())),
            ModuleContribution::Pipeline(PipelineComponent::ModelInterceptor(hooks.clone())),
            ModuleContribution::Pipeline(PipelineComponent::ToolOutputProcessor(hooks.clone())),
            ModuleContribution::Pipeline(PipelineComponent::TurnCommitHook(hooks.clone())),
            ModuleContribution::Observer {
                name: "fixture.events".into(),
                observer: hooks.clone(),
            },
            ModuleContribution::Command(SlashCommand {
                name: "fixture".into(),
                description: "fixture command".into(),
            }),
            ModuleContribution::StatusItem {
                name: "pressure".into(),
                source: hooks.clone(),
            },
        ]))
    }
}

fn request(workspace: &std::path::Path, compiled: Vec<CompiledModule>) -> RuntimeRequest {
    let mut config = resolved_config();
    config.provider = provider(KIND_FAKE, None);
    config.model_limits.context_tokens = Some(sourced(32_768));
    config.model_limits.max_input_tokens = Some(sourced(24_000));
    config.model_limits.max_output_tokens = Some(sourced(4_096));
    config.approval.mode = sourced(ApprovalMode::AllowAll);
    config.persistence.enabled = sourced(false);
    config.agent.profile.delegation = sourced(false);
    let mut request = RuntimeRequest::new(config, HostSurface::Headless);
    request.workspace = Some(Arc::new(ProjectWorkspace::new(workspace).unwrap()));
    request.built_in_tools = false;
    request.modules = ModuleComposition::with_defaults(compiled, vec![]);
    request
}

fn compiled(hooks: Arc<Hooks>, origin: ModuleOrigin, fail: bool) -> CompiledModule {
    CompiledModule {
        module: Arc::new(FixtureModule {
            id: "fixture",
            hooks,
            fail,
        }),
        origin,
    }
}

#[tokio::test]
async fn mounted_values_reach_registry_pipeline_observer_and_evidence() {
    for origin in [
        ModuleOrigin::FirstParty,
        ModuleOrigin::ThirdParty {
            crate_name: "fixture-crate".into(),
        },
    ] {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("example.txt"), "fixture body").unwrap();
        let hooks = Arc::new(Hooks::default());
        let mut request = request(
            workspace.path(),
            vec![compiled(hooks.clone(), origin.clone(), false)],
        );
        let mut call = agent_runtime::provider::fake::tool_call_fragments(
            0,
            "read-1",
            "read",
            r#"{"path":"example.txt"}"#,
        );
        call.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        request.provider = Some(Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![
                ScriptedStream::new(call),
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "done".into(),
                    },
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
            ],
        )));
        let harness =
            crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap();
        let runtime = build(harness).await.unwrap();
        assert!(runtime.policy().tools.iter().any(|name| name == "read"));
        assert!(runtime.abilities().descriptors().iter().any(
            |descriptor| descriptor.id() == &agent_runtime::registry::RegistryId::tool("read")
        ));
        let record = &runtime.harness_modules()[0];
        assert_eq!(record.trust, crate::harness::ModuleTrust::TrustedNative);
        match origin {
            ModuleOrigin::Sandboxed { .. } => unreachable!("this test supplies native origins"),
            ModuleOrigin::FirstParty => {
                assert_eq!(record.id.as_str(), "smith/fixture");
                assert_eq!(record.provenance, crate::harness::ModuleProvenance::BuiltIn);
            }
            ModuleOrigin::ThirdParty { crate_name } => {
                assert_eq!(
                    record.provenance,
                    crate::harness::ModuleProvenance::CompiledThirdParty(crate_name)
                );
            }
        }
        assert_eq!(record.contributions.len(), 10);
        assert_eq!(
            record.granted_capabilities,
            crate::harness::CapabilitySet::from([crate::harness::Capability::WorkspaceRead])
        );
        assert!(
            record
                .contributions
                .contains(&crate::harness::Contribution::Pipeline {
                    phase: ComponentPhase::TurnCommit,
                    component: "fixture.module".into()
                })
        );
        assert!(
            record
                .contributions
                .contains(&crate::harness::Contribution::StatusItem {
                    name: "pressure".into()
                })
        );
        assert_eq!(runtime.module_report()[0].state, ModuleState::Mounted);
        assert_eq!(runtime.module_status()[0].name(), "pressure");
        let session = runtime
            .runtime()
            .start_session(StartSession::new())
            .await
            .unwrap();
        let turn = session.send(UserInput::text("read example.txt")).unwrap();
        turn.completed().await;
        for (index, count) in hooks.phases.iter().enumerate() {
            assert!(count.load(Ordering::Relaxed) > 0, "phase {index} never ran");
        }
        assert!(hooks.events.load(Ordering::Relaxed) > 0);
    }
}

#[tokio::test]
async fn failed_and_off_modules_leave_no_record_or_contributions() {
    for failed in [true, false] {
        let workspace = tempfile::tempdir().unwrap();
        let hooks = Arc::new(Hooks::default());
        let mut request = request(
            workspace.path(),
            vec![compiled(hooks.clone(), ModuleOrigin::FirstParty, failed)],
        );
        if !failed {
            request.modules.enabled.clear();
        }
        let runtime =
            build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap())
                .await
                .unwrap();
        assert!(runtime.harness_modules().is_empty());
        assert!(runtime.mounted_modules().is_empty());
        assert!(runtime.module_status().is_empty());
        assert!(!runtime.policy().tools.iter().any(|name| name == "read"));
        if failed {
            assert!(matches!(
                runtime.module_report()[0].state,
                ModuleState::Failed { .. }
            ));
        } else {
            assert_eq!(runtime.module_report()[0].state, ModuleState::Off);
        }
        assert_eq!(hooks.events.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn explicit_empty_list_keeps_factory_policy_and_evidence_identical() {
    let workspace = tempfile::tempdir().unwrap();
    let baseline = build(
        crate::harness::resolve(crate::harness::HarnessSpec::trusted(request(
            workspace.path(),
            vec![],
        )))
        .unwrap(),
    )
    .await
    .unwrap();
    let mut explicit = request(workspace.path(), vec![]);
    explicit.modules = ModuleComposition::with_defaults(vec![], vec![]);
    let explicit =
        build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(explicit)).unwrap())
            .await
            .unwrap();
    assert_eq!(baseline.policy(), explicit.policy());
    assert_eq!(baseline.harness_identity(), explicit.harness_identity());
    assert_eq!(baseline.harness_modules(), explicit.harness_modules());
    assert_eq!(baseline.harness_report(), explicit.harness_report());
    assert!(explicit.module_report().is_empty());
}

fn first_party_request(workspace: &std::path::Path) -> RuntimeRequest {
    let compiled = vec![
        CompiledModule {
            module: Arc::new(smith_module_image_generation::ImageGenerationModule),
            origin: ModuleOrigin::FirstParty,
        },
        CompiledModule {
            module: Arc::new(smith_module_budget_notice::BudgetNoticeModule),
            origin: ModuleOrigin::FirstParty,
        },
    ];
    let mut request = request(workspace, compiled);
    request.built_in_tools = true;
    request.config.user_dir = workspace.to_path_buf();
    request.config.provider = provider(KIND_OPENAI_COMPATIBLE, Some("https://api.openai.com/v1"));
    request.config.provider.api_key = Some(sourced(Secret::new("module-fixture")));
    request.config.image_generation.enabled = sourced(true);
    let project = crate::session::ProjectId::new("module-fixture").unwrap();
    request.artifact_store = Some(Arc::new(crate::artifact::SmithArtifactStore::new(
        crate::session::SessionPaths::new(workspace, &project),
    )));
    request.semantic_summary = Some(crate::summary::SmithSemanticSummaryConfig::standard());
    request
}

#[tokio::test]
async fn first_party_ports_record_only_their_mounted_contributions() {
    use crate::harness::{Contribution, ModuleProvenance};

    for off in [None, Some("image-generation"), Some("budget-notice")] {
        let workspace = tempfile::tempdir().unwrap();
        let mut request = first_party_request(workspace.path());
        if let Some(id) = off {
            request.modules.enabled.remove(id);
        }
        let runtime =
            build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap())
                .await
                .unwrap();
        let image = runtime
            .harness_modules()
            .iter()
            .find(|record| record.id.as_str() == "smith/image-generation");
        let budget = runtime
            .harness_modules()
            .iter()
            .find(|record| record.id.as_str() == "smith/budget-notice");
        assert_eq!(image.is_some(), off != Some("image-generation"));
        assert_eq!(budget.is_some(), off != Some("budget-notice"));
        assert_eq!(
            runtime
                .policy()
                .tools
                .iter()
                .any(|name| name == "generate_image"),
            image.is_some()
        );
        if let Some(image) = image {
            assert_eq!(image.provenance, ModuleProvenance::BuiltIn);
            assert!(
                matches!(image.contributions.as_slice(), [Contribution::Tool { name, .. }] if name == "generate_image")
            );
        }
        if let Some(budget) = budget {
            assert_eq!(budget.provenance, ModuleProvenance::BuiltIn);
            assert_eq!(
                budget.contributions,
                vec![
                    Contribution::Pipeline {
                        phase: ComponentPhase::Context,
                        component: "smith.budget_notice".into()
                    },
                    Contribution::Pipeline {
                        phase: ComponentPhase::TurnCommit,
                        component: "smith.budget_notice".into()
                    },
                    Contribution::StatusItem {
                        name: "budget-notice".into()
                    },
                ]
            );
        }
        let components = runtime
            .mounted_modules()
            .iter()
            .flat_map(|module| &module.contributions)
            .filter_map(|contribution| {
                if let ModuleContribution::Pipeline(component) = contribution {
                    Some(component.descriptor())
                } else {
                    None
                }
            })
            .filter(|descriptor| descriptor.id().as_str() == "smith.budget_notice")
            .count();
        assert_eq!(components, if budget.is_some() { 2 } else { 0 });
        assert!(runtime.module_status().is_empty());
    }
}

#[derive(Debug)]
struct StatusModule {
    id: &'static str,
    active: Arc<std::sync::atomic::AtomicBool>,
}

impl Module for StatusModule {
    fn id(&self) -> &str {
        self.id
    }
    fn revision(&self) -> &str {
        "fixture-status-v1"
    }
    fn description(&self) -> &str {
        "status ordering fixture"
    }
    fn default_enabled(&self) -> bool {
        true
    }
    fn mount(&self, _: &ModuleContext) -> Result<Mounted, ModuleError> {
        Ok(Mounted::Contributions(
            ["z", "a"]
                .into_iter()
                .map(|suffix| {
                    let name = format!("{}-{suffix}", self.id);
                    ModuleContribution::StatusItem {
                        name: name.clone(),
                        source: Arc::new(LiveStatus {
                            name,
                            active: self.active.clone(),
                        }),
                    }
                })
                .collect(),
        ))
    }
}

#[derive(Debug)]
struct LiveStatus {
    name: String,
    active: Arc<std::sync::atomic::AtomicBool>,
}

impl StatusSource for LiveStatus {
    fn current(&self) -> Option<StatusItem> {
        self.active
            .load(Ordering::Relaxed)
            .then(|| StatusItem::new(&self.name, "fixture", None))
    }
}

#[tokio::test]
async fn module_status_reads_live_sources_in_mount_then_name_order() {
    let workspace = tempfile::tempdir().unwrap();
    let active = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let compiled = ["z", "a"]
        .into_iter()
        .map(|id| CompiledModule {
            module: Arc::new(StatusModule {
                id,
                active: active.clone(),
            }),
            origin: ModuleOrigin::FirstParty,
        })
        .collect();
    let runtime = build(
        crate::harness::resolve(crate::harness::HarnessSpec::trusted(request(
            workspace.path(),
            compiled,
        )))
        .unwrap(),
    )
    .await
    .unwrap();
    let items = runtime.module_status();
    assert_eq!(
        items.iter().map(StatusItem::name).collect::<Vec<_>>(),
        ["a-a", "a-z", "z-a", "z-z"]
    );
    active.store(false, Ordering::Relaxed);
    assert!(runtime.module_status().is_empty());
}

#[tokio::test]
async fn host_history_service_visits_registered_canonical_inputs_until_shutdown() {
    use agent_runtime_core::content::{ContentPart, Message};
    use smith_module::SessionHistory;

    let workspace = tempfile::tempdir().unwrap();
    let runtime = build(
        crate::harness::resolve(crate::harness::HarnessSpec::trusted(request(
            workspace.path(),
            vec![],
        )))
        .unwrap(),
    )
    .await
    .unwrap();
    let image = "data:image/png;base64,fixture";
    let session = runtime
        .runtime()
        .start_session(
            StartSession::new().with_history(vec![Message::assistant(vec![ContentPart::Image {
                url: image.into(),
                detail: None,
            }])]),
        )
        .await
        .unwrap();
    let registration = runtime.session_history().register(session.clone());
    let mut seen = Vec::new();
    runtime
        .session_history()
        .with_history(session.id(), &mut |history| {
            for message in history {
                for part in &message.content {
                    if let ContentPart::Image { url, .. } = part {
                        seen.push(url.clone());
                    }
                }
            }
        })
        .unwrap();
    assert_eq!(seen, [image]);
    registration.unregister();
    registration.unregister();
    assert!(
        runtime
            .session_history()
            .with_history(session.id(), &mut |_| {})
            .is_err()
    );
}

#[tokio::test]
async fn rebuilding_off_then_on_restores_every_contribution() {
    let workspace = tempfile::tempdir().unwrap();
    let hooks = Arc::new(Hooks::default());
    let make_request = || {
        request(
            workspace.path(),
            vec![compiled(hooks.clone(), ModuleOrigin::FirstParty, false)],
        )
    };
    let baseline_request = make_request();
    let mut off_request = make_request();
    off_request.modules.enabled.clear();
    let on_request = make_request();
    let baseline = build(
        crate::harness::resolve(crate::harness::HarnessSpec::trusted(baseline_request)).unwrap(),
    )
    .await
    .unwrap();
    let off =
        build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(off_request)).unwrap())
            .await
            .unwrap();
    let on =
        build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(on_request)).unwrap())
            .await
            .unwrap();
    assert!(!off.policy().tools.contains(&"read".to_owned()));
    let core = build(
        crate::harness::resolve(crate::harness::HarnessSpec::trusted(request(
            workspace.path(),
            vec![],
        )))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(off.policy().tools, core.policy().tools);
    assert!(off.mounted_modules().is_empty());
    assert_eq!(off.harness_modules(), core.harness_modules());
    assert!(off.module_status().is_empty());
    assert_eq!(off.module_report()[0].state, ModuleState::Off);
    assert_eq!(on.policy().tools, baseline.policy().tools);
    assert_eq!(on.harness_identity(), baseline.harness_identity());
    assert_eq!(on.harness_modules(), baseline.harness_modules());
    assert_eq!(on.module_status(), baseline.module_status());

    let pipeline = |runtime: &SmithRuntime| {
        let mut builder = agent_runtime::harness::HarnessPipelineBuilder::new();
        for contribution in runtime
            .mounted_modules()
            .iter()
            .flat_map(|module| &module.contributions)
        {
            if let ModuleContribution::Pipeline(component) = contribution {
                match component {
                    PipelineComponent::HistoryProjector(component) => {
                        builder.history_projector(component.clone());
                    }
                    PipelineComponent::ContextContributor(component) => {
                        builder.context_contributor(component.clone());
                    }
                    PipelineComponent::ToolViewResolver(component) => {
                        builder.tool_view_resolver(component.clone());
                    }
                    PipelineComponent::ModelInterceptor(component) => {
                        builder.model_interceptor(component.clone());
                    }
                    PipelineComponent::ToolOutputProcessor(component) => {
                        builder.tool_output_processor(component.clone());
                    }
                    PipelineComponent::TurnCommitHook(component) => {
                        builder.turn_commit_hook(component.clone());
                    }
                }
            }
        }
        builder.seal().unwrap().fingerprint().clone()
    };
    assert_ne!(pipeline(&off), pipeline(&baseline));
    assert_eq!(pipeline(&on), pipeline(&baseline));
}

#[tokio::test]
async fn inheriting_children_mount_fresh_pipeline_state() {
    use agent_runtime::delegation::ChildRuntimeFactory;
    use agent_runtime_core::delegation::{
        ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
    };

    let workspace = tempfile::tempdir().unwrap();
    let mut request = first_party_request(workspace.path());
    request.config.agent.profile.delegation.value = true;
    request.provider = Some(Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        [13_000, 100]
            .into_iter()
            .map(|input| {
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "done".into(),
                    },
                    agent_runtime::provider::fake::usage_event(input, 20),
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ])
            })
            .collect(),
    )));
    let runtime =
        build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap())
            .await
            .unwrap();
    let root = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    root.send(UserInput::text("pressure"))
        .unwrap()
        .completed()
        .await;
    assert_eq!(runtime.module_status().len(), 1);
    let spec = ChildSpec {
        task: UserInput::text("child task"),
        model: ChildModelSelection::Inherit,
        limits: ChildLimits {
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        },
        tools: ToolViewScope::All,
        workspace: WorkspacePolicy::SharedProject,
    };
    let child_runtime = runtime
        .delegation()
        .unwrap()
        .factory
        .child_builder(&spec)
        .unwrap()
        .build()
        .unwrap();
    let child = child_runtime
        .start_session(StartSession::new())
        .await
        .unwrap();
    child.send(spec.task).unwrap().completed().await;
    assert_eq!(
        child.snapshot().extension_state["smith.budget_notice"].value["remaining_tokens"],
        23_900
    );
    assert_eq!(
        runtime.module_status().len(),
        1,
        "a child's commit must not clear the root notice"
    );
    child.shutdown().await.unwrap();
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn wasm_and_native_modules_share_registry_and_record_distinct_tiers() {
    let workspace = tempfile::tempdir().unwrap();
    let module = smith_wasm_host::WasmModule::load(
        smith_wasm_host::WasmEngine::new().unwrap(),
        include_bytes!("../../../../smith-wasm-host/tests/fixtures/tools.wasm"),
        smith_wasm_host::WasmModuleConfig {
            plugin_name: "fixture-plugin".into(),
            module_id: "test/module".into(),
            requested_capabilities: vec![smith_wasm_host::CapabilityGrant {
                capability: smith_module::ModuleCapability::WorkspaceWrite,
                roots: vec!["workspace".into()],
                hosts: vec![],
                allow_http: false,
            }],
            granted_capabilities: vec![smith_wasm_host::CapabilityGrant {
                capability: smith_module::ModuleCapability::WorkspaceWrite,
                roots: vec!["workspace".into()],
                hosts: vec![],
                allow_http: false,
            }],
            host_state_dir: workspace.path().join("host"),
            data_dir: workspace.path().join("data"),
            cache_dir: workspace
                .path()
                .join("host")
                .join(smith_wasm_host::COMPONENT_CACHE_DIRECTORY),
            limits: smith_wasm_host::WasmLimits::default(),
            session_health: Arc::new(smith_module::ModuleHealth::default()),
            reserved_tool_names: BTreeSet::new(),
        },
    )
    .unwrap();
    let wasm = CompiledModule {
        origin: module.origin(),
        module,
    };
    let mut request = request(
        workspace.path(),
        vec![
            compiled(Arc::new(Hooks::default()), ModuleOrigin::FirstParty, false),
            wasm,
        ],
    );
    request.config.user_dir = workspace.path().join("host");
    request.config.approval.mode = sourced(ApprovalMode::Ask);
    let approvals = Arc::new(WasmApproval(
        AtomicUsize::new(0),
        workspace.path().join("fixture.txt"),
    ));
    request.approval = Some(approvals.clone());
    let mut call = agent_runtime::provider::fake::tool_call_fragments(
        0,
        "wasm-call",
        "wasm_14_fixture-plugin_guest__probe",
        r#"{"action":"fs-write"}"#,
    );
    call.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    request.provider = Some(Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(call),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "done".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    )));
    let harness = crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap();
    let runtime = build(harness).await.unwrap();
    assert!(runtime.policy().tools.iter().any(|name| name == "read"));
    assert!(
        runtime
            .policy()
            .tools
            .iter()
            .any(|name| name.ends_with("__probe"))
    );
    assert!(
        runtime
            .harness_modules()
            .iter()
            .any(|record| record.trust == crate::harness::ModuleTrust::TrustedNative)
    );
    let sandbox = runtime
        .harness_modules()
        .iter()
        .find(|record| record.trust == crate::harness::ModuleTrust::Sandboxed)
        .unwrap();
    assert_eq!(
        sandbox.provenance,
        crate::harness::ModuleProvenance::UserManifest("fixture-plugin".into())
    );
    assert_eq!(
        sandbox.granted_capabilities,
        crate::harness::CapabilitySet::from([crate::harness::Capability::WorkspaceWrite])
    );
    let session = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    session
        .send(UserInput::text(
            "call wasm_14_fixture-plugin_guest__probe with fs-write",
        ))
        .unwrap()
        .completed()
        .await;
    assert_eq!(
        approvals.0.load(Ordering::Relaxed),
        1,
        "read-only guest claims must not bypass approval"
    );
    assert_eq!(
        std::fs::read(workspace.path().join("fixture.txt")).unwrap(),
        b"written"
    );
}

#[derive(Debug, Default)]
struct WasmApproval(AtomicUsize, std::path::PathBuf);
#[async_trait::async_trait]
impl agent_runtime_core::approval::ApprovalPolicy for WasmApproval {
    async fn decide(
        &self,
        _: &agent_runtime_core::approval::ApprovalRequest,
    ) -> agent_runtime_core::approval::ApprovalDecision {
        assert!(!self.1.exists(), "guest must not write before approval");
        self.0.fetch_add(1, Ordering::Relaxed);
        agent_runtime_core::approval::ApprovalDecision::Allow
    }
}

#[tokio::test]
async fn wasm_faults_leave_runtime_sessions_running() {
    for action in ["trap", "loop", "memory", "oversized", "malformed"] {
        let workspace = tempfile::tempdir().unwrap();
        let module = smith_wasm_host::WasmModule::load(
            smith_wasm_host::WasmEngine::new().unwrap(),
            include_bytes!("../../../../smith-wasm-host/tests/fixtures/tools.wasm"),
            smith_wasm_host::WasmModuleConfig {
                plugin_name: "faults".into(),
                module_id: "test/module".into(),
                requested_capabilities: vec![],
                granted_capabilities: vec![],
                host_state_dir: workspace.path().join("host"),
                data_dir: workspace.path().join("data"),
                cache_dir: workspace
                    .path()
                    .join("host")
                    .join(smith_wasm_host::COMPONENT_CACHE_DIRECTORY),
                limits: smith_wasm_host::WasmLimits {
                    tool_deadline: Duration::from_millis(if action == "memory" {
                        1000
                    } else {
                        30
                    }),
                    ..Default::default()
                },
                session_health: Arc::new(smith_module::ModuleHealth::default()),
                reserved_tool_names: BTreeSet::new(),
            },
        )
        .unwrap();
        let mut request = request(
            workspace.path(),
            vec![CompiledModule {
                origin: module.origin(),
                module,
            }],
        );
        request.config.user_dir = workspace.path().join("host");
        let mut call = agent_runtime::provider::fake::tool_call_fragments(
            0,
            "fault-call",
            "wasm_6_faults_guest__probe",
            &serde_json::json!({"action":action}).to_string(),
        );
        call.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        let provider = Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![
                ScriptedStream::new(call),
                ScriptedStream::new(vec![
                    ProviderStreamEvent::TextDelta {
                        text: "session survived".into(),
                    },
                    ProviderStreamEvent::Finish {
                        reason: FinishReason::Stop,
                    },
                ]),
            ],
        ));
        request.provider = Some(provider.clone());
        let harness =
            crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap();
        let runtime = build(harness).await.unwrap();
        let session = runtime
            .runtime()
            .start_session(StartSession::new())
            .await
            .unwrap();
        session
            .send(UserInput::text("call wasm_6_faults_guest__probe"))
            .unwrap()
            .completed()
            .await;
        assert_eq!(
            provider.requests().len(),
            2,
            "{action}: provider must continue after guest failure"
        );
        let history = format!("{:?}", session.history());
        assert!(
            !history.contains("not active"),
            "fixture must enter the guest: {history}"
        );
        assert!(
            history.contains("module "),
            "guest failure must be reported: {history}"
        );
    }
}

#[tokio::test]
async fn plugin_wasm_tools_reach_inheriting_children_and_keep_ordinary_approval() {
    use agent_runtime::delegation::ChildRuntimeFactory;
    use agent_runtime_core::delegation::{
        ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
    };
    let workspace = tempfile::tempdir().unwrap();
    let grant = smith_wasm_host::CapabilityGrant {
        capability: smith_module::ModuleCapability::WorkspaceWrite,
        roots: vec!["workspace".into()],
        hosts: vec![],
        allow_http: false,
    };
    let module = smith_wasm_host::WasmModule::load(
        smith_wasm_host::WasmEngine::new().unwrap(),
        include_bytes!("../../../../smith-wasm-host/tests/fixtures/tools.wasm"),
        smith_wasm_host::WasmModuleConfig {
            plugin_name: "child-plugin".into(),
            module_id: "test/module".into(),
            requested_capabilities: vec![grant.clone()],
            granted_capabilities: vec![grant],
            host_state_dir: workspace.path().join("host"),
            data_dir: workspace.path().join("data"),
            cache_dir: workspace
                .path()
                .join("host")
                .join(smith_wasm_host::COMPONENT_CACHE_DIRECTORY),
            limits: Default::default(),
            session_health: Arc::new(smith_module::ModuleHealth::default()),
            reserved_tool_names: BTreeSet::new(),
        },
    )
    .unwrap();
    let mut request = request(workspace.path(), vec![]);
    request.config.user_dir = workspace.path().join("host");
    request.config.agent.profile.delegation.value = true;
    request.config.approval.mode = sourced(ApprovalMode::Ask);
    let approval = Arc::new(WasmApproval(
        AtomicUsize::new(0),
        workspace.path().join("fixture.txt"),
    ));
    request.approval = Some(approval.clone());
    request.plugin_profiles.insert(
        request.config.agent.profile.name.clone(),
        PluginProfile {
            wasm_modules: ModuleComposition::with_defaults(
                vec![CompiledModule {
                    origin: module.origin(),
                    module,
                }],
                vec![],
            ),
            ..Default::default()
        },
    );
    let name = "wasm_12_child-plugin_guest__probe";
    let mut call = agent_runtime::provider::fake::tool_call_fragments(
        0,
        "child-wasm",
        name,
        r#"{"action":"fs-write"}"#,
    );
    call.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    request.provider = Some(Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(call),
            ScriptedStream::new(vec![ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            }]),
        ],
    )));
    let runtime =
        build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap())
            .await
            .unwrap();
    let spec = ChildSpec {
        task: UserInput::text(format!("call {name}")),
        model: ChildModelSelection::Inherit,
        limits: ChildLimits {
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        },
        tools: ToolViewScope::All,
        workspace: WorkspacePolicy::SharedProject,
    };
    let child_runtime = runtime
        .delegation()
        .unwrap()
        .factory
        .child_builder(&spec)
        .unwrap()
        .build()
        .unwrap();
    let child = child_runtime
        .start_session(StartSession::new())
        .await
        .unwrap();
    child.send(spec.task).unwrap().completed().await;
    assert_eq!(
        approval.0.load(Ordering::Relaxed),
        1,
        "child WASM tools must reach ordinary approval"
    );
    assert_eq!(
        std::fs::read(workspace.path().join("fixture.txt")).unwrap(),
        b"written"
    );
    child.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_child_rebuild_omits_disabled_wasm_schemas_without_disabling_siblings() {
    use agent_runtime::delegation::ChildRuntimeFactory;
    use agent_runtime_core::delegation::{
        ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
    };
    let workspace = tempfile::tempdir().unwrap();
    let health = Arc::new(smith_module::ModuleHealth::default());
    let module = smith_wasm_host::WasmModule::load(
        smith_wasm_host::WasmEngine::new().unwrap(),
        include_bytes!("../../../../smith-wasm-host/tests/fixtures/tools.wasm"),
        smith_wasm_host::WasmModuleConfig {
            plugin_name: "view".into(),
            module_id: "test/module".into(),
            requested_capabilities: vec![],
            granted_capabilities: vec![],
            host_state_dir: workspace.path().join("host"),
            data_dir: workspace.path().join("data"),
            cache_dir: workspace
                .path()
                .join("host")
                .join(smith_wasm_host::COMPONENT_CACHE_DIRECTORY),
            limits: Default::default(),
            session_health: health.clone(),
            reserved_tool_names: BTreeSet::new(),
        },
    )
    .unwrap();
    let mut request = request(workspace.path(), vec![]);
    request.config.user_dir = workspace.path().join("host");
    request.config.agent.profile.delegation.value = true;
    request.plugin_profiles.insert(
        request.config.agent.profile.name.clone(),
        PluginProfile {
            wasm_modules: ModuleComposition::with_defaults(
                vec![CompiledModule {
                    origin: module.origin(),
                    module,
                }],
                vec![],
            ),
            ..Default::default()
        },
    );
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        (0..2)
            .map(|_| {
                ScriptedStream::new(vec![ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                }])
            })
            .collect(),
    ));
    request.provider = Some(provider.clone());
    let runtime =
        build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap())
            .await
            .unwrap();
    health.fail("parent disabled".into());
    let failed = SessionId::new("failed-child");
    health.child_session(&failed).fail("child disabled".into());
    let name = "wasm_4_view_guest__probe";
    let spec = ChildSpec {
        task: UserInput::text(format!("call {name}")),
        model: ChildModelSelection::Inherit,
        limits: ChildLimits {
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        },
        tools: ToolViewScope::All,
        workspace: WorkspacePolicy::SharedProject,
    };
    for id in [failed, SessionId::new("healthy-child")] {
        let child_runtime = runtime
            .delegation()
            .unwrap()
            .factory
            .child_builder(&spec)
            .unwrap()
            .build()
            .unwrap();
        let child = child_runtime
            .start_session(StartSession::create(id, Vec::new()))
            .await
            .unwrap();
        child.send(spec.task.clone()).unwrap().completed().await;
        child.shutdown().await.unwrap();
    }
    let requests = provider.requests();
    assert!(
        !requests[0].tools.iter().any(|schema| schema.name == name),
        "a failed child must not offer the tool after rebuild"
    );
    assert!(
        requests[1].tools.iter().any(|schema| schema.name == name),
        "parent or sibling failures must not hide a healthy child's tool"
    );
}

mod web;

#[tokio::test]
async fn capability_hint_fragment_is_stable_revisioned_and_absent_without_hints() {
    use agent_runtime::context::{FragmentContent, FragmentKind};
    use smith_module::{ModuleDescriptor, ModuleReport, MountPlan, SetupHint};

    let mut plan = MountPlan::default();
    assert!(super::super::modules::capability_hints(&plan).is_none());
    plan.report.push(ModuleReport {
        descriptor: ModuleDescriptor {
            id: "vision".into(),
            description: "vision".into(),
            default_enabled: true,
            origin: ModuleOrigin::FirstParty,
            compiled_in: true,
        },
        state: ModuleState::Inactive {
            reason: "no vision model".into(),
            setup_hint: None,
        },
        health: None,
    });
    assert!(super::super::modules::capability_hints(&plan).is_none());
    plan.report[0].state = ModuleState::Inactive {
        reason: "no vision model".into(),
        setup_hint: Some(SetupHint::new(
            "no vision model is configured, so images can't be viewed",
            "smith.guide.vision",
            vec!["visual".into()],
        )),
    };
    let first = super::super::modules::capability_hints(&plan).unwrap();
    let second = super::super::modules::capability_hints(&plan).unwrap();
    assert_eq!(first.body, second.body);
    assert_eq!(first.revision, second.revision);
    let view = |turn: &str| ContextView {
        session: agent_runtime_core::ids::SessionId::new("hints"),
        turn: agent_runtime_core::ids::TurnId::new(turn),
        history: Arc::from([]),
        activation: agent_runtime::registry::Fingerprint::of("hints"),
        state: None,
    };
    let patch = first.contribute(&view("one")).await.unwrap();
    let again = second.contribute(&view("two")).await.unwrap();
    assert_eq!(patch.fragments, again.fragments);
    let [fragment] = patch.fragments.as_slice() else {
        panic!("one fragment")
    };
    assert_eq!(fragment.kind, FragmentKind::DeveloperInstruction);
    let FragmentContent::Text(body) = &fragment.content else {
        panic!("text")
    };
    assert_eq!(body.lines().count(), 2);
    assert!(body.contains("only when the current request needs it"));
    assert!(body.contains("vision: no vision model"));
    assert!(body.contains("Profiles that accept images: visual"));
    assert!(body.contains("activate smith.guide.vision"));
    if let ModuleState::Inactive {
        setup_hint: Some(hint),
        ..
    } = &mut plan.report[0].state
    {
        hint.candidates.push("another".into());
    }
    let changed = super::super::modules::capability_hints(&plan).unwrap();
    assert_ne!(first.revision, changed.revision);
    let mut image = plan.report[0].clone();
    image.descriptor.id = "image-generation".into();
    image.state = ModuleState::Inactive {
        reason: "no image binding".into(),
        setup_hint: Some(SetupHint::new(
            "images cannot be generated",
            "smith.guide.setup",
            vec![],
        )),
    };
    plan.report.push(image);
    let combined = super::super::modules::capability_hints(&plan).unwrap();
    let patch = combined.contribute(&view("three")).await.unwrap();
    assert_eq!(patch.fragments.len(), 1);
    assert_eq!(patch.fragments[0].id.as_str(), "smith.capability-hints");
    assert_eq!(combined.body.lines().count(), 3);
    assert!(
        combined
            .body
            .contains("image-generation: images cannot be generated")
    );
}

#[test]
fn read_only_posture_keeps_hints_only_for_admitted_network_modules() {
    use smith_module::{ModuleDescriptor, ModulePosture, ModuleReport, MountPlan, SetupHint};

    let inactive = |id: &str| ModuleReport {
        descriptor: ModuleDescriptor {
            id: id.into(),
            description: id.into(),
            default_enabled: true,
            origin: ModuleOrigin::FirstParty,
            compiled_in: true,
        },
        state: ModuleState::Inactive {
            reason: "not configured".into(),
            setup_hint: Some(SetupHint::new("missing", "smith.guide.setup", vec![])),
        },
        health: None,
    };
    let hinted = |plan: &MountPlan| {
        plan.report
            .iter()
            .filter(|report| {
                matches!(
                    report.state,
                    ModuleState::Inactive {
                        setup_hint: Some(_),
                        ..
                    }
                )
            })
            .map(|report| report.descriptor.id.clone())
            .collect::<Vec<_>>()
    };
    let plan = || MountPlan {
        report: vec![inactive("vision"), inactive("image-generation")],
        ..MountPlan::default()
    };

    let mut writable = plan();
    super::super::modules::enforce_posture(
        &mut writable,
        ModulePosture::ReadWrite,
        &["web".into(), "vision".into()],
    );
    assert_eq!(hinted(&writable), ["vision", "image-generation"]);

    let mut planning = plan();
    super::super::modules::enforce_posture(
        &mut planning,
        ModulePosture::ReadOnly,
        &["web".into(), "vision".into()],
    );
    assert_eq!(hinted(&planning), ["vision"]);

    let mut strict = plan();
    super::super::modules::enforce_posture(&mut strict, ModulePosture::ReadOnly, &[]);
    assert!(hinted(&strict).is_empty());
}
