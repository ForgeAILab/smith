use super::*;

#[tokio::test]
async fn context_contributor_passes_newest_bounded_suffix_past_256_messages() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let mut cfg = config(root.path());
    cfg.granted_capabilities = vec![grant(C::HostRead, &["session-history"], &[])];
    cfg.requested_capabilities = cfg.granted_capabilities.clone();
    let payload_bytes = cfg.limits.payload_bytes;
    let (_, contributions, health) = mount("context-tools", cfg, &ctx);

    // 260 messages with "malformed" only in the oldest slot: the newest 256
    // suffix drops the oldest 4 messages, so the guest sees 256 clean messages.
    let mut long = Vec::with_capacity(260);
    long.push(Message::user("malformed"));
    long.extend((1..260).map(|_| Message::user("turn")));
    let mut view = ContextView {
        session: SessionId::new("session"),
        turn: TurnId::new("turn"),
        history: Arc::from(long),
        activation: Fingerprint::of("activation"),
        state: None,
    };
    let patch = contributor(&contributions).contribute(&view).await.unwrap();
    assert_eq!(patch.fragments.len(), 1);
    assert!(
        format!("{:?}", patch.fragments[0].content).contains("history: 256"),
        "{:?}",
        patch.fragments[0].content
    );
    assert!(health.failure().is_none());

    // When fewer than 256 messages still exceed the byte budget, keep the
    // newest suffix that fits within payload_bytes.
    view.history = Arc::from([
        Message::user("malformed"),
        Message::user("x".repeat(payload_bytes)),
        Message::user("recent-one"),
        Message::user("recent-two"),
    ]);
    let patch = contributor(&contributions).contribute(&view).await.unwrap();
    assert_eq!(patch.fragments.len(), 1);
    assert!(
        format!("{:?}", patch.fragments[0].content).contains("history: 2"),
        "{:?}",
        patch.fragments[0].content
    );
    assert!(health.failure().is_none());

    // Cumulative multi-message overflow also keeps the longest fitting newest suffix.
    view.history = Arc::from([
        Message::user("malformed"),
        Message::user("a".repeat(payload_bytes / 2 + 1024)),
        Message::user("b".repeat(payload_bytes / 2 + 1024)),
        Message::user("recent"),
    ]);
    let patch = contributor(&contributions).contribute(&view).await.unwrap();
    assert_eq!(patch.fragments.len(), 1);
    assert!(
        format!("{:?}", patch.fragments[0].content).contains("history: 2"),
        "{:?}",
        patch.fragments[0].content
    );
    assert!(health.failure().is_none());
}

#[tokio::test]
async fn output_processor_handles_semantic_summary_long_usage_and_oversized_inputs_without_disabling()
 {
    use agent_runtime_core::usage::{
        CounterKind, Provenance, UsageDelta, UsageRecord, UsageSource,
    };

    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path(), Arc::new(ReplayTransport::single(Vec::new())));
    let cfg = config(root.path());
    let payload_bytes = cfg.limits.payload_bytes;
    let (_, contributions, health) = mount("processor", cfg, &ctx);
    let ModuleContribution::Pipeline(PipelineComponent::ToolOutputProcessor(component)) =
        &contributions[0]
    else {
        panic!("output processor");
    };
    let mut view = ToolOutputView {
        session: SessionId::new("session"),
        turn: TurnId::new("turn"),
        request: RequestId::new("request"),
        call: ToolCall {
            id: ToolCallId::new("call"),
            name: "read".into(),
            arguments: serde_json::json!({}),
        },
        state: None,
        usage: Arc::from([UsageRecord {
            source: UsageSource::SemanticSummary,
            provenance: Provenance::default(),
            delta: UsageDelta::new()
                .with(CounterKind::InputUncached, 500)
                .with(CounterKind::Output, 100),
        }]),
        now: Timestamp(100),
    };
    let original = ToolOutcome::text("original");

    // 1. SemanticSummary usage record does not crash or disable the processor.
    let processed = component.process(&view, original.clone()).await.unwrap();
    assert_eq!(
        processed.outcome.content.as_inline().unwrap()[0].as_text(),
        Some("processed")
    );
    assert!(health.failure().is_none());

    // 2. >256 usage records are capped to the newest bounded suffix and keep running.
    let many_usage: Vec<_> = (0..300)
        .map(|i| UsageRecord {
            source: if i % 10 == 0 {
                UsageSource::SemanticSummary
            } else {
                UsageSource::ProviderAttempt
            },
            provenance: Provenance::default(),
            delta: UsageDelta::new().with(CounterKind::InputUncached, 100 + i),
        })
        .collect();
    view.usage = Arc::from(many_usage);
    for _ in 0..5 {
        let processed = component.process(&view, original.clone()).await.unwrap();
        assert_eq!(
            processed.outcome.content.as_inline().unwrap()[0].as_text(),
            Some("processed")
        );
    }
    assert!(health.failure().is_none());

    // 3. Host-side oversized outcome skips safely without blaming or resetting module health.
    let huge = ToolOutcome::text("x".repeat(payload_bytes + 1));
    assert!(!health.record_failure("prior failure".into()));
    for _ in 0..5 {
        let patch = component.process(&view, huge.clone()).await.unwrap();
        assert_eq!(patch.outcome, huge);
    }
    assert!(health.failure().is_none());
    // A skipped oversized input must not reset the prior failure count either.
    assert!(!health.record_failure("second failure".into()));
    // Healthy normal calls still run and reset consecutive failures.
    let processed = component.process(&view, original.clone()).await.unwrap();
    assert_eq!(
        processed.outcome.content.as_inline().unwrap()[0].as_text(),
        Some("processed")
    );
    assert!(health.failure().is_none());
}
