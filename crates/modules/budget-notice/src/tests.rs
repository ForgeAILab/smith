use agent_runtime::harness::HarnessPipelineBuilder;
use agent_runtime_testkit::transport::ReplayTransport;
use smith_module::{ModulePosture, ModuleSettings};

use super::*;

fn context() -> ModuleContext {
    ModuleContext {
        settings: ModuleSettings::new(),
        user_dir: "user".into(),
        posture: ModulePosture::ReadWrite,
        transport: Arc::new(ReplayTransport::single(Vec::new())),
        image_binding: None,
        session_history: None,
        semantic_summary_enabled: true,
        max_input_tokens: 24_000,
        built_in_tools: true,
    }
}

#[test]
fn mounted_pipeline_keeps_the_factory_fingerprint() {
    let old = Arc::new(BudgetNoticeComponent::new(24_000, 12_000).unwrap());
    let mut baseline = HarnessPipelineBuilder::new();
    baseline
        .context_contributor(old.clone())
        .turn_commit_hook(old);
    let baseline = baseline.seal().unwrap();

    let Mounted::Contributions(contributions) = BudgetNoticeModule.mount(&context()).unwrap()
    else {
        panic!("notice should mount");
    };
    let mut mounted = HarnessPipelineBuilder::new();
    for contribution in contributions {
        match contribution {
            ModuleContribution::Pipeline(PipelineComponent::ContextContributor(component)) => {
                mounted.context_contributor(component);
            }
            ModuleContribution::Pipeline(PipelineComponent::TurnCommitHook(component)) => {
                mounted.turn_commit_hook(component);
            }
            ModuleContribution::StatusItem { name, source } => {
                assert_eq!(name, "budget-notice");
                assert!(source.current().is_none());
            }
            other => panic!("unexpected contribution: {other:?}"),
        }
    }
    assert_eq!(
        baseline.fingerprint(),
        mounted.seal().unwrap().fingerprint()
    );
}

#[test]
fn semantic_summary_and_a_large_enough_budget_gate_the_notice() {
    let mut context = context();
    context.semantic_summary_enabled = false;
    assert!(matches!(
        BudgetNoticeModule.mount(&context).unwrap(),
        Mounted::Inactive { .. }
    ));
    context.semantic_summary_enabled = true;
    context.max_input_tokens = 12_000;
    assert!(matches!(
        BudgetNoticeModule.mount(&context).unwrap(),
        Mounted::Inactive { .. }
    ));
}
