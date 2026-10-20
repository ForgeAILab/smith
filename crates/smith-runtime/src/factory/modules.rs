//! Adapts returned module values into existing registry, pipeline, and evidence.

use std::sync::Arc;

use agent_runtime::registry::RegistrySource;
use agent_runtime::runtime::RuntimeBuilder;
use agent_runtime_core::tool::Tool;
use smith_module::{
    ImageBinding, ModuleContext, ModuleContribution, ModuleOrigin, ModulePosture, ModuleSettings,
    MountPlan, MountedModule, PipelineComponent, mount_modules,
};

use super::{FactoryError, RuntimeRequest};
use crate::harness::{
    CapabilitySet, Contribution, ModuleId, ModuleProvenance, ModuleRevision, ModuleSpec,
    ModuleTrust, tool_capabilities,
};
use crate::transport::ReqwestTransport;

/// Computes the same mount outcomes as runtime construction without requiring
/// an approval surface or constructing a runtime, session, or persistence.
pub async fn module_report(
    request: &RuntimeRequest,
) -> Result<Vec<smith_module::ModuleReport>, FactoryError> {
    super::provider::validate_pool_references(request)?;
    let prepared = super::provider::prepare(request).await?;
    let provider = super::provider::construct_runtime(
        request,
        prepared.adapter,
        prepared.endpoint,
        prepared.secret,
        &prepared.profile.profile,
        &prepared.reasoning,
        prepared.command.map(|command| command.provider),
    )?;
    let web = super::web::prepare(
        request,
        provider
            .provider
            .capabilities(&prepared.model)
            .map(|caps| caps.hosted_web_search)
            .unwrap_or(false),
    )
    .await;
    Ok(prepare(
        request,
        &web,
        provider.image_binding,
        Box::pin(super::provider::prepare_vision_route(
            request,
            Arc::new(crate::vision::VisionAccounting::default()),
        ))
        .await?
        .map(|vision| vision.binding),
        prepared.profile.profile.limits.max_input_tokens,
        request.semantic_summary.is_some(),
        Arc::new(crate::session_history::LiveSessionHistory::default()),
        None,
    )?
    .report)
}

// Independent host-owned services are resolved before module mounting.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    request: &RuntimeRequest,
    web: &super::web::WebRoute,
    image_binding: Option<ImageBinding>,
    vision: Option<smith_module::VisionBinding>,
    max_input_tokens: u32,
    semantic_summary_enabled: bool,
    session_history: Arc<crate::session_history::LiveSessionHistory>,
    model: Option<Arc<dyn smith_module::ModuleModel>>,
) -> Result<MountPlan, FactoryError> {
    let mut composition = request.modules.clone();
    if let Some(profile) = request
        .plugin_profiles
        .get(&request.config.agent.profile.name)
    {
        composition
            .compiled
            .extend(profile.wasm_modules.compiled.clone());
        composition.known.extend(profile.wasm_modules.known.clone());
        composition
            .enabled
            .extend(profile.wasm_modules.enabled.clone());
    }
    if composition.compiled.is_empty() && composition.known.is_empty() {
        return Ok(MountPlan::default());
    }
    let mut context = context(
        request,
        web.settings.clone(),
        image_binding,
        vision,
        max_input_tokens,
        semantic_summary_enabled,
        session_history,
    )?;
    context.model = model;
    let mut plan = mount_modules(&composition, &context);
    report_web_diagnostic(&mut plan, web.diagnostic.as_deref());
    apply_sandbox_authority(&mut plan);
    enforce_posture(
        &mut plan,
        context.posture,
        &request.config.agent.read_only_network_modules.value,
    );
    reject_sandbox_tool_collisions(&mut plan, request);
    reject_sandbox_pipeline_ordering(&mut plan);
    Ok(plan)
}

/// Guest declarations cannot classify their own authority. Use exactly the
/// shared MCP policy for an unreviewed external service, including preparation.
pub(crate) fn apply_sandbox_authority(plan: &mut MountPlan) {
    for module in &mut plan.mounted {
        if !matches!(module.descriptor.origin, ModuleOrigin::Sandboxed { .. }) {
            continue;
        }
        let policy = agent_runtime_mcp::McpServerConfig::stdio(
            &module.descriptor.id,
            format!("wasm:{}", module.descriptor.id),
        );
        for contribution in &mut module.contributions {
            if let ModuleContribution::Tool(tool) = contribution {
                let mut spec = tool.spec();
                spec.effects = policy.resolved_effect_floor();
                spec.permission_upper_bound = spec.effects.permission_upper_bound();
                *tool = Arc::new(SandboxTool {
                    inner: tool.clone(),
                    spec,
                });
            }
        }
    }
}

