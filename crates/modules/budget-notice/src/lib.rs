//! Context-pressure pipeline components and their live client status.

mod budget_notice;

use std::sync::Arc;

use smith_module::{
    Module, ModuleContext, ModuleContribution, ModuleError, Mounted, PipelineComponent,
};

use budget_notice::{BudgetNoticeComponent, DEFAULT_NOTICE_THRESHOLD_TOKENS};

/// Warns when conversation context nears the resolved input budget boundary.
#[derive(Debug, Default)]
pub struct BudgetNoticeModule;

/// Scales the default notice threshold so smaller context windows still arm safely.
pub(crate) const fn effective_threshold_tokens(input_budget_tokens: u64) -> u64 {
    let scaled = input_budget_tokens / 5;
    let floor = if scaled > 1 { scaled } else { 1 };
    if DEFAULT_NOTICE_THRESHOLD_TOKENS < floor {
        DEFAULT_NOTICE_THRESHOLD_TOKENS
    } else {
        floor
    }
}

impl Module for BudgetNoticeModule {
    fn id(&self) -> &str {
        "budget-notice"
    }

    fn revision(&self) -> &str {
        "smith-budget-notice-v1"
    }

    fn description(&self) -> &str {
        "Warn when conversation context nears the input budget boundary"
    }

    fn default_enabled(&self) -> bool {
        true
    }

    fn mount(&self, context: &ModuleContext) -> Result<Mounted, ModuleError> {
        let input_budget_tokens = u64::from(context.max_input_tokens);
        let threshold_tokens = effective_threshold_tokens(input_budget_tokens);
        // A zero or degenerate budget omits the notice rather than failing
        // construction of the session.
        let Ok(component) = BudgetNoticeComponent::new(input_budget_tokens, threshold_tokens)
        else {
            return Ok(Mounted::Inactive {
                reason: "the input budget cannot fit the notice threshold".into(),
                setup_hint: None,
            });
        };
        let component =
            Arc::new(component.with_semantic_summary_enabled(context.semantic_summary_enabled));
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
