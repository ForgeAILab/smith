// reducer behavior tests.

    #[test]
    fn reducer_event_v4_pre_attempt_scoping_fixture_is_frozen() {
        // This is deliberately JSON evidence rather than a current
        // `EventEnvelope` decode test. Event schema v4 streamed text without a
        // request/attempt identity, so it cannot safely reconstruct
        // speculative output after the v5 migration. A separate v5 fixture
        // exercises the current reducer contract.
        let fixture =
            include_str!("../../../tests/fixtures/reducer-events-v4-pre-attempt-scoping.json");
        let events: serde_json::Value =
            serde_json::from_str(fixture).expect("valid reducer fixture");
        let events = events.as_array().expect("an event array");

        assert_eq!(events.len(), 5);
        assert!(events.iter().all(|event| event["schema_version"] == 4));
        let delta = &events[2]["payload"];
        assert_eq!(delta["event"], "text_delta");
        assert_eq!(delta["text"], "fixture answer");
        assert!(delta.get("request").is_none());
        assert!(delta.get("attempt").is_none());
        assert!(
            serde_json::from_str::<Vec<EventEnvelope>>(fixture).is_err(),
            "v5 must reject unattributed v4 deltas instead of synthesizing attempt identity"
        );
    }

    #[test]
    fn reducer_event_v5_fixture_commits_only_the_successful_attempt() {
        let events: Vec<EventEnvelope> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/reducer-events-v5-attempt-scoped.json"
        ))
        .expect("valid v5 reducer fixture");
        let mut app = app();
        for event in &events {
            app.apply(event);
        }

        assert_eq!(app.status.activity, Activity::Idle);
        assert_eq!(app.status.context.render(), "30");
        assert_eq!(app.speculative_attempt_count(), 0);
        assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Assistant { text, open: false } if text == "fixture answer"
        )));
        assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Notice { kind: source, text }
                if source.label() == "retry" && text.contains("attempt-failed")
        )));
        assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Tool {
                call_id,
                status: ToolStatus::Ok,
                ..
            } if call_id == "call-fixture"
        )));
        assert!(
            !format!("{:?}", app.transcript.blocks()).contains("discarded prefix"),
            "failed speculative output entered the committed transcript"
        );
    }

    #[test]
    fn live_reducer_and_journal_replay_produce_equivalent_ui_state() {
        use agent_runtime_core::interaction::InteractionOutcomeKind;
        use agent_runtime_core::provider::FinishReason;

        let session = SessionId::new("replay-session");
        let ordinary_turn = TurnId::new("turn-ordinary");
        let harness_turn = TurnId::new("turn-harness");
        let ordinary_request = RequestId::new("request-ordinary");
        let ordinary_attempt = AttemptId::new("attempt-ordinary");
        let retry_request = RequestId::new("request-retry");
        let failed_attempt = AttemptId::new("attempt-failed");
        let successful_attempt = AttemptId::new("attempt-successful");
        let final_request = RequestId::new("request-final");
        let final_attempt = AttemptId::new("attempt-final");
        let edit_call = ToolCallId::new("call-approved-edit");
        let question_call = ToolCallId::new("call-question");
        let question_request = InteractionRequestId::new("interaction-direction");
        let events = vec![
            (None, RuntimeEvent::SessionStarted),
            (
                None,
                RuntimeEvent::CapabilitiesActivated {
                    epoch: 1,
                    activation: vec![
                        ActivatedCapability::new(
                            agent_runtime_registry::RegistryId::tool("read"),
                            agent_runtime_registry::RegistryRevision::new("read-1"),
                        ),
                        ActivatedCapability::new(
                            agent_runtime_registry::RegistryId::tool("edit"),
                            agent_runtime_registry::RegistryRevision::new("edit-1"),
                        ),
                    ],
                },
            ),
            (Some(ordinary_turn.clone()), RuntimeEvent::TurnStarted),
            (
                Some(ordinary_turn.clone()),
                RuntimeEvent::ProviderAttemptStarted {
                    request: ordinary_request.clone(),
                    attempt: ordinary_attempt.clone(),
                    index: 0,
                    model: "fixture-model".to_owned(),
                },
            ),
            (
                Some(ordinary_turn.clone()),
                RuntimeEvent::TextDelta {
                    request: ordinary_request.clone(),
                    attempt: ordinary_attempt.clone(),
                    text: "ordinary committed answer".to_owned(),
                },
            ),
            (
                Some(ordinary_turn.clone()),
                RuntimeEvent::ProviderAttemptOutputCommitted {
                    request: ordinary_request,
                    attempt: ordinary_attempt.clone(),
                },
            ),
            (
                Some(ordinary_turn.clone()),
                RuntimeEvent::ProviderAttemptFinished {
                    attempt: ordinary_attempt,
                    index: None,
                    max_attempts: None,
                    finish: FinishReason::Stop,
                    retryable: false,
                    error: None,
                    retry_delay_ms: None,
                },
            ),
            (
                Some(ordinary_turn),
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ),
            (Some(harness_turn.clone()), RuntimeEvent::TurnStarted),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptStarted {
                    request: retry_request.clone(),
                    attempt: failed_attempt.clone(),
                    index: 0,
                    model: "fixture-model".to_owned(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::TextDelta {
                    request: retry_request.clone(),
                    attempt: failed_attempt.clone(),
                    text: "discarded speculative prefix".to_owned(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptOutputDiscarded {
                    request: retry_request.clone(),
                    attempt: failed_attempt.clone(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptFinished {
                    attempt: failed_attempt,
                    index: None,
                    max_attempts: None,
                    finish: FinishReason::Error,
                    retryable: true,
                    error: None,
                    retry_delay_ms: None,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptStarted {
                    request: retry_request.clone(),
                    attempt: successful_attempt.clone(),
                    index: 1,
                    model: "fixture-model".to_owned(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::TextDelta {
                    request: retry_request.clone(),
                    attempt: successful_attempt.clone(),
                    text: "retry committed answer".to_owned(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptOutputCommitted {
                    request: retry_request,
                    attempt: successful_attempt.clone(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptFinished {
                    attempt: successful_attempt,
                    index: None,
                    max_attempts: None,
                    finish: FinishReason::ToolCalls,
                    retryable: false,
                    error: None,
                    retry_delay_ms: None,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ToolCallRequested {
                    call: edit_call.clone(),
                    name: "edit".to_owned(),
                    argument_keys: vec![
                        "new_string".to_owned(),
                        "old_string".to_owned(),
                        "path".to_owned(),
                    ],
                    argument_fingerprint: fingerprint("approved-edit"),
                    arguments: None,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ToolCallCompleted {
                    call: edit_call,
                    name: "edit".to_owned(),
                    is_error: false,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ToolCallRequested {
                    call: question_call.clone(),
                    name: "ask_user".to_owned(),
                    argument_keys: vec!["questions".to_owned()],
                    argument_fingerprint: fingerprint("question"),
                    arguments: None,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::InteractionRequested {
                    request: question_request.clone(),
                    call: question_call.clone(),
                    question_count: 1,
                    sensitivity: InteractionSensitivity::Sensitive,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::InteractionResolved {
                    request: question_request,
                    call: question_call.clone(),
                    outcome: InteractionOutcomeKind::Answered,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ToolCallCompleted {
                    call: question_call,
                    name: "ask_user".to_owned(),
                    is_error: false,
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::PlanUpdated {
                    revision: 2,
                    sensitivity: PlanSensitivity::Public,
                    counts: BTreeMap::from([
                        ("cancelled".to_owned(), 0),
                        ("completed".to_owned(), 1),
                        ("in_progress".to_owned(), 1),
                        ("pending".to_owned(), 0),
                    ]),
                    items: Some(vec![
                        PlanItemProjection {
                            id: "inspect".to_owned(),
                            text: "Inspect state".to_owned(),
                            status: PlanItemStatus::Completed,
                            reason: None,
                        },
                        PlanItemProjection {
                            id: "verify".to_owned(),
                            text: "Verify replay".to_owned(),
                            status: PlanItemStatus::InProgress,
                            reason: None,
                        },
                    ]),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptStarted {
                    request: final_request.clone(),
                    attempt: final_attempt.clone(),
                    index: 0,
                    model: "fixture-model".to_owned(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::TextDelta {
                    request: final_request.clone(),
                    attempt: final_attempt.clone(),
                    text: "tools and question completed".to_owned(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptOutputCommitted {
                    request: final_request,
                    attempt: final_attempt.clone(),
                },
            ),
            (
                Some(harness_turn.clone()),
                RuntimeEvent::ProviderAttemptFinished {
                    attempt: final_attempt,
                    index: None,
                    max_attempts: None,
                    finish: FinishReason::Stop,
                    retryable: false,
                    error: None,
                    retry_delay_ms: None,
                },
            ),
            (
                Some(harness_turn),
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(seq, (turn, payload))| {
            EventEnvelope::new(
                u64::try_from(seq).expect("fixture sequence"),
                EventId::new(format!("event-{seq}")),
                session.clone(),
                turn,
                Timestamp(10_000 + u64::try_from(seq).expect("fixture time") * 100),
                payload,
            )
        })
        .collect::<Vec<_>>();
        let journal_bytes = serde_json::to_vec(&events).expect("serializable journal events");
        let replayed_events: Vec<EventEnvelope> =
            serde_json::from_slice(&journal_bytes).expect("replayable journal events");

        let mut live = app();
        live.present_recovered_ephemeral_work(1, 1, 1);
        for event in &events {
            live.apply(event);
        }
        let mut replayed = app();
        replayed.present_recovered_ephemeral_work(1, 1, 1);
        for event in &replayed_events {
            replayed.apply_recovered(event);
        }

        assert_eq!(live.transcript.blocks(), replayed.transcript.blocks());
        assert_eq!(live.status.activity, replayed.status.activity);
        assert_eq!(live.status.context, replayed.status.context);
        assert_eq!(live.status.context_plan, replayed.status.context_plan);
        assert_eq!(live.status.cache_read, replayed.status.cache_read);
        assert_eq!(live.status.capabilities, replayed.status.capabilities);
        assert_eq!(live.plan, replayed.plan);
        assert_eq!(live.children, replayed.children);
        assert_eq!(live.pending_approval_count(), 0);
        assert_eq!(replayed.pending_approval_count(), 0);
        assert_eq!(live.pending_questionnaire_count(), 0);
        assert_eq!(replayed.pending_questionnaire_count(), 0);
        assert_eq!(live.speculative_attempt_count(), 0);
        assert_eq!(replayed.speculative_attempt_count(), 0);
        let rendered = format!("{:?}", live.transcript.blocks());
        assert!(rendered.contains("ordinary committed answer"));
        assert!(rendered.contains("retry committed answer"));
        assert!(rendered.contains("call-approved-edit"));
        assert!(rendered.contains("call-question"));
        assert!(rendered.contains("not restarted"));
        assert_eq!(replayed.turn_summary, None);
        assert_eq!(replayed.visible_turn_summary(), None);
        assert!(
            live.turn_summary
                .as_deref()
                .is_some_and(|summary| summary.starts_with("Worked")),
            "{:?}",
            live.turn_summary
        );
        assert!(!rendered.contains("reasoning only"));
        assert!(
            !rendered.contains("discarded speculative prefix"),
            "failed attempt output entered the committed transcript"
        );
    }

    #[test]
    fn replayed_significant_cache_notice_is_local_and_not_duplicated() {
        let turn = TurnId::new("cache-turn");
        let event = |seq: u64, payload| {
            EventEnvelope::new(
                seq,
                EventId::new(format!("cache-event-{seq}")),
                SessionId::new("cache-session"),
                Some(turn.clone()),
                Timestamp(seq.saturating_mul(60_000)),
                payload,
            )
        };
        let events = vec![
            event(
                1,
                RuntimeEvent::ProviderAttemptStarted {
                    request: RequestId::new("cache-request"),
                    attempt: AttemptId::new("cache-attempt"),
                    index: 0,
                    model: "fixture-model".to_owned(),
                },
            ),
            event(
                2,
                RuntimeEvent::Usage {
                    record: UsageRecord {
                        source: UsageSource::ProviderAttempt,
                        provenance: Provenance {
                            request: Some(RequestId::new("cache-request")),
                            attempt: Some(AttemptId::new("cache-attempt")),
                            ..Provenance::default()
                        },
                        delta: UsageDelta::new().with(CounterKind::InputUncached, 20_000),
                    },
                },
            ),
            event(
                3,
                RuntimeEvent::CacheStateChanged {
                    request: RequestId::new("cache-request"),
                    attempt: AttemptId::new("cache-attempt"),
                    cache_plan: fingerprint("cache-plan"),
                    cache_identity: None,
                    state: CacheState::MissObserved,
                    expected_read_tokens: Some(20_000),
                    observed_read_tokens: Some(0),
                    observed_write_tokens: None,
                    missed_tokens: Some(20_000),
                    confidence: EstimationConfidence::Exact,
                },
            ),
            event(
                4,
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ),
        ];

        let mut app = app();
        app.set_cache_miss_notices(true);
        app.restore_cache_events(events.clone());
        app.restore_cache_events(events);

        let notices: Vec<_> = app
            .transcript
            .blocks()
            .iter()
            .filter_map(|block| match block {
                Block::Notice { kind: source, text } if source.label() == "cache" => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            notices,
            vec!["Cache miss · re-billed 20k · expected 20k · observed 0"]
        );
        assert_eq!(app.status.session_usage().cache_miss_count, 1);
        assert_eq!(app.status.session_usage().cache_rebilled_tokens, 20_000);
    }

    // -- Delegated usage is accounted separately (usage-accounting group 7) --

    fn usage_event(delta: UsageDelta) -> RuntimeEvent {
        RuntimeEvent::Usage {
            record: UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance::default(),
                delta,
            },
        }
    }


#[path = "reducer/retry.rs"]
mod retry;
#[path = "reducer/turns.rs"]
mod turns;
#[path = "reducer/stream_gaps.rs"]
mod stream_gaps;
#[path = "reducer/projections.rs"]
mod projections;
#[path = "reducer/usage.rs"]
mod usage;