#[derive(Debug)]
struct SandboxTool {
    inner: Arc<dyn Tool>,
    spec: agent_runtime_core::tool::ToolSpec,
}

#[async_trait::async_trait]
impl Tool for SandboxTool {
    fn spec(&self) -> agent_runtime_core::tool::ToolSpec {
        self.spec.clone()
    }

    async fn invoke(
        &self,
        prepared: agent_runtime_core::tool::PreparedToolCall,
        context: &agent_runtime_core::tool::InvocationContext,
    ) -> Result<agent_runtime_core::tool::ToolOutcome, agent_runtime_core::error::RuntimeError>
    {
        self.inner.invoke(prepared, context).await
    }
}

pub(crate) fn enforce_posture(
    plan: &mut MountPlan,
    posture: ModulePosture,
    network_modules: &[String],
) {
    // Trusted modules receive the posture, but the host still enforces it at
    // registration, just as it does for trusted embedding tools.
    if posture == ModulePosture::ReadOnly {
        for module in &mut plan.mounted {
            let reviewed = matches!(module.descriptor.origin, ModuleOrigin::FirstParty)
                && network_modules.contains(&module.descriptor.id);
            module
                .contributions
                .retain(|contribution| match contribution {
                    ModuleContribution::Tool(tool) => {
                        super::capabilities::read_only_extension(tool.spec())
                            || (reviewed
                                && super::capabilities::read_only_network_extension(tool.spec()))
                    }
                    _ => true,
                });
        }
        // A hint must name a fix that works here: only modules this posture
        // would admit keep theirs.
        for report in &mut plan.report {
            let reviewed = matches!(report.descriptor.origin, ModuleOrigin::FirstParty)
                && network_modules.contains(&report.descriptor.id);
            if let smith_module::ModuleState::Inactive { setup_hint, .. } = &mut report.state
                && !reviewed
            {
                *setup_hint = None;
            }
        }
    }
}

pub(super) fn context(
    request: &RuntimeRequest,
    web: smith_module::WebSettings,
    image_binding: Option<ImageBinding>,
    vision: Option<smith_module::VisionBinding>,
    max_input_tokens: u32,
    semantic_summary_enabled: bool,
    session_history: Arc<crate::session_history::LiveSessionHistory>,
) -> Result<ModuleContext, FactoryError> {
    let transport = Arc::new(
        ReqwestTransport::new(request.transport.clone()).map_err(FactoryError::Transport)?,
    );
    Ok(ModuleContext {
        settings: ModuleSettings::new(),
        is_child_session: matches!(request.surface, super::HostSurface::Child),
        workspace_root: request
            .workspace
            .as_ref()
            .map(|workspace| std::path::PathBuf::from(workspace.root())),
        user_dir: request.config.user_dir.clone(),
        posture: if request.config.agent.active_posture().is_read_only() {
            ModulePosture::ReadOnly
        } else {
            ModulePosture::ReadWrite
        },
        transport: transport.clone(),
        http: Some(transport),
        model: None,
        image_binding,
        vision,
        vision_unset: matches!(
            request.config.agent.profile.vision,
            smith_config::resolve::ResolvedVision::Unset
        ),
        image_capable_profiles: request.config.agent.image_capable_profiles.clone(),
        web: Some(web),
        main_model_accepts_images: request.config.model_image_input.value
            == smith_config::resolve::ImageInput::Accepts,
        session_history: Some(session_history),
        semantic_summary_enabled,
        max_input_tokens,
        built_in_tools: request.built_in_tools,
    })
}

pub(super) fn for_config(
    composition: &smith_module::ModuleComposition,
    config: &smith_config::resolve::ResolvedConfig,
) -> smith_module::ModuleComposition {
    let mut modules = composition.clone();
    for (id, module) in &config.modules {
        if module.enabled.value {
            modules.enabled.insert(id.clone());
        } else {
            modules.enabled.remove(id);
        }
    }
    modules.settings.insert(
        "image-generation".into(),
        [
            (
                "model".into(),
                smith_module::SettingValue::String(config.image_generation.model.value.clone()),
            ),
            (
                "quality".into(),
                smith_module::SettingValue::String(config.image_generation.quality.value.clone()),
            ),
            (
                "size".into(),
                smith_module::SettingValue::String(config.image_generation.size.value.clone()),
            ),
        ]
        .into(),
    );
    modules
}

