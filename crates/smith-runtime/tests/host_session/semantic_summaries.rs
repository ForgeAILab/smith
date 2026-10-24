use super::*;

fn compaction_reply(text: &str, input: u64) -> ScriptedStream {
    ScriptedStream::new(vec![
        ProviderStreamEvent::TextDelta { text: text.into() },
        usage_event(input, 5),
        ProviderStreamEvent::Finish {
            reason: FinishReason::Stop,
        },
    ])
}

#[tokio::test]
async fn manual_instructions_apply_once_and_defaults_survive_cold_resume_changes() {
    use smith_runtime::cache_controller::IdleCompactionOutcome;

    let fixture = Fixture::with_config(&format!(
        "{CONFIG}\n[context.compaction]\ninstructions = \"DEFAULT SUMMARY NOTES\"\n"
    ));
    let mut scripts = (0..4)
        .map(|index| compaction_reply(&format!("answer {index}"), 100))
        .collect::<Vec<_>>();
    scripts.extend([
        compaction_reply("FIRST SUMMARY", 100),
        compaction_reply("answer 4", 100),
        compaction_reply("SECOND SUMMARY", 100),
    ]);
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        scripts,
    ));
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone());
    let host = start(request).await.expect("host");
    for index in 0..4 {
        host.session()
            .run(UserInput::text(format!("request {index}")))
            .await
            .unwrap();
    }
    assert_eq!(
        host.compact_with_instructions(Some("ONLY RELEASE DECISIONS".into()))
            .await
            .unwrap(),
        IdleCompactionOutcome::Completed
    );
    let first = serde_json::to_string(&provider.requests()[4].messages).unwrap();
    assert!(first.contains("ONLY RELEASE DECISIONS"));
    assert!(!first.contains("DEFAULT SUMMARY NOTES"));
    host.session()
        .run(UserInput::text("request 4"))
        .await
        .unwrap();
    assert_eq!(
        host.compact().await.unwrap(),
        IdleCompactionOutcome::Completed
    );
    let second = serde_json::to_string(&provider.requests()[6].messages).unwrap();
    assert!(second.contains("DEFAULT SUMMARY NOTES"));
    assert!(!second.contains("ONLY RELEASE DECISIONS"));
    let session = host.session().id().clone();
    let original = host.session().history();
    host.shutdown().await.unwrap();
    std::fs::write(
        fixture.project.path().join(".smith/config.toml"),
        format!("{CONFIG}\n[context.compaction]\ninstructions = \"NEW SUMMARY DEFAULT\"\n"),
    )
    .unwrap();
    let mut request = fixture.request(HostSurface::Headless).resume(session);
    request.runtime.provider = Some(Arc::new(FakeProvider::text_reply("continued")));
    let resumed = start(request)
        .await
        .expect("changed optional summary instructions cannot block resume");
    assert_eq!(resumed.session().history(), original);
    resumed
        .session()
        .run(UserInput::text("continue"))
        .await
        .unwrap();
    resumed.shutdown().await.unwrap();
}

