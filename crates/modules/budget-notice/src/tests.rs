use agent_runtime::harness::HarnessPipelineBuilder;
use agent_runtime_testkit::transport::ReplayTransport;
use smith_module::{ModulePosture, ModuleSettings};

use super::*;

fn context() -> ModuleContext {
    ModuleContext {
        session_identity: Default::default(),
        feedback_redactor: None,
        recent_failures: None,
        is_child_session: false,
        web: None,
        http: None,
        settings: ModuleSettings::new(),
        workspace_root: None,
        user_dir: "user".into(),
        posture: ModulePosture::ReadWrite,
        transport: Arc::new(ReplayTransport::single(Vec::new())),
        vision: None,
        vision_unset: true,
        image_capable_profiles: Vec::new(),
        main_model_accepts_images: false,
        model: None,
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
fn zero_or_unusable_budget_gates_the_notice() {
    let mut context = context();
    context.max_input_tokens = 0;
    assert!(matches!(
        BudgetNoticeModule.mount(&context).unwrap(),
        Mounted::Inactive { .. }
    ));
    context.max_input_tokens = 1;
    assert!(matches!(
        BudgetNoticeModule.mount(&context).unwrap(),
        Mounted::Inactive { .. }
    ));
}

#[tokio::test]
async fn activates_when_semantic_summary_is_disabled_and_scales_threshold_for_small_windows() {
    use agent_runtime::context::FragmentContent;
    use agent_runtime::harness::{ContextContributor, ContextView, TurnCommitHook, TurnCommitView};
    use agent_runtime_core::clock::Timestamp;
    use agent_runtime_core::event::TurnFinish;
    use agent_runtime_core::ids::{SessionId, TurnId};
    use agent_runtime_core::store::VersionedSessionState;
    use agent_runtime_core::usage::{
        CounterKind, Provenance, UsageDelta, UsageRecord, UsageSource,
    };

    let mut ctx = context();
    ctx.semantic_summary_enabled = false;
    ctx.max_input_tokens = 8_000;

    let Mounted::Contributions(contributions) = BudgetNoticeModule.mount(&ctx).unwrap() else {
        panic!("budget notice should mount with semantic summaries disabled on an 8K window");
    };
    let mut contributor: Option<Arc<dyn ContextContributor>> = None;
    let mut hook: Option<Arc<dyn TurnCommitHook>> = None;
    for contribution in contributions {
        match contribution {
            ModuleContribution::Pipeline(PipelineComponent::ContextContributor(c)) => {
                contributor = Some(c);
            }
            ModuleContribution::Pipeline(PipelineComponent::TurnCommitHook(h)) => {
                hook = Some(h);
            }
            ModuleContribution::StatusItem { .. } => {}
            other => panic!("unexpected contribution: {other:?}"),
        }
    }
    let contributor = contributor.expect("context contributor");
    let hook = hook.expect("turn commit hook");

    let run_step = |input_tokens: u64| {
        let contributor = contributor.clone();
        let hook = hook.clone();
        async move {
            let commit = TurnCommitView {
                session: SessionId::new("s"),
                turn: TurnId::new("t"),
                finish: TurnFinish::Completed,
                provider_error_kind: None,
                visible_output: false,
                history: Arc::from([]),
                state: None,
                usage: Arc::from([UsageRecord {
                    source: UsageSource::ProviderAttempt,
                    provenance: Provenance::default(),
                    delta: UsageDelta::new().with(CounterKind::InputUncached, input_tokens),
                }]),
                started_at: Timestamp(0),
                committed_at: Timestamp(1),
            };
            let state_patch = hook
                .before_provider(&commit)
                .await
                .unwrap()
                .patch
                .state
                .unwrap();
            let view = ContextView {
                session: SessionId::new("s"),
                turn: TurnId::new("t"),
                history: Arc::from([]),
                activation: agent_runtime::registry::Fingerprint::of_fields([b"a".as_slice()]),
                state: Some(VersionedSessionState {
                    revision: state_patch.revision,
                    sensitivity: state_patch.sensitivity,
                    value: state_patch.value,
                }),
            };
            contributor.contribute(&view).await.unwrap()
        }
    };

    // On an 8,000-token window, effective threshold is 8,000 / 5 = 1,600.
    // 1,601 remaining is quiet; 1,600 remaining fires.
    assert!(run_step(6_399).await.fragments.is_empty());
    let armed = run_step(6_400).await;
    let [fragment] = armed.fragments.as_slice() else {
        panic!("expected notice at scaled threshold: {armed:?}");
    };
    match &fragment.content {
        FragmentContent::Text(body) => {
            assert!(body.contains("1600"), "{body}");
            assert!(
                body.contains("limit") || body.contains("compacted"),
                "{body}"
            );
        }
        other => panic!("expected text fragment: {other:?}"),
    }

    // Verify threshold scaling across common window sizes.
    for (budget, expected_threshold) in [
        (256_000u64, 12_000u64),
        (64_000, 12_000),
        (32_000, 6_400),
        (8_000, 1_600),
        (4, 1),
    ] {
        assert_eq!(effective_threshold_tokens(budget), expected_threshold);
    }
}