pub(super) fn tools(plan: &MountPlan) -> Vec<(Arc<dyn Tool>, RegistrySource)> {
    plan.mounted
        .iter()
        .flat_map(|module| {
            let source = match module.descriptor.origin {
                ModuleOrigin::FirstParty => RegistrySource::BuiltIn,
                ModuleOrigin::ThirdParty { .. } | ModuleOrigin::Sandboxed { .. } => {
                    RegistrySource::Host
                }
            };
            module
                .contributions
                .iter()
                .filter_map(move |contribution| match contribution {
                    ModuleContribution::Tool(tool) => Some((tool.clone(), source)),
                    _ => None,
                })
        })
        .collect()
}

pub(super) fn specs(plan: &MountPlan) -> Result<Vec<ModuleSpec>, FactoryError> {
    plan.mounted.iter().map(module_spec).collect()
}

fn module_spec(module: &MountedModule) -> Result<ModuleSpec, FactoryError> {
    let mut required = CapabilitySet::new();
    let contributions = module
        .contributions
        .iter()
        .map(|contribution| match contribution {
            ModuleContribution::Tool(tool) => {
                let capabilities = tool_capabilities(tool.as_ref());
                required.extend(capabilities.iter().copied());
                Contribution::Tool {
                    name: tool.spec().name,
                    required: capabilities,
                }
            }
            ModuleContribution::Pipeline(component) => Contribution::Pipeline {
                phase: component.phase(),
                component: component.descriptor().id().as_str().to_owned(),
            },
            ModuleContribution::Observer { name, .. } => {
                Contribution::Observer { name: name.clone() }
            }
            ModuleContribution::Command(command) => Contribution::Command {
                name: command.name.clone(),
            },
            ModuleContribution::Watcher { name, .. } => {
                Contribution::Watcher { name: name.clone() }
            }
            ModuleContribution::StatusItem { name, .. } => {
                Contribution::StatusItem { name: name.clone() }
            }
        })
        .collect();
    let (id, provenance) = match &module.descriptor.origin {
        ModuleOrigin::FirstParty => (
            format!("smith/{}", module.descriptor.id),
            ModuleProvenance::BuiltIn,
        ),
        ModuleOrigin::Sandboxed { plugin, .. } => (
            format!(
                "wasm/{}/{}",
                &smith_config::trust::ContentDigest::of(plugin.as_bytes()).as_hex()[..16],
                module.descriptor.id
            ),
            ModuleProvenance::UserManifest(plugin.clone()),
        ),
        ModuleOrigin::ThirdParty { crate_name } => (
            format!(
                "third-party/{}/{}",
                crate_name.replace('_', "-"),
                module.descriptor.id
            ),
            ModuleProvenance::CompiledThirdParty(crate_name.clone()),
        ),
    };
    let (trust, requested, granted) = match &module.descriptor.origin {
        ModuleOrigin::Sandboxed {
            requested_capabilities,
            granted_capabilities,
            ..
        } => (
            ModuleTrust::Sandboxed,
            requested_capabilities.clone(),
            granted_capabilities.clone(),
        ),
        _ => (ModuleTrust::TrustedNative, required.clone(), required),
    };
    Ok(ModuleSpec {
        id: ModuleId::parse(id)?,
        revision: ModuleRevision::parse(module.revision.clone())?,
        provenance,
        trust,
        contributions,
        requested_capabilities: requested,
        granted_capabilities: granted,
    })
}

pub(crate) fn apply(mut builder: RuntimeBuilder, plan: &MountPlan) -> RuntimeBuilder {
    for contribution in plan.mounted.iter().flat_map(|module| &module.contributions) {
        builder = match contribution {
            ModuleContribution::Pipeline(component) => match component {
                PipelineComponent::HistoryProjector(component) => {
                    builder.history_projector(component.clone())
                }
                PipelineComponent::ContextContributor(component) => {
                    builder.context_contributor(component.clone())
                }
                PipelineComponent::ToolViewResolver(component) => {
                    builder.tool_view_resolver(component.clone())
                }
                PipelineComponent::ModelInterceptor(component) => {
                    builder.model_interceptor(component.clone())
                }
                PipelineComponent::ToolOutputProcessor(component) => {
                    builder.tool_output_processor(component.clone())
                }
                PipelineComponent::TurnCommitHook(component) => {
                    builder.turn_commit_hook(component.clone())
                }
            },
            ModuleContribution::Observer { observer, .. } => builder.observer(observer.clone()),
            ModuleContribution::Tool(_)
            | ModuleContribution::Command(_)
            | ModuleContribution::StatusItem { .. }
            | ModuleContribution::Watcher { .. } => builder,
        };
    }
    if let Some(hints) = capability_hints(plan) {
        builder = builder.context_contributor(Arc::new(hints));
    }
    builder
}

