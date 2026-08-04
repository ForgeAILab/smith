// reducer behavior tests.

    #[test]
    fn goal_events_reduce_identically_without_duplicating_transcript_history() {
        let update = event(RuntimeEvent::GoalUpdated {
            cause: GoalUpdateCause::TurnCommit,
            sensitivity: PlanSensitivity::Public,
            goal: Some(goal_projection(GoalStatus::Active, Some(17))),
        });
        let mut live = app();
        let mut replayed = app();
        live.apply(&update);
        replayed.apply(&update);

        assert_eq!(live.status.goal, replayed.status.goal);
        assert_eq!(
            live.status.render_goal_footer().as_deref(),
            Some("goal active · 17/100 tok")
        );
        assert!(live.transcript.blocks().is_empty());

        let cleared = event(RuntimeEvent::GoalUpdated {
            cause: GoalUpdateCause::Cleared,
            sensitivity: PlanSensitivity::Public,
            goal: None,
        });
        live.apply(&cleared);
        assert!(live.status.goal.is_none());
    }

    #[test]
    fn streaming_text_lands_in_the_transcript_with_usage_in_the_header() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(text_delta("The retry ")));
        app.apply(&event(text_delta("policy")));
        assert_eq!(app.speculative_text(), Some("The retry policy"));
        assert!(app.transcript.is_empty());
        app.apply(&event(commit_output()));
        app.apply(&event(RuntimeEvent::Usage {
            record: UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance::default(),
                delta: UsageDelta::new().with(CounterKind::InputUncached, 12_400),
            },
        }));
        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        }));

        assert_eq!(app.transcript.len(), 1);
        assert_eq!(
            app.transcript.blocks()[0],
            Block::Assistant {
                text: "The retry policy".into(),
                open: false
            }
        );
        assert_eq!(app.turn_summary.as_deref(), Some("Worked"));
        assert!(
            !app.transcript
                .blocks()
                .iter()
                .any(|block| matches!(block, Block::Notice { source, .. } if source == "work"))
        );
        assert_eq!(app.status.context.render(), "12.4k");
        assert_eq!(app.status.activity, Activity::Idle);
    }

    #[test]
    fn an_interrupted_turn_says_so() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(text_delta("partial")));
        app.apply(&event(discard_output()));
        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Cancelled {
                reason: CancelReason::UserRequested,
            },
            visible_output: true,
        }));

        assert_eq!(app.status.activity, Activity::Idle);
        assert!(matches!(
            app.transcript.blocks().last(),
            Some(Block::Notice { .. })
        ));
    }

    #[test]
    fn a_completed_turn_uses_canonical_duration_and_clears_live_timing() {
        let mut app = app();
        app.apply(&event_at(Timestamp(1_000), RuntimeEvent::TurnStarted));
        app.turn_started_at = Instant::now().checked_sub(Duration::from_secs(65));
        assert!(
            app.turn_elapsed()
                .is_some_and(|elapsed| elapsed.as_secs() >= 65)
        );

        app.apply(&event_at(
            Timestamp(66_000),
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));

        assert!(app.turn_elapsed().is_none());
        assert_eq!(app.turn_started_timestamp, None);
        assert!(app.transcript.is_empty());
        assert_eq!(app.turn_summary.as_deref(), Some("Worked for 1m 05s"));
    }

    #[test]
    fn a_success_without_visible_text_keeps_an_honest_subsecond_notice() {
        let mut app = app();
        app.apply(&event_at(Timestamp(1_000), RuntimeEvent::TurnStarted));
        app.apply(&event_at(
            Timestamp(1_842),
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: false,
            },
        ));

        assert_eq!(app.status.activity, Activity::Idle);
        assert!(app.turn_elapsed().is_none());
        assert_eq!(app.turn_summary.as_deref(), Some("Worked for 842ms"));
        let rendered = format!("{:?}", app.transcript.blocks());
        assert!(!rendered.contains("reasoning only"), "{rendered}");
    }

    #[test]
    fn unavailable_or_backward_canonical_timing_never_fabricates_duration() {
        for (started, completed) in [
            (Timestamp::ZERO, Timestamp(5_000)),
            (Timestamp(7_000), Timestamp(6_000)),
        ] {
            let mut app = app();
            app.apply(&event_at(started, RuntimeEvent::TurnStarted));
            app.apply(&event_at(
                completed,
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ));
            assert!(app.transcript.is_empty());
            assert_eq!(app.turn_summary.as_deref(), Some("Worked"));
        }
    }

    #[test]
    fn an_error_event_becomes_a_visible_error_block() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::Error {
            error: RuntimeError::config("no provider is configured"),
        }));
        match &app.transcript.blocks()[0] {
            Block::Error { message } => assert!(message.contains("no provider")),
            other => panic!("expected an error block, got {other:?}"),
        }
    }

    #[test]
    fn a_live_sequence_gap_parks_the_envelope_for_journal_replay() {
        let mut app = app();
        let mut first = event(RuntimeEvent::TurnStarted);
        first.seq = 4;
        app.apply(&first);
        let mut later = event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        });
        later.seq = 7;
        app.apply(&later);

        // The out-of-order terminal must not fold ahead of the missing
        // events: the turn still reads as live until the host replays.
        assert_eq!(app.status.activity, Activity::Working);
        let gap = app.take_stream_gap().expect("a parked stream gap");
        assert_eq!(gap.first_missing, 5);
        assert_eq!(gap.last_missing, 6);
        assert_eq!(gap.deferred.seq, 7);
        assert!(app.take_stream_gap().is_none(), "taking the gap clears it");
    }

    #[test]
    fn journal_replay_heals_a_gap_without_an_error_row() {
        let mut app = app();
        let mut first = event(RuntimeEvent::TurnStarted);
        first.seq = 4;
        app.apply(&first);
        let mut later = event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        });
        later.seq = 6;
        app.apply(&later);
        let gap = app.take_stream_gap().expect("a parked stream gap");

        let mut missing = event(RuntimeEvent::CacheObservation {
            read_tokens: 128,
            write_tokens: 0,
        });
        missing.seq = 5;
        app.apply_recovered(&missing);
        app.apply_recovered(&gap.deferred);

        assert_eq!(app.status.activity, Activity::Idle);
        assert_eq!(app.status.cache_read, Some(128));
        assert!(
            !app.transcript
                .blocks()
                .iter()
                .any(|block| matches!(block, Block::Error { .. })),
            "a fully replayed gap must not report a skip: {:?}",
            app.transcript.blocks()
        );
    }

    #[test]
    fn a_replayed_turn_terminal_still_releases_the_queued_turn() {
        // The wedge this protects against: the live stream lost
        // `TurnCompleted`, so without journal replay the queued next turn
        // would never dispatch and the UI would sit "working" forever.
        let mut app = app();
        let mut started = turn_event("turn-1", RuntimeEvent::TurnStarted);
        started.seq = 1;
        app.apply(&started);
        app.composer.replace("queued for later");
        assert_eq!(app.on_key(key(KeyCode::Tab)), None);
        assert_eq!(app.pending_input_previews()[0].entries, ["queued for later"]);

        let mut after_gap = event(RuntimeEvent::CacheObservation {
            read_tokens: 1,
            write_tokens: 0,
        });
        after_gap.seq = 4;
        app.apply(&after_gap);
        assert!(app.take_ready_submission().is_none(), "still parked");
        let gap = app.take_stream_gap().expect("a parked stream gap");
        assert_eq!((gap.first_missing, gap.last_missing), (2, 3));

        let mut usage = event(RuntimeEvent::CacheObservation {
            read_tokens: 2,
            write_tokens: 0,
        });
        usage.seq = 2;
        app.apply_recovered(&usage);
        let mut completed = turn_event(
            "turn-1",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        );
        completed.seq = 3;
        app.apply_recovered(&completed);
        app.apply_recovered(&gap.deferred);

        assert_eq!(app.status.activity, Activity::Idle);
        let submission = app
            .take_ready_submission()
            .expect("the replayed terminal releases the queued turn");
        assert_eq!(submission.display_text(), "queued for later");
    }

    #[test]
    fn an_unhealable_gap_is_still_visible_instead_of_silently_losing_output() {
        let mut app = app();
        let mut first = event(RuntimeEvent::TurnStarted);
        first.seq = 4;
        app.apply(&first);
        let mut later = event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        });
        later.seq = 7;
        app.apply(&later);

        // The journal had nothing for 5..=6: the parked envelope goes through
        // the replay path alone and the skip is reported honestly.
        let gap = app.take_stream_gap().expect("a parked stream gap");
        app.apply_recovered(&gap.deferred);

        assert_eq!(app.status.activity, Activity::Idle);
        assert!(
            app.transcript.blocks().iter().any(|block| matches!(
                block,
                Block::Error { message } if message.contains("sequence 5 through 6")
            )),
            "{:?}",
            app.transcript.blocks()
        );
    }

    #[test]
    fn provider_phase_tracks_the_round_trip_and_clears_at_the_attempt_end() {
        let mut app = app();
        assert!(app.provider_phase().is_none());

        app.apply(&event(RuntimeEvent::ProviderAttemptStarted {
            request: RequestId::new("request-fixture"),
            attempt: AttemptId::new("attempt-fixture"),
            index: 0,
            model: "gpt-5.3".to_owned(),
        }));
        assert_eq!(
            app.provider_phase().map(|(phase, _)| phase),
            Some(ProviderPhase::Sending)
        );

        app.apply(&event(RuntimeEvent::ReasoningDelta {
            request: RequestId::new("request-fixture"),
            attempt: AttemptId::new("attempt-fixture"),
            text: "weighing options".to_owned(),
            redacted: false,
        }));
        assert_eq!(
            app.provider_phase().map(|(phase, _)| phase),
            Some(ProviderPhase::Thinking)
        );

        app.apply(&event(text_delta("the answer")));
        assert_eq!(
            app.provider_phase().map(|(phase, _)| phase),
            Some(ProviderPhase::Responding)
        );

        app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
            attempt: AttemptId::new("attempt-fixture"),
            finish: agent_runtime_core::provider::FinishReason::Stop,
            retryable: false,
        }));
        assert!(
            app.provider_phase().is_none(),
            "a finished attempt leaves no live phase to display"
        );
    }

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
            Block::Notice { source, text }
                if source == "retry" && text.contains("attempt-failed")
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
                    finish: FinishReason::Stop,
                    retryable: false,
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
                    finish: FinishReason::Error,
                    retryable: true,
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
                    finish: FinishReason::ToolCalls,
                    retryable: false,
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
                    finish: FinishReason::Stop,
                    retryable: false,
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
            replayed.apply(event);
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
        assert_eq!(live.turn_summary, replayed.turn_summary);
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
    fn unterminated_speculative_output_is_discarded_at_the_turn_boundary() {
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(text_delta("orphaned draft")));
        assert_eq!(app.speculative_attempt_count(), 1);

        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Failed,
            visible_output: false,
        }));

        assert_eq!(app.speculative_attempt_count(), 0);
        assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Notice { source, text }
                if source == "integrity" && text.contains("unterminated")
        )));
        assert!(!format!("{:?}", app.transcript.blocks()).contains("orphaned draft"));
    }

    #[test]
    fn capability_lifecycle_becomes_bounded_status_and_a_concise_notice() {
        let mut app = app();
        let snapshot = fingerprint("registry");
        let view = fingerprint("view");
        app.apply(&event(RuntimeEvent::RegistrySnapshotSealed {
            snapshot: snapshot.clone(),
            entries: 6,
        }));
        app.apply(&event(RuntimeEvent::ScopedViewDerived {
            snapshot,
            view: view.clone(),
            visible_entries: 4,
        }));
        app.apply(&event(RuntimeEvent::CapabilityRetrievalPerformed {
            resolver_revision: agent_runtime_registry::RegistryRevision::new("resolver-1"),
            index_revision: None,
            candidates: vec![
                agent_runtime_registry::RegistryId::tool("read"),
                agent_runtime_registry::RegistryId::tool("search"),
            ],
        }));
        app.apply(&event(RuntimeEvent::CapabilitiesActivated {
            epoch: 2,
            activation: vec![
                ActivatedCapability::new(
                    agent_runtime_registry::RegistryId::tool("read"),
                    agent_runtime_registry::RegistryRevision::new("read-1"),
                ),
                ActivatedCapability::new(
                    agent_runtime_registry::RegistryId::tool("search"),
                    agent_runtime_registry::RegistryRevision::new("search-1"),
                ),
            ],
        }));

        assert_eq!(
            app.status.capabilities.registry,
            Some((fingerprint("registry").as_str().to_owned(), 6))
        );
        assert_eq!(
            app.status.capabilities.view,
            Some((view.as_str().to_owned(), 4))
        );
        assert_eq!(
            app.status.capabilities.retrieval,
            Some((
                "resolver-1".to_owned(),
                vec!["tool:read".to_owned(), "tool:search".to_owned()]
            ))
        );
        assert_eq!(
            app.status.capabilities.activation,
            Some((2, vec!["tool:read".to_owned(), "tool:search".to_owned()]))
        );
        assert!(app.transcript.blocks().iter().any(|block| {
            matches!(
                block,
                Block::Notice { source, text }
                    if source == "capabilities"
                        && text == "activation epoch 2: tool:read, tool:search"
            )
        }));
    }

    #[test]
    fn public_todo_updates_remain_replaceable_and_replay_equivalent() {
        let update = event(RuntimeEvent::PlanUpdated {
            revision: 3,
            sensitivity: PlanSensitivity::Public,
            counts: BTreeMap::from([
                ("cancelled".to_owned(), 0),
                ("completed".to_owned(), 1),
                ("in_progress".to_owned(), 1),
                ("pending".to_owned(), 1),
            ]),
            items: Some(vec![
                PlanItemProjection {
                    id: "inspect".to_owned(),
                    text: "Inspect\nrelevant code".to_owned(),
                    status: PlanItemStatus::Completed,
                    reason: None,
                },
                PlanItemProjection {
                    id: "change".to_owned(),
                    text: "Implement the change".to_owned(),
                    status: PlanItemStatus::InProgress,
                    reason: None,
                },
                PlanItemProjection {
                    id: "verify".to_owned(),
                    text: "Run focused tests".to_owned(),
                    status: PlanItemStatus::Pending,
                    reason: None,
                },
            ]),
        });
        let mut live = app();
        live.apply(&event(RuntimeEvent::TurnStarted));
        live.apply(&update);
        let mut replayed = app();
        replayed.apply(&event(RuntimeEvent::TurnStarted));
        replayed.apply(&update);

        assert_eq!(live.plan, replayed.plan);
        assert_eq!(live.transcript.blocks(), replayed.transcript.blocks());
        assert!(live.work_detail_lines().is_empty());
        assert!(
            !live
                .transcript
                .blocks()
                .iter()
                .any(|block| matches!(block, Block::Notice { source, .. } if source == "plan")),
            "plan updates must replace one work row instead of appending notices"
        );
    }

    #[test]
    fn sensitive_todo_update_displays_counts_without_item_text() {
        const PROTECTED_ITEM: &str = "PROTECTED PLAN CONTENT";
        let mut app = app();
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::PlanUpdated {
            revision: 1,
            sensitivity: PlanSensitivity::Sensitive,
            counts: BTreeMap::from([
                ("cancelled".to_owned(), 1),
                ("completed".to_owned(), 0),
                ("in_progress".to_owned(), 0),
                ("pending".to_owned(), 2),
            ]),
            items: Some(vec![PlanItemProjection {
                id: "protected".to_owned(),
                text: PROTECTED_ITEM.to_owned(),
                status: PlanItemStatus::Pending,
                reason: None,
            }]),
        }));

        let plan = app.plan.as_ref().expect("latest plan");
        assert_eq!(plan.sensitivity, PlanSensitivity::Sensitive);
        assert!(
            plan.items.is_none(),
            "sensitive item text survived the reducer seam"
        );
        assert!(app.work_detail_lines().is_empty());
        assert!(!format!("{:?}", app.transcript.blocks()).contains(PROTECTED_ITEM));
    }
