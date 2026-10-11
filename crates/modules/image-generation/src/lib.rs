//! Image generation and editing mounted through Smith's module contract.

mod image_api;
mod image_history;

use std::sync::Arc;

use smith_module::{
    Module, ModuleContext, ModuleContribution, ModuleError, ModulePosture, Mounted, SettingValue,
};

/// Optional tool backed by the provider-selected image route.
#[derive(Debug, Default)]
pub struct ImageGenerationModule;

impl Module for ImageGenerationModule {
    fn id(&self) -> &str {
        "image-generation"
    }

    fn revision(&self) -> &str {
        "smith-image-generation-v1"
    }

    fn description(&self) -> &str {
        "Generate and edit images through the active provider"
    }

    fn default_enabled(&self) -> bool {
        true
    }

    fn mount(&self, context: &ModuleContext) -> Result<Mounted, ModuleError> {
        let reason = if !context.built_in_tools {
            Some("built-in tools are disabled")
        } else if context.posture == ModulePosture::ReadOnly {
            Some("image generation is unavailable in read-only posture")
        } else if context.image_binding.is_none() {
            Some("the active provider has no image binding")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Ok(Mounted::Inactive {
                reason: reason.into(),
            });
        }
        let binding = context
            .image_binding
            .as_ref()
            .expect("checked image binding");
        let history = context.session_history.clone().ok_or_else(|| {
            ModuleError("image generation requires canonical session history".into())
        })?;
        let backend = Arc::new(image_api::ImagesApiBackend::new(
            binding.endpoint.clone(),
            context.transport.clone(),
            binding.target.clone(),
            binding.credentials.clone(),
            binding.chatgpt,
        ));
        let tool = smith_tools::GenerateImageTool::new(
            backend,
            Arc::new(image_history::ConversationImages(history)),
            context.user_dir.join("generated_images"),
            text_setting(context, "model", "gpt-image-2")?,
            text_setting(context, "quality", "auto")?,
            text_setting(context, "size", "auto")?,
        );
        Ok(Mounted::Contributions(vec![ModuleContribution::Tool(
            Arc::new(tool),
        )]))
    }
}

fn text_setting(context: &ModuleContext, name: &str, default: &str) -> Result<String, ModuleError> {
    match context.settings.get(name) {
        Some(SettingValue::String(value)) => Ok(value.clone()),
        None => Ok(default.into()),
        Some(_) => Err(ModuleError(format!(
            "image-generation setting `{name}` must be text"
        ))),
    }
}

#[cfg(test)]
mod tests;