#[tokio::test]
async fn configured_compaction_model_has_its_own_route_and_usage() {
    use smith_config::resolve::{AdvisorTarget, Source, Sourced};
    use smith_runtime::cache_controller::IdleCompactionOutcome;
    use smith_runtime::factory::VisionProfileRequest;

    let fixture = Fixture::with_config(&format!(
        r#"{CONFIG}
[context.compaction]
model = "summaries/small"
instructions = "Keep exact test evidence"
[providers.summaries]
kind = "fake"
[models."summaries/small"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 1024
"#
    ));
    let main_provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        (0..4)
            .map(|index| compaction_reply(&format!("answer {index}"), 100))
            .collect(),
    ));
    let summary_provider = Arc::new(FakeProvider::new(
        "small",
        Capabilities::basic_streaming(),
        vec![compaction_reply("DEDICATED SUMMARY", 200)],
    ));
    let secondary = resolve(
        &ResolveRequest::new(fixture.project.path())
            .with_home_dir(fixture.home.path())
            .with_advisor_route(Sourced::new(
                AdvisorTarget::Model {
                    provider: "summaries".into(),
                    model: "small".into(),
                },
                Source::built_in("test compaction target"),
            )),
    )
    .unwrap();
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(main_provider.clone());
    request.runtime.compaction_profile = Some(Box::new(VisionProfileRequest {
        config: secondary.config,
        catalog_sources: Vec::new(),
        provider: Some(summary_provider.clone()),
    }));
    let host = start(request).await.unwrap();
    for index in 0..4 {
        host.session()
            .run(UserInput::text(format!("request {index}")))
            .await
            .unwrap();
    }
    assert_eq!(
        host.compact().await.unwrap(),
        IdleCompactionOutcome::Completed
    );
    assert_eq!(main_provider.requests().len(), 4);
    assert_eq!(summary_provider.requests().len(), 1);
    assert_eq!(summary_provider.requests()[0].max_output_tokens, Some(1024));
    let summary = host.resume_capsule().unwrap().semantic_summary.unwrap();
    assert_eq!(summary.provenance.provider, "summaries");
    assert!(summary.provenance.model.contains("small"));
    assert_eq!(summary.provenance.usage.input_uncached, 200);
    assert!(
        serde_json::to_string(&summary_provider.requests()[0].messages)
            .unwrap()
            .contains("Keep exact test evidence")
    );
    let session = host.session().id().clone();
    let original = host.session().history();
    host.shutdown().await.unwrap();
    let config_path = fixture.project.path().join(".smith/config.toml");
    let config = std::fs::read_to_string(&config_path)
        .unwrap()
        .replace("model = \"summaries/small\"", "");
    std::fs::write(&config_path, config).unwrap();
    let mut request = fixture.request(HostSurface::Headless).resume(session);
    request.runtime.provider = Some(Arc::new(FakeProvider::text_reply("continued")));
    let resumed = start(request)
        .await
        .expect("changing the summary model preserves canonical resume");
    assert_eq!(resumed.session().history(), original);
    resumed
        .session()
        .run(UserInput::text("continue"))
        .await
        .unwrap();
    resumed.shutdown().await.unwrap();
}

