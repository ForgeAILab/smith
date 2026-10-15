//! Stable image-placeholder guidance contributed only by the mounted module.

use agent_runtime::context::{ContextFragment, FragmentContent, FragmentKind, FragmentSource};
use agent_runtime::harness::{ComponentDescriptor, ContextContributor, ContextPatch, ContextView};
use agent_runtime::registry::RegistryRevision;
use agent_runtime_core::error::RuntimeError;
use async_trait::async_trait;

#[derive(Debug)]
pub(crate) struct VisionGuidance;

#[async_trait]
impl ContextContributor for VisionGuidance {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(
            "smith.vision-guidance",
            RegistryRevision::new("smith-vision-v1"),
        )
    }

    async fn contribute(&self, _: &ContextView) -> Result<ContextPatch, RuntimeError> {
        Ok(ContextPatch::new(vec![ContextFragment::new(
            "smith.vision-guidance",
            FragmentKind::DeveloperInstruction,
            FragmentSource::Host,
            RegistryRevision::new("smith-vision-v1"),
            FragmentContent::Text("Images shown as [image <id>: …] placeholders can be inspected by calling view_image with that id as `image` and a `question`.".into()),
        )]))
    }
}
