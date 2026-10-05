use super::*;

#[tokio::test]
async fn persistent_sessions_use_incremental_recoverable_semantic_summaries_with_disjoint_usage() {
    let fixture = Fixture::new();
    let main = |index: usize| {
        ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta {
                text: format!("answer {index}"),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])
    };
    let summary = |text: &str, input: u64, output: u64| {
        ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta { text: text.into() },
            usage_event(input, output),
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])
    };
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            main(0),
            main(1),
            main(2),
            main(3),
            main(4),
            main(5),
            summary("SUMMARY_ONE", 40, 5),
            main(6),
            summary("SUMMARY_TWO", 50, 6),
        ],
    ));
    let mut request = fixture.request(HostSurface::Headless);
    request.checkpoint_keys = Some(Arc::new(UnavailableCheckpointKeys));
    request.runtime.provider = Some(provider.clone());
    // Pinned for the same reason as the fallback test: the scripted sequence
    // encodes a cadence, and the scripted streams report no usage, so the
    // completed-turn floor is what decides here.
    let mut summary_config = smith_runtime::summary::SmithSemanticSummaryConfig::standard();
    summary_config.policy.min_turns = 6;
    request.runtime.semantic_summary = Some(summary_config.clone());
    let host = start(request).await.expect("a hosted session");
    let summary_policy = host
        .runtime()
        .policy()
        .semantic_summary
        .as_ref()
        .expect("persistent hosts enable the standard summary policy");
    assert_eq!(summary_policy.purpose, "context.semantic_summary");
    assert_eq!(summary_policy.min_turns, 6);
    assert_eq!(summary_policy.trigger_percent, 85);
    assert_eq!(summary_policy.retain_turns, 2);
    // Resolved from the model's declared input ceiling rather than guessed, so
    // the pressure comparison has something authoritative to measure against.
    assert_eq!(summary_policy.input_budget_tokens, 124_000);

    for index in 0..7 {
        host.session()
            .run(UserInput::text(format!("request {index}")))
            .await
            .unwrap_or_else(|error| panic!("turn {index} failed: {error}"));
    }

    let requests = provider.requests();
    assert_eq!(requests.len(), 9);
    let first_summary = &requests[6];
    assert!(first_summary.tools.is_empty());
    assert_eq!(
        first_summary.tool_choice,
        agent_runtime_core::provider::ToolChoice::None
    );
    assert_eq!(first_summary.max_output_tokens, Some(2_048));
    let first_summary_wire =
        serde_json::to_string(&first_summary.messages).expect("first summary request");
    assert!(first_summary_wire.contains("context.semantic_summary"));
    assert!(first_summary_wire.contains("request 0"));
    assert!(first_summary_wire.contains("answer 3"));
    assert!(
        !first_summary_wire.contains("request 4"),
        "the retained exact suffix must stay out of summary input"
    );

    let projected_wire =
        serde_json::to_string(&requests[7].messages).expect("projected main request");
    assert!(projected_wire.contains("SUMMARY_ONE"));
    assert!(projected_wire.contains("request 4"));
    assert!(projected_wire.contains("answer 5"));
    assert!(projected_wire.contains("request 6"));
    assert!(
        !projected_wire.contains("request 0"),
        "the covered prefix must not coexist with its summary"
    );

    let second_summary_wire =
        serde_json::to_string(&requests[8].messages).expect("second summary request");
    assert_eq!(
        requests[8].messages.len(),
        4,
        "the repeated summary request must contain only its adapter system message, prior protected summary, and one new canonical exchange"
    );
    assert!(second_summary_wire.contains("Previously committed semantic summary"));
    assert!(second_summary_wire.contains("SUMMARY_ONE"));
    assert!(
        !second_summary_wire.contains("request 0"),
        "a repeated summary must not reread the original canonical prefix"
    );
    assert!(
        !second_summary_wire.contains("answer 3"),
        "a repeated summary must not resend already covered canonical turns"
    );
    assert!(second_summary_wire.contains("answer 4"));
    assert!(
        second_summary_wire.contains("request 4"),
        "a repeated summary must include the newly committed canonical delta"
    );

    let snapshot = host.session().snapshot();
    let state = snapshot
        .extension_state
        .get("harness.semantic_summary")
        .expect("protected summary state");
    assert_eq!(state.sensitivity, SessionStateSensitivity::Sensitive);
    assert_eq!(state.value["purpose"], "context.semantic_summary");
    assert_eq!(state.value["summary"], "SUMMARY_TWO");
    assert_eq!(state.value["omit_prefix"], 10);
    assert_eq!(
        snapshot
            .usage
            .records()
            .iter()
            .filter(|record| record.source == UsageSource::SemanticSummary)
            .count(),
        2
    );
    assert_eq!(
        snapshot
            .usage
            .total_for(UsageSource::SemanticSummary)
            .get(CounterKind::InputUncached),
        90
    );
    assert_eq!(
        snapshot
            .usage
            .total_for(UsageSource::SemanticSummary)
            .get(CounterKind::Output),
        11
    );
    assert_eq!(snapshot.manifests.len(), 7);
    assert_eq!(snapshot.manifests[6].manifest.summaries.len(), 1);
    assert_eq!(
        snapshot.manifests[6].manifest.summaries[0]
            .covered
            .iter()
            .map(|segment| segment.as_str())
            .collect::<Vec<_>>(),
        vec![
            "history:0",
            "history:1",
            "history:2",
            "history:3",
            "history:4",
            "history:5",
            "history:6",
            "history:7",
        ]
    );

    let source: ArtifactRef = serde_json::from_value(state.value["source_artifact"].clone())
        .expect("typed source artifact");
    assert_eq!(source.provenance.session, *host.session().id());
    assert_eq!(source.provenance.purpose, "context.semantic_summary");
    let store = host
        .runtime()
        .artifact_store()
        .expect("the protected original store");
    let mut offset = 0;
    let mut original = Vec::new();
    loop {
        let page = store
            .read(ArtifactRead {
                session: host.session().id().clone(),
                id: source.id.clone(),
                offset,
                limit: MAX_ARTIFACT_READ_BYTES,
            })
            .await
            .expect("a protected original page");
        original.extend_from_slice(&page.bytes);
        let Some(next) = page.next_offset else {
            break;
        };
        offset = next;
    }
    let original: Vec<agent_runtime_core::content::Message> =
        serde_json::from_slice(&original).expect("recoverable canonical originals");
    assert_eq!(original.len(), 10);
    assert_eq!(original[0].joined_text(), "request 0");
    assert_eq!(original[9].joined_text(), "answer 4");

    let capsule = host.resume_capsule().expect("live resume capsule");
    let capsule_summary = capsule
        .semantic_summary
        .expect("ordinary summary capsule projection");
    assert_eq!(capsule_summary.provenance.provider, "local");
    assert_eq!(
        capsule_summary.provenance.model,
        "local/example-model:semantic-summary"
    );
    assert_eq!(
        capsule_summary.provenance.revision,
        state.value["summary_revision"]
            .as_str()
            .map(RegistryRevision::new)
            .expect("summary revision")
    );
    assert_eq!(capsule_summary.provenance.usage.input_uncached, 50);
    assert_eq!(capsule_summary.provenance.usage.output, 6);
    assert!(capsule_summary.provenance.summary_artifact.is_some());
    assert!(capsule.latest_summary_state_artifact.is_some());

    let session_id = host.session().id().clone();
    let paths = host.paths().expect("persistent paths").clone();
    assert!(
        !paths.checkpoint(&session_id).unwrap().exists(),
        "fixture must exercise summary restoration without a protected checkpoint"
    );

    host.shutdown().await.expect("clean shutdown");

    let mut resume = fixture
        .request(HostSurface::Headless)
        .resume(session_id.clone());
    resume.checkpoint_keys = Some(Arc::new(UnavailableCheckpointKeys));
    // Summarization is opt-in per session, resume included: a session does
    // not inherit a second model route from the fact that it once had one.
    resume.runtime.semantic_summary = Some(summary_config);
    let resumed = start(resume)
        .await
        .expect("protected summary state restores from the capsule artifact");
    let restored = resumed.session().snapshot();
    assert_eq!(
        restored.extension_state["harness.semantic_summary"].value["summary"],
        "SUMMARY_TWO"
    );
    assert_eq!(restored.id, session_id);
    resumed.shutdown().await.expect("clean resumed shutdown");
}

