use super::*;

/// A deliberately small budget that reaches compaction on the third turn.
const COMPACTION_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 1000
max_input_tokens = 1000
max_output_tokens = 100

[context]
output_reserve = 100
compaction_high_watermark_percent = 85
compaction_low_watermark_percent = 60

[approval]
mode = "allow-all"
"#;

#[tokio::test]
async fn the_shared_factory_compacts_optional_history_before_an_over_budget_turn() {
    let fixture = Fixture::new(COMPACTION_CONFIG);
    let scripts = (0..3)
        .map(|turn| {
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: format!("answer-{turn}:{}", "a".repeat(800)),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])
        })
        .collect();
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        scripts,
    ));
    let recorder = RecordingObserver::shared();
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        observers: vec![recorder.clone() as Arc<dyn EventObserver>],
        system_prompt: Some("s".to_owned()),
        built_in_tools: false,
        ..request(&fixture, HostSurface::Child)
    };
    let smith = factory::build_request(request).await.expect("a runtime");

    assert_eq!(smith.policy().compaction_policy.high_watermark, 765);
    assert_eq!(smith.policy().compaction_policy.low_watermark, 540);

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    for turn in 0..3 {
        session
            .run(UserInput::text(format!("turn-{turn}:{}", "u".repeat(800))))
            .await
            .expect("the turn runs");
    }

    assert_eq!(
        provider.requests().len(),
        3,
        "the third request would fail preflight if the compactor were absent"
    );
    let last_plan = recorder
        .payloads()
        .into_iter()
        .filter_map(|event| match event {
            RuntimeEvent::ContextPlanned {
                input_tokens,
                input_budget_tokens,
                totals,
                ..
            } => Some((input_tokens, input_budget_tokens, totals)),
            _ => None,
        })
        .next_back()
        .expect("a context plan");
    assert_eq!(last_plan.1, 900);
    assert!(
        last_plan.0 <= smith.policy().compaction_policy.high_watermark,
        "the compacted request used {} tokens and remained above the {}-token pressure boundary",
        last_plan.0,
        smith.policy().compaction_policy.high_watermark
    );
    let compacted = recorder
        .payloads()
        .into_iter()
        .filter_map(|event| match event {
            RuntimeEvent::ContextCompacted {
                reclaimed_tokens,
                summaries,
                ..
            } => Some((reclaimed_tokens, summaries)),
            _ => None,
        })
        .next_back()
        .expect("a structural compaction event");
    assert!(
        compacted.0 > 0,
        "structural compaction did not reclaim input tokens"
    );
    assert!(
        compacted.1.is_empty() && last_plan.2.keys().all(|kind| kind.as_str() != "summary"),
        "the deterministic structural compactor fabricated a semantic summary"
    );

    session.shutdown().await.expect("a clean shutdown");
}
