//! Feedback guidance exists only while the module is mounted.

use agent_runtime::context::{ContextFragment, FragmentContent, FragmentKind, FragmentSource};
use agent_runtime::harness::{ComponentDescriptor, ContextContributor, ContextPatch, ContextView};
use agent_runtime::registry::RegistryRevision;
use agent_runtime_core::error::RuntimeError;
use async_trait::async_trait;

#[derive(Debug)]
pub(crate) struct FeedbackGuidance;

#[async_trait]
impl ContextContributor for FeedbackGuidance {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(
            "smith.harness-feedback-guidance",
            RegistryRevision::new("smith-harness-feedback-v1"),
        )
    }

    async fn contribute(&self, _: &ContextView) -> Result<ContextPatch, RuntimeError> {
        Ok(ContextPatch::new(vec![ContextFragment::new(
            "smith.harness-feedback-guidance",
            FragmentKind::DeveloperInstruction,
            FragmentSource::Host,
            RegistryRevision::new("smith-harness-feedback-v1"),
            FragmentContent::Text(crate::GUIDANCE.into()),
        )]))
    }
}
