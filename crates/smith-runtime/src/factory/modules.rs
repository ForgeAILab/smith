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
    Ok(prepare(
        request,
        provider.image_binding,
        prepared.profile.profile.limits.max_input_tokens,
        request.semantic_summary.is_some(),
        Arc::new(crate::session_history::LiveSessionHistory::default()),
    )?
    .report)
}

pub(super) fn prepare(
    request: &RuntimeRequest,
    image_binding: Option<ImageBinding>,
    max_input_tokens: u32,
    semantic_summary_enabled: bool,
    session_history: Arc<crate::session_history::LiveSessionHistory>,
) -> Result<MountPlan, FactoryError> {
    if request.modules.compiled.is_empty() && request.modules.known.is_empty() {
        return Ok(MountPlan::default());
    }
    let context = context(
        request,
        image_binding,
        max_input_tokens,
        semantic_summary_enabled,
        session_history,
    )?;
    let mut plan = mount_modules(&request.modules, &context);
    enforce_posture(&mut plan, context.posture);
    Ok(plan)
}

pub(crate) fn enforce_posture(plan: &mut MountPlan, posture: ModulePosture) {
    // Trusted modules receive the posture, but the host still enforces it at
    // registration, just as it does for trusted embedding tools.
    if posture == ModulePosture::ReadOnly {
        for module in &mut plan.mounted {
            module
                .contributions
                .retain(|contribution| match contribution {
                    ModuleContribution::Tool(tool) => {
                        super::capabilities::read_only_extension(tool.spec())
                    }
                    _ => true,
                });
        }
    }
}

pub(super) fn context(
    request: &RuntimeRequest,
    image_binding: Option<ImageBinding>,
    max_input_tokens: u32,
    semantic_summary_enabled: bool,
    session_history: Arc<crate::session_history::LiveSessionHistory>,
) -> Result<ModuleContext, FactoryError> {
    Ok(ModuleContext {
        settings: ModuleSettings::new(),
        user_dir: request.config.user_dir.clone(),
        posture: if request.config.agent.active_posture().is_read_only() {
            ModulePosture::ReadOnly
        } else {
            ModulePosture::ReadWrite
        },
        transport: Arc::new(
            ReqwestTransport::new(request.transport.clone()).map_err(FactoryError::Transport)?,
        ),
        image_binding,
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
                ModuleOrigin::ThirdParty { .. } => RegistrySource::Host,
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
        ModuleOrigin::ThirdParty { crate_name } => (
            format!(
                "third-party/{}/{}",
                crate_name.replace('_', "-"),
                module.descriptor.id
            ),
            ModuleProvenance::CompiledThirdParty(crate_name.clone()),
        ),
    };
    Ok(ModuleSpec {
        id: ModuleId::parse(id)?,
        revision: ModuleRevision::parse(module.revision.clone())?,
        provenance,
        trust: ModuleTrust::TrustedNative,
        contributions,
        requested_capabilities: required.clone(),
        granted_capabilities: required,
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
            | ModuleContribution::StatusItem { .. } => builder,
        };
    }
    builder
}
