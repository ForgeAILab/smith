//! Context-pressure pipeline components and their live client status.

mod budget_notice;

use std::sync::Arc;

use smith_module::{
    Module, ModuleContext, ModuleContribution, ModuleError, Mounted, PipelineComponent,
};

use budget_notice::{BudgetNoticeComponent, DEFAULT_NOTICE_THRESHOLD_TOKENS};

/// Warns about the semantic-summary boundary using the resolved input ceiling.
#[derive(Debug, Default)]
pub struct BudgetNoticeModule;

impl Module for BudgetNoticeModule {
    fn id(&self) -> &str {
        "budget-notice"
    }

    fn revision(&self) -> &str {
        "smith-budget-notice-v1"
    }

    fn description(&self) -> &str {
        "Warn when conversation context nears the summary boundary"
    }

    fn default_enabled(&self) -> bool {
        true
    }

    fn mount(&self, context: &ModuleContext) -> Result<Mounted, ModuleError> {
        if !context.semantic_summary_enabled {
            return Ok(Mounted::Inactive {
                reason: "semantic summaries are disabled".into(),
            });
        }
        // As in the factory port, a budget too small for the threshold omits
        // the notice rather than failing construction of the session.
        let Ok(component) = BudgetNoticeComponent::new(
            u64::from(context.max_input_tokens),
            DEFAULT_NOTICE_THRESHOLD_TOKENS,
        ) else {
            return Ok(Mounted::Inactive {
                reason: "the input budget cannot fit the notice threshold".into(),
            });
        };
        let component = Arc::new(component);
        Ok(Mounted::Contributions(vec![
            ModuleContribution::Pipeline(PipelineComponent::ContextContributor(component.clone())),
            ModuleContribution::Pipeline(PipelineComponent::TurnCommitHook(component.clone())),
            ModuleContribution::StatusItem {
                name: "budget-notice".into(),
                source: component,
            },
        ]))
    }
}

#[cfg(test)]
mod tests;
