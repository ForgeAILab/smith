//! One-image questions for agents whose active model cannot accept images.

mod guidance;
mod history;
mod tool;

use smith_module::{
    Module, ModuleContext, ModuleContribution, ModuleError, Mounted, PipelineComponent,
};
use std::sync::Arc;

/// Optional image inspection through a separately resolved vision model.
#[derive(Debug, Default)]
pub struct VisionModule;

impl Module for VisionModule {
    fn id(&self) -> &str {
        "vision"
    }

    fn revision(&self) -> &str {
        "smith-vision-v1"
    }

    fn description(&self) -> &str {
        "Ask a vision model about one image"
    }

    fn default_enabled(&self) -> bool {
        true
    }

    fn mount(&self, context: &ModuleContext) -> Result<Mounted, ModuleError> {
        let reason = if !context.built_in_tools {
            Some("built-in tools are disabled")
        } else if context.main_model_accepts_images {
            Some("the active main model accepts images")
        } else if context.vision.is_none() {
            Some("no vision model is configured")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Ok(Mounted::Inactive {
                reason: reason.into(),
            });
        }
        let binding = context
            .vision
            .as_ref()
            .expect("checked vision binding")
            .clone();
        let history = context
            .session_history
            .clone()
            .ok_or_else(|| ModuleError("vision requires canonical session history".into()))?;
        Ok(Mounted::Contributions(vec![
            ModuleContribution::Pipeline(PipelineComponent::ContextContributor(Arc::new(
                guidance::VisionGuidance,
            ))),
            ModuleContribution::Pipeline(PipelineComponent::TurnCommitHook(binding.usage.clone())),
            ModuleContribution::Tool(Arc::new(tool::ViewImageTool {
                binding,
                history,
                generated_dir: context.user_dir.join("generated_images"),
            })),
        ]))
    }
}

#[cfg(test)]
mod tests;