#[tokio::test]
async fn custom_summary_route_is_attributed_without_replacing_parent_cache_identity() {
    let fixture = Fixture::new();
    let main = |index: usize| {
        ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta {
                text: format!("answer {index}"),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])
    };
    let summary = |text: &str, input: u64, output: u64| {
        ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta { text: text.into() },
            usage_event(input, output),
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])
    };
    let mut main_capabilities = Capabilities::basic_streaming();
    main_capabilities.cache_contract = Some(
        agent_runtime_core::provider::ProviderCacheContract::from_control(
            agent_runtime_core::provider::PromptCacheControl::Implicit,
        ),
    );
    let main_provider = Arc::new(FakeProvider::new(
        "example-model",
        main_capabilities,
        (0..7).map(main).collect(),
    ));
    let summary_provider = Arc::new(FakeProvider::new(
        "summary-model",
        Capabilities::basic_streaming(),
        vec![summary("SUMMARY_ONE", 40, 5), summary("SUMMARY_TWO", 50, 6)],
    ));
    let summary_model = smith_runtime::summary::SmithProviderSummaryModel::new(
        summary_provider.clone(),
        "summary-provider",
        agent_runtime_core::provider::ModelId::new("summary-model"),
        Arc::new(agent_runtime_core::clock::SystemClock),
        2_048,
        30_000,
    )
    .expect("custom summary adapter");

    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(main_provider.clone());
    let mut summary_config = smith_runtime::summary::SmithSemanticSummaryConfig::standard();
    summary_config.policy.min_turns = 6;
    summary_config.provider = Some("summary-provider".to_owned());
    summary_config.model = Some(Arc::new(summary_model));
    request.runtime.semantic_summary = Some(summary_config);
    let host = start(request).await.expect("a hosted session");

    for index in 0..7 {
        host.session()
            .run(UserInput::text(format!("request {index}")))
            .await
            .unwrap_or_else(|error| panic!("turn {index} failed: {error}"));
    }

    assert_eq!(main_provider.requests().len(), 7);
    assert_eq!(summary_provider.requests().len(), 2);
    assert!(summary_provider.calls().iter().all(|call| {
        call.purpose == agent_runtime_core::cache::ProviderAttemptPurpose::Ordinary
            && call.cache_identity.is_none()
    }));

    let summary_policy = host
        .runtime()
        .policy()
        .semantic_summary
        .as_ref()
        .expect("custom summary policy");
    assert_eq!(summary_policy.provider, "summary-provider");
    assert_eq!(
        summary_policy.model,
        "summary-provider/summary-model:semantic-summary"
    );

    let capsule = host.resume_capsule().expect("live resume capsule");
    let capsule_summary = capsule
        .semantic_summary
        .expect("ordinary summary capsule projection");
    assert_eq!(capsule_summary.provenance.provider, "summary-provider");
    assert_eq!(
        capsule_summary.provenance.model,
        "summary-provider/summary-model:semantic-summary"
    );

    let controller = host.cache_lifecycle().expect("cache controller");
    assert_eq!(
        controller.idle_compaction_provider.as_deref(),
        Some("summary-provider")
    );
    assert_eq!(
        controller.idle_compaction_model.as_deref(),
        Some("summary-provider/summary-model:semantic-summary")
    );
    assert!(controller.synthetic_attempts.is_empty());
    assert_eq!(host.runtime().policy().provider_name, "local");
    assert_eq!(host.runtime().policy().model.as_str(), "example-model");

    host.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_failed_semantic_summary_preserves_the_structural_history_plan() {
    let fixture = Fixture::new();
    let main = |index: usize| {
        ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta {
                text: format!("answer {index}"),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])
    };
    let failed_summary = || {
        ScriptedStream::new(vec![ProviderStreamEvent::Error {
            error: ProviderError::new(ProviderErrorKind::Server, "summary unavailable"),
        }])
    };
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            main(0),
            main(1),
            main(2),
            main(3),
            main(4),
            main(5),
            failed_summary(),
            main(6),
            failed_summary(),
        ],
    ));
    let observer = RecordingObserver::shared();
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone());
    request.runtime.observers.push(observer.clone());
    // This test scripts an exact provider sequence, so it pins the cadence it
    // depends on instead of inheriting the product default. Pressure is not
    // measurable here — the scripted streams report no usage — so the floor is
    // what decides, and it must match the script.
    let mut summary_config = smith_runtime::summary::SmithSemanticSummaryConfig::standard();
    summary_config.policy.min_turns = 6;
    request.runtime.semantic_summary = Some(summary_config);
    let host = start(request).await.expect("a hosted session");

    for index in 0..7 {
        host.session()
            .run(UserInput::text(format!("request {index}")))
            .await
            .unwrap_or_else(|error| panic!("turn {index} failed: {error}"));
    }

    let requests = provider.requests();
    assert_eq!(requests.len(), 9);
    let seventh_main = serde_json::to_string(&requests[7].messages).expect("seventh main request");
    assert!(seventh_main.contains("request 0"));
    assert!(seventh_main.contains("answer 5"));
    assert!(seventh_main.contains("request 6"));
    assert!(
        !seventh_main.contains("SUMMARY_"),
        "failed summary output must never alter the deterministic structural plan"
    );
    assert!(
        !host
            .session()
            .snapshot()
            .extension_state
            .contains_key("harness.semantic_summary")
    );
    let fallbacks = observer
        .payloads()
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                RuntimeEvent::Downgrade { capability, detail }
                    if capability == "semantic_summary"
                        && detail == "summary_model_unavailable"
            )
        })
        .count();
    assert_eq!(fallbacks, 2);

    host.shutdown().await.expect("clean shutdown");
}