#[tokio::test]
async fn default_persisted_sessions_summarize_automatically_under_pressure() {
    let fixture = Fixture::new();
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            compaction_reply("answer 0", 100),
            compaction_reply("answer 1", 100),
            compaction_reply("answer 2", 110_000),
            compaction_reply("answer 3", 110_000),
            compaction_reply("AUTOMATIC_SUMMARY", 100),
            compaction_reply("answer 4", 100),
        ],
    ));
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone());
    let host = start(request).await.expect("default summary host");
    assert!(host.cache_lifecycle().unwrap().policy.idle_compaction);
    for index in 0..3 {
        host.session()
            .run(UserInput::text(format!("request {index}")))
            .await
            .expect("ordinary turn");
    }
    assert_eq!(provider.requests().len(), 3, "minimum turn floor holds");
    host.session()
        .run(UserInput::text("request 3"))
        .await
        .expect("pressure turn");
    let snapshot = host.session().snapshot();
    assert_eq!(
        snapshot.extension_state["harness.semantic_summary"].value["summary"],
        "AUTOMATIC_SUMMARY"
    );
    host.session()
        .run(UserInput::text("request 4"))
        .await
        .expect("follow-up turn");
    let requests = provider.requests();
    assert_eq!(requests.len(), 6);
    let wire = serde_json::to_string(&requests[5].messages).unwrap();
    assert!(wire.contains("AUTOMATIC_SUMMARY"));
    assert!(!wire.contains("request 0"));
    assert!(wire.contains("request 2"));
    host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn default_persisted_sessions_compact_once_after_inactivity() {
    use smith_runtime::cache_controller::IdleCompactionOutcome;

    let fixture = Fixture::with_config(&format!(
        "{CONFIG}\n[context.cache]\ninactivity_limit_ms = 1000\nkeepalive_margin_ms = 0\n"
    ));
    let clock = agent_runtime_testkit::ManualClock::shared(0);
    let mut scripts = (0..4)
        .map(|index| compaction_reply(&format!("answer {index}"), 100))
        .collect::<Vec<_>>();
    scripts.push(compaction_reply("IDLE_SUMMARY", 100));
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        scripts,
    ));
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.clock = Some(clock.clone());
    request.runtime.provider = Some(provider.clone());
    let host = start(request).await.expect("default idle host");
    for index in 0..4 {
        host.session()
            .run(UserInput::text(format!("request {index}")))
            .await
            .expect("ordinary turn");
    }
    assert_eq!(provider.requests().len(), 4);
    clock.advance(1000);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if host.cache_lifecycle().unwrap().idle_compaction_outcome
                == Some(IdleCompactionOutcome::Completed)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the automatic idle timer commits a summary");
    assert_eq!(provider.requests().len(), 5);
    assert_eq!(
        host.session().snapshot().extension_state["harness.semantic_summary"].value["summary"],
        "IDLE_SUMMARY"
    );
    // An explicit request reaches Runtime in the same interval, and the new
    // summary already covers this history.
    assert_eq!(
        host.compact().await.unwrap(),
        IdleCompactionOutcome::NoSummary
    );
    assert_eq!(provider.requests().len(), 5);
    let session = host.session().id().clone();
    host.shutdown().await.expect("shutdown");

    // The durable gate keeps the resumed automatic timer from spending again.
    let resumed_provider = Arc::new(FakeProvider::text_reply("unused"));
    let mut resume = fixture.request(HostSurface::Headless).resume(session);
    resume.runtime.clock = Some(clock.clone());
    resume.runtime.provider = Some(resumed_provider.clone());
    let resumed = start(resume).await.expect("resume compacted session");
    clock.advance(10_000);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        resumed_provider.requests().is_empty(),
        "the resumed idle timer stays gated"
    );
    // An explicit request may retry after a restart, but the committed
    // summary already covers this history: nothing failed and nothing is
    // replaced.
    assert_eq!(
        resumed.compact().await.unwrap(),
        IdleCompactionOutcome::NoSummary
    );
    assert!(resumed_provider.requests().is_empty());
    assert_eq!(
        resumed
            .cache_lifecycle()
            .unwrap()
            .idle_compaction_reason
            .as_deref(),
        Some("not_eligible")
    );
    let capsule = resumed.resume_capsule().expect("capsule");
    let summary = capsule.semantic_summary.expect("summary projection kept");
    assert!(summary.provenance.summary_artifact.is_some());
    resumed.shutdown().await.expect("resumed shutdown");
}

