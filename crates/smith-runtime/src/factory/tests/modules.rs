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
    StatusSeverity,
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
            ModuleContribution::Tool(Arc::new(smith_tools::ReadTool)),
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
            ModuleContribution::StatusItem(StatusItem::new(
                "pressure",
                "context pressure",
                Some(StatusSeverity::Warning),
            )),
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