pub(super) fn capability_hints(plan: &MountPlan) -> Option<CapabilityHints> {
    let lines = plan
        .report
        .iter()
        .filter_map(|report| match &report.state {
            smith_module::ModuleState::Inactive {
                setup_hint: Some(hint),
                ..
            } => Some(format!("{}: {}", report.descriptor.id, hint.agent_text())),
            _ => None,
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    let body = format!(
        "Raise a missing capability with the user only when the current request needs it; do not raise setup gaps at session start.\n{}",
        lines.join("\n"),
    );
    use sha2::{Digest, Sha256};
    let revision = agent_runtime::registry::RegistryRevision::new(format!(
        "{:x}",
        Sha256::digest(body.as_bytes())
    ));
    Some(CapabilityHints { body, revision })
}

#[derive(Debug)]
pub(super) struct CapabilityHints {
    pub(super) body: String,
    pub(super) revision: agent_runtime::registry::RegistryRevision,
}

#[async_trait::async_trait]
impl agent_runtime::harness::ContextContributor for CapabilityHints {
    fn descriptor(&self) -> agent_runtime::harness::ComponentDescriptor {
        agent_runtime::harness::ComponentDescriptor::new(
            "smith.capability-hints",
            self.revision.clone(),
        )
    }

    async fn contribute(
        &self,
        _: &agent_runtime::harness::ContextView,
    ) -> Result<agent_runtime::harness::ContextPatch, agent_runtime_core::error::RuntimeError> {
        use agent_runtime::context::{
            ContextFragment, FragmentContent, FragmentKind, FragmentSource,
        };
        Ok(agent_runtime::harness::ContextPatch::new(vec![
            ContextFragment::new(
                "smith.capability-hints",
                FragmentKind::DeveloperInstruction,
                FragmentSource::Host,
                self.revision.clone(),
                FragmentContent::Text(self.body.clone()),
            ),
        ]))
    }
}

fn reject_sandbox_tool_collisions(plan: &mut MountPlan, request: &RuntimeRequest) {
    let mut taken = super::capabilities::tools(request)
        .iter()
        .map(|tool| tool.spec().name)
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(mcp) = &request.mcp {
        taken.extend(mcp.tools().iter().map(|tool| tool.spec().name));
    }
    reject_sandbox_tool_names(plan, taken);
}

pub(crate) fn reject_sandbox_tool_names(
    plan: &mut MountPlan,
    mut taken: std::collections::BTreeSet<String>,
) {
    // Native modules keep their existing registration semantics and take precedence.
    for module in &plan.mounted {
        if !matches!(module.descriptor.origin, ModuleOrigin::Sandboxed { .. }) {
            taken.extend(module.contributions.iter().filter_map(|c| match c {
                ModuleContribution::Tool(tool) => Some(tool.spec().name),
                _ => None,
            }));
        }
    }
    plan.mounted.retain(|module| {
        if !matches!(module.descriptor.origin, ModuleOrigin::Sandboxed { .. }) {
            return true;
        }
        let names = module
            .contributions
            .iter()
            .filter_map(|c| match c {
                ModuleContribution::Tool(tool) => Some(tool.spec().name),
                _ => None,
            })
            .collect::<Vec<_>>();
        if let Some(name) = names.iter().find(|name| taken.contains(*name)) {
            let reason = format!("sandboxed tool collision: {name}");
            if let Some(report) = plan
                .report
                .iter_mut()
                .find(|report| report.descriptor.id == module.descriptor.id)
            {
                report.state = smith_module::ModuleState::Failed {
                    reason: reason.clone(),
                };
                if let Some(health) = &report.health {
                    health.fail(reason);
                }
            }
            false
        } else {
            taken.extend(names);
            true
        }
    });
}

pub(crate) fn reject_sandbox_pipeline_ordering(plan: &mut MountPlan) {
    let has_guest_pipeline = |module: &MountedModule| {
        matches!(module.descriptor.origin, ModuleOrigin::Sandboxed { .. })
            && module
                .contributions
                .iter()
                .any(|c| matches!(c, ModuleContribution::Pipeline(_)))
    };
    if !plan.mounted.iter().any(has_guest_pipeline) {
        return;
    }
    let mut builder = agent_runtime::harness::HarnessPipelineBuilder::new();
    for contribution in plan.mounted.iter().flat_map(|module| &module.contributions) {
        if let ModuleContribution::Pipeline(component) = contribution {
            match component {
                PipelineComponent::HistoryProjector(c) => {
                    builder.history_projector(c.clone());
                }
                PipelineComponent::ContextContributor(c) => {
                    builder.context_contributor(c.clone());
                }
                PipelineComponent::ToolViewResolver(c) => {
                    builder.tool_view_resolver(c.clone());
                }
                PipelineComponent::ModelInterceptor(c) => {
                    builder.model_interceptor(c.clone());
                }
                PipelineComponent::ToolOutputProcessor(c) => {
                    builder.tool_output_processor(c.clone());
                }
                PipelineComponent::TurnCommitHook(c) => {
                    builder.turn_commit_hook(c.clone());
                }
            }
        }
    }
    if let Err(error) = builder.seal() {
        // Ordering is validated by the shared pipeline, before guest metadata
        // can make RuntimeBuilder fail and prevent the session from starting.
        plan.mounted.retain(|module| {
            if !has_guest_pipeline(module) {
                return true;
            }
            if let Some(report) = plan
                .report
                .iter_mut()
                .find(|r| r.descriptor.id == module.descriptor.id)
            {
                let reason = format!("sandboxed pipeline ordering rejected: {error}");
                if let Some(health) = &report.health {
                    health.fail(reason.clone());
                }
                report.state = smith_module::ModuleState::Failed { reason };
            }
            false
        });
    }
}

/// Host protection, not a guest contribution: apply failure state to the actual
/// session's ability view when a safe-boundary rebuild creates that view.
pub(crate) fn sandbox_health_filter(
    composition: &smith_module::ModuleComposition,
    plan: &MountPlan,
    child: bool,
) -> Option<Arc<dyn agent_runtime::harness::ToolViewResolver>> {
    let modules = plan
        .mounted
        .iter()
        .filter(|module| matches!(module.descriptor.origin, ModuleOrigin::Sandboxed { .. }))
        .filter_map(|module| {
            let health = composition
                .compiled
                .iter()
                .find(|compiled| compiled.module.id() == module.descriptor.id)?
                .module
                .session_health()?;
            let ids = module
                .contributions
                .iter()
                .filter_map(|contribution| match contribution {
                    ModuleContribution::Tool(tool) => {
                        Some(agent_runtime::registry::RegistryId::tool(tool.spec().name))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            Some(SandboxHealthEntry { health, ids })
        })
        .collect::<Vec<_>>();
    (!modules.is_empty()).then(|| {
        Arc::new(SandboxHealthView { modules, child })
            as Arc<dyn agent_runtime::harness::ToolViewResolver>
    })
}

#[derive(Debug)]
struct SandboxHealthEntry {
    health: Arc<smith_module::ModuleHealth>,
    ids: Vec<agent_runtime::registry::RegistryId>,
}
#[derive(Debug)]
struct SandboxHealthView {
    modules: Vec<SandboxHealthEntry>,
    child: bool,
}
#[async_trait::async_trait]
impl agent_runtime::harness::ToolViewResolver for SandboxHealthView {
    fn descriptor(&self) -> agent_runtime::harness::ComponentDescriptor {
        agent_runtime::harness::ComponentDescriptor::new(
            "smith.wasm-health",
            agent_runtime::registry::RegistryRevision::new("1"),
        )
    }
    async fn resolve(
        &self,
        view: &agent_runtime::harness::ToolViewContext,
    ) -> Result<agent_runtime::harness::ToolViewPatch, agent_runtime_core::error::RuntimeError>
    {
        let deny = self
            .modules
            .iter()
            .filter(|entry| {
                if self.child {
                    entry
                        .health
                        .child_session(&view.session)
                        .failure()
                        .is_some()
                } else {
                    entry.health.failure().is_some()
                }
            })
            .flat_map(|entry| entry.ids.clone())
            .collect();
        Ok(agent_runtime::harness::ToolViewPatch {
            deny,
            ..Default::default()
        })
    }
}

#[derive(Debug)]
struct WebSearchDiagnostic(String);

impl smith_module::StatusSource for WebSearchDiagnostic {
    fn current(&self) -> Option<smith_module::StatusItem> {
        Some(smith_module::StatusItem::new(
            "web-search-unavailable",
            &self.0,
            Some(smith_module::StatusSeverity::Warning),
        ))
    }
}

pub(crate) fn report_web_diagnostic(plan: &mut MountPlan, reason: Option<&str>) {
    if let Some(reason) = reason
        && let Some(module) = plan
            .mounted
            .iter_mut()
            .find(|module| module.descriptor.id == "web")
    {
        module.contributions.push(ModuleContribution::StatusItem {
            name: "web-search-unavailable".into(),
            source: Arc::new(WebSearchDiagnostic(reason.into())),
        });
    }
}