#[tokio::test]
async fn ephemeral_sessions_do_not_install_a_summary_route_or_allow_manual_compaction() {
    let fixture = Fixture::new();
    let mut config = fixture.config();
    config.persistence.enabled.value = false;
    let request = fixture.request_with_config(config, HostSurface::Headless);
    assert!(request.runtime.semantic_summary.is_none());
    let host = start(request).await.expect("ephemeral host");
    assert!(host.compact().await.is_err());
    host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn manual_compaction_bypasses_the_automatic_timer_and_persists_for_resume() {
    use smith_runtime::cache_controller::IdleCompactionOutcome;

    let fixture = Fixture::with_config(&format!(
        "{CONFIG}\n[context.cache]\nidle_compaction = false\n"
    ));
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            compaction_reply("answer 0", 100),
            compaction_reply("answer 1", 100),
            compaction_reply("answer 2", 100),
            compaction_reply("answer 3", 100),
            compaction_reply("MANUAL_SUMMARY", 100),
        ],
    ));
    let mut request = fixture.request(HostSurface::Terminal);
    request.runtime.provider = Some(provider.clone());
    let host = start(request).await.expect("manual summary host");
    assert_eq!(
        host.compact().await.unwrap(),
        IdleCompactionOutcome::NoSummary
    );
    assert!(provider.requests().is_empty());
    for index in 0..4 {
        host.session()
            .run(UserInput::text(format!("request {index}")))
            .await
            .expect("ordinary turn");
        if index == 0 {
            // Too little history is a no-op, not a failure.
            assert_eq!(
                host.compact().await.unwrap(),
                IdleCompactionOutcome::NoSummary
            );
            assert!(host.resume_capsule().unwrap().semantic_summary.is_none());
        }
    }
    assert_eq!(
        provider.requests().len(),
        4,
        "low pressure makes no automatic call"
    );
    let original = host.session().history();
    assert_eq!(
        host.compact().await.unwrap(),
        IdleCompactionOutcome::Completed
    );
    assert_eq!(provider.requests().len(), 5);
    assert_eq!(host.session().history(), original);
    assert_eq!(
        host.compact().await.unwrap(),
        IdleCompactionOutcome::NoSummary
    );
    assert_eq!(
        provider.requests().len(),
        5,
        "a repeat with nothing new to summarize sends no request"
    );
    let capsule = host.resume_capsule().expect("capsule");
    assert!(
        capsule
            .semantic_summary
            .unwrap()
            .provenance
            .summary_artifact
            .is_some()
    );
    assert!(capsule.cache.idle_compaction_attempted);
    assert_eq!(
        provider.calls()[4].purpose,
        agent_runtime_core::provider::ProviderAttemptPurpose::IdleCompaction
    );
    let session = host.session().id().clone();
    host.shutdown().await.expect("shutdown");
    assert_eq!(
        host.compact().await.unwrap(),
        IdleCompactionOutcome::Shutdown
    );
    let mut resume = fixture.request(HostSurface::Headless).resume(session);
    let resumed_provider = Arc::new(FakeProvider::text_reply("continued"));
    resume.runtime.provider = Some(resumed_provider);
    let resumed = start(resume).await.expect("resume compacted session");
    assert_eq!(
        resumed.session().snapshot().extension_state["harness.semantic_summary"].value["summary"],
        "MANUAL_SUMMARY"
    );
    // A resumed process may retry, and finds nothing new to summarize.
    assert_eq!(
        resumed.compact().await.unwrap(),
        IdleCompactionOutcome::NoSummary
    );
    assert_eq!(
        resumed.session().snapshot().extension_state["harness.semantic_summary"].value["summary"],
        "MANUAL_SUMMARY"
    );
    resumed.shutdown().await.expect("resumed shutdown");
}

#[tokio::test]
async fn failed_manual_compaction_preserves_history_and_retries_on_request() {
    use smith_runtime::cache_controller::IdleCompactionOutcome;

    let fixture = Fixture::new();
    let mut scripts = (0..4)
        .map(|index| compaction_reply(&format!("answer {index}"), 100))
        .collect::<Vec<_>>();
    scripts.push(ScriptedStream::new(vec![ProviderStreamEvent::Error {
        error: ProviderError::new(ProviderErrorKind::Server, "summary unavailable"),
    }]));
    scripts.push(compaction_reply("RETRIED_SUMMARY", 100));
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        scripts,
    ));
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone());
    let host = start(request).await.expect("host");
    for index in 0..4 {
        host.session()
            .run(UserInput::text(format!("request {index}")))
            .await
            .expect("turn");
    }
    let original = host.session().history();
    assert_eq!(host.compact().await.unwrap(), IdleCompactionOutcome::Failed);
    assert_eq!(host.session().history(), original);
    assert!(
        !host
            .session()
            .snapshot()
            .extension_state
            .contains_key("harness.semantic_summary")
    );
    // An explicit retry in the same interval reaches the summary model again
    // and the summary now commits.
    assert_eq!(
        host.compact().await.unwrap(),
        IdleCompactionOutcome::Completed
    );
    assert_eq!(provider.requests().len(), 6);
    assert_eq!(
        host.session().snapshot().extension_state["harness.semantic_summary"].value["summary"],
        "RETRIED_SUMMARY"
    );
    assert_eq!(
        host.compact().await.unwrap(),
        IdleCompactionOutcome::NoSummary
    );
    assert_eq!(provider.requests().len(), 6);
    host.shutdown().await.expect("shutdown");
}

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
