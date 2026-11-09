//! Local feedback reports and public filing previews, independent of the UI.
mod filing;
mod guidance;
mod store;
mod tool;
pub use filing::{FilingPreview, github_issue};
use smith_module::{
    Module, ModuleContext, ModuleContribution, ModuleError, Mounted, PipelineComponent,
};
use std::sync::Arc;
pub use store::{Evidence, FeedbackStore, Kind, Listing, Report, ReportInput, Status};

pub(crate) const GUIDANCE: &str = "Report a problem with Smith itself, not with the user's project. Use harness_feedback when: a tool or Smith feature failed or returned something wrong and you resolved or abandoned it; a missing capability blocked a reasonable request; the user expressed frustration with Smith; or Smith's behavior pushed you into a mistake. Do not report bugs in the user's code or task difficulty Smith did not cause. File at most one report per distinct issue. Do not announce the report or ask the user about it; continue the task.";
/// Contributes feedback only while the module switch is enabled.
#[derive(Debug, Default)]
pub struct HarnessFeedbackModule;
impl Module for HarnessFeedbackModule {
    fn id(&self) -> &str {
        "harness-feedback"
    }
    fn revision(&self) -> &str {
        "smith-harness-feedback-v1"
    }
    fn description(&self) -> &str {
        "Record local Smith feedback"
    }
    fn default_enabled(&self) -> bool {
        true
    }
    fn mount(&self, context: &ModuleContext) -> Result<Mounted, ModuleError> {
        Ok(Mounted::Contributions(vec![
            ModuleContribution::Tool(Arc::new(tool::FeedbackTool {
                context: context.clone(),
                store: FeedbackStore::open(&context.user_dir),
            })),
            ModuleContribution::Pipeline(PipelineComponent::ContextContributor(Arc::new(
                guidance::FeedbackGuidance,
            ))),
        ]))
    }
}
#[cfg(test)]
mod tests;
