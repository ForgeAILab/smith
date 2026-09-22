//! Result projection, serialization compatibility, and diagnostic tests.

use super::*;

/// A turn stopped by the provider-attempt budget reports the last
/// attempt's error: the limit terminal carries no cause of its own, and
/// without this the result would claim a causeless limit.
#[test]
fn an_exhausted_attempt_budget_reports_the_last_attempt_error() {
    let finish = TurnFinish::LimitReached {
        limit: LimitKind::ProviderAttempts,
    };
    assert_eq!(
        terminal_error(
            Some(&finish),
            None,
            Some("upstream 503: service unavailable".to_owned()),
        )
        .as_deref(),
        Some("upstream 503: service unavailable"),
    );
}

/// Budget limits that end attempts without errors keep the causeless
/// limit status: there is no provider error to promote.
#[test]
fn an_output_limit_keeps_its_causeless_status() {
    let finish = TurnFinish::LimitReached {
        limit: LimitKind::Output,
    };
    assert_eq!(terminal_error(Some(&finish), None, None), None);
}

/// A completed turn never reports an earlier attempt's error: a retry
/// that eventually succeeded is history, not the outcome.
#[test]
fn a_completed_turn_never_reports_a_retried_error() {
    assert_eq!(
        terminal_error(
            Some(&TurnFinish::Completed),
            None,
            Some("upstream 503: service unavailable".to_owned()),
        ),
        None,
    );
}

/// A failed turn prefers its terminal error and falls back to the last
/// attempt's own account when the stream never delivered one.
#[test]
fn a_failed_turn_prefers_its_terminal_error() {
    assert_eq!(
        terminal_error(
            Some(&TurnFinish::Failed),
            Some("terminal".to_owned()),
            Some("attempt".to_owned()),
        )
        .as_deref(),
        Some("terminal"),
    );
    assert_eq!(
        terminal_error(Some(&TurnFinish::Failed), None, Some("attempt".to_owned())).as_deref(),
        Some("attempt"),
    );
}

#[test]
fn approval_has_a_distinct_stable_exit_status() {
    let required = ApprovalRequired {
        call_id: "call-1".into(),
        tool: "edit".into(),
        argument_keys: vec!["path".into()],
        mutates: true,
        requires_authorization: true,
        permissions: vec!["fs.write".into()],
        resource: SecurityResource::filesystem("/repo", vec!["target.txt".into()]),
        authority_warnings: Vec::new(),
        deadline_at_ms: None,
        preparation_fingerprint: "0123456789abcdef0123456789abcdef".into(),
    };
    assert!(matches!(
        outcome(
            Some(&TurnFinish::Completed),
            None,
            Some(&required),
            None,
            None,
        ),
        (ResultStatus::ApprovalRequired, APPROVAL_REQUIRED_EXIT)
    ));
}

#[test]
fn returned_child_input_has_the_interaction_required_exit_status() {
    assert!(matches!(
        outcome(
            Some(&TurnFinish::NeedsInput {
                request: agent_runtime_core::ids::InteractionRequestId::new("child-question"),
            }),
            None,
            None,
            None,
            None,
        ),
        (ResultStatus::InteractionRequired, INTERACTION_REQUIRED_EXIT)
    ));
}

#[test]
fn empty_usage_is_labelled_unknown_instead_of_reported_zero() {
    assert!(matches!(
        UsageProvenance::of(&UsageDelta::new()),
        UsageProvenance::Unknown
    ));
}

fn goal_projection(status: GoalStatus) -> GoalProjection {
    GoalProjection {
        id: GoalId::new("goal-fixture"),
        generation: 4,
        objective: "Finish the fixture".into(),
        status,
        token_budget: Some(100),
        usage: GoalTokenUsage {
            charged_tokens: Some(120),
            provenance: GoalUsageProvenance::ProviderReported,
            active_elapsed_ms: 25,
        },
        created_at: Timestamp(10),
        updated_at: Timestamp(20),
        stopped_reason: None,
    }
}

#[test]
fn every_goal_terminal_maps_to_a_stable_headless_outcome() {
    for (status, expected) in [
        (GoalStatus::Active, ResultStatus::Failed),
        (GoalStatus::Paused, ResultStatus::Cancelled),
        (GoalStatus::Blocked, ResultStatus::Failed),
        (GoalStatus::UsageLimited, ResultStatus::LimitReached),
        (GoalStatus::BudgetLimited, ResultStatus::LimitReached),
        (GoalStatus::Complete, ResultStatus::Ok),
    ] {
        let goal = goal_projection(status);
        assert_eq!(
            outcome(Some(&TurnFinish::Completed), Some(&goal), None, None, None,).0,
            expected
        );
    }
}

#[test]
fn machine_lifecycle_projects_durable_child_continuation_without_content() {
    let lifecycle = LifecycleOutput {
        children: vec![ChildSessionOutput {
            child_id: "child-3".to_owned(),
            child_session_id: "child-session-3".to_owned(),
            durability: "durable",
            state: "interrupted",
            resumable: true,
            turns_used: 1,
            max_turns: None,
            tokens_used: 42,
            incompatibility: None,
        }],
        ..LifecycleOutput::default()
    };
    let value = serde_json::to_value(lifecycle).expect("machine lifecycle serializes");
    assert_eq!(value["children"][0]["child_id"], "child-3");
    assert_eq!(value["children"][0]["child_session_id"], "child-session-3");
    assert_eq!(value["children"][0]["durability"], "durable");
    assert_eq!(value["children"][0]["resumable"], true);
    // An unbounded child reports no cap at all rather than a sentinel.
    assert!(value["children"][0].get("max_turns").is_none());
    assert!(
        !value.to_string().contains("task"),
        "child task content entered the machine status projection"
    );
}

#[test]
fn a_sequence_gap_is_a_failure_instead_of_silent_stream_loss() {
    let mut last = None;
    let mut error = None;
    observe_sequence(&mut last, 4, &mut error);
    observe_sequence(&mut last, 5, &mut error);
    assert!(error.is_none());

    observe_sequence(&mut last, 8, &mut error);
    assert!(error.expect("a gap").contains("between sequence 5 and 8"));
}

#[test]
fn synthetic_usage_has_a_typed_machine_bucket() {
    let ordinary = UsageRecord {
        source: UsageSource::ProviderAttempt,
        provenance: Provenance::default(),
        delta: UsageDelta::new().with(CounterKind::InputUncached, 100),
    };
    let keepalive = UsageRecord {
        source: UsageSource::ProviderAttempt,
        provenance: Provenance {
            attempt_purpose: Some(ProviderAttemptPurpose::CacheKeepalive),
            ..Provenance::default()
        },
        delta: UsageDelta::new()
            .with(CounterKind::InputCached, 80)
            .with(CounterKind::Output, 2),
    };
    let idle = UsageRecord {
        source: UsageSource::SemanticSummary,
        provenance: Provenance {
            purpose: Some("cache_idle_compaction".to_owned()),
            attempt_purpose: Some(ProviderAttemptPurpose::IdleCompaction),
            ..Provenance::default()
        },
        delta: UsageDelta::new()
            .with(CounterKind::InputUncached, 10)
            .with(CounterKind::Output, 3),
    };

    assert!(!is_synthetic_usage(&ordinary));
    assert!(is_synthetic_usage(&keepalive));
    assert!(is_synthetic_usage(&idle));
    let projected = SyntheticUsageOutput::from_records(&[ordinary, keepalive, idle]);
    let value = serde_json::to_value(projected).expect("synthetic usage serializes");
    assert_eq!(value["total"]["input_cached"], 80);
    assert_eq!(value["total"]["input_uncached"], 10);
    assert_eq!(value["total"]["output"], 5);
    assert_eq!(value["by_purpose"]["cache_keepalive"]["input_cached"], 80);
    assert_eq!(value["by_purpose"]["cache_idle_compaction"]["output"], 3);
    assert!(!value.to_string().contains("100"));
}

#[test]
fn machine_result_v2_compatibility_fixture_is_stable() {
    let usage = UsageDelta::new()
        .with(agent_runtime_core::usage::CounterKind::InputUncached, 12)
        .with(agent_runtime_core::usage::CounterKind::Output, 2);
    let result = ResultEnvelope {
        schema_version: 2,
        kind: "result",
        status: ResultStatus::Ok,
        session_id: "session-fixture".into(),
        turn_id: "turn-fixture".into(),
        provider: "fixture-provider".into(),
        model: "fixture-model".into(),
        output: "fixture answer".into(),
        usage: UsageOutput {
            current_turn: usage.clone(),
            session: usage,
            synthetic_cache: SyntheticUsageOutput::default(),
            current_turn_provenance: UsageProvenance::ProviderReported,
            session_provenance: UsageProvenance::ProviderReported,
        },
        lifecycle: LifecycleOutput {
            attempts_committed: 1,
            attempts_discarded: 0,
            activation: Some(ActivationOutput {
                epoch: 1,
                capabilities: vec!["tool:read".into()],
            }),
            plan: None,
            children: Vec::new(),
            parent_state: None,
        },
        goal: None,
        goal_continuation_turns: None,
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: None,
        recovery: None,
        account: None,
        background_exit: None,
        reasoning: None,
        cache: None,
        resume_capsule: None,
        error: None,
    };

    let actual = serde_json::to_value(result).expect("serializable result");
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/machine-result-v2.json"
    ))
    .expect("valid fixture");
    assert_eq!(actual, expected);
}

#[test]
fn approval_required_v2_compatibility_fixture_is_stable_and_redacted() {
    let result = ResultEnvelope {
        schema_version: 2,
        kind: "result",
        status: ResultStatus::ApprovalRequired,
        session_id: "session-fixture".into(),
        turn_id: "turn-fixture".into(),
        provider: "fixture-provider".into(),
        model: "fixture-model".into(),
        output: String::new(),
        usage: UsageOutput {
            current_turn: UsageDelta::new(),
            session: UsageDelta::new(),
            synthetic_cache: SyntheticUsageOutput::default(),
            current_turn_provenance: UsageProvenance::Unknown,
            session_provenance: UsageProvenance::Unknown,
        },
        lifecycle: LifecycleOutput::default(),
        goal: None,
        goal_continuation_turns: None,
        artifacts: Vec::new(),
        approval_required: Some(ApprovalOutput {
            call_id: "call-fixture".into(),
            tool: "edit".into(),
            argument_keys: vec!["new_string".into(), "old_string".into(), "path".into()],
            mutates: true,
            requires_authorization: true,
            permissions: vec!["fs.write".into()],
            resource: SecurityResource::filesystem("/repo", vec!["src".into(), "lib.rs".into()]),
            authority_warnings: Vec::new(),
            deadline_at_ms: Some(1_750_000_000_000),
            preparation_fingerprint: "0123456789abcdef0123456789abcdef".into(),
        }),
        interaction_required: None,
        recovery: None,
        account: None,
        background_exit: None,
        reasoning: None,
        cache: None,
        resume_capsule: None,
        error: None,
    };

    let actual = serde_json::to_value(result).expect("serializable result");
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/approval-required-v2.json"
    ))
    .expect("valid fixture");
    assert_eq!(actual, expected);
    assert_eq!(
        actual["approval_required"]["requires_authorization"], true,
        "the v2 fixture records authoritative permission-bearing work"
    );
    assert!(
        !actual.to_string().contains("replacement contents"),
        "machine approval fixture exposed an argument value"
    );
}

#[test]
fn interaction_required_v2_fixture_is_stable_and_content_free() {
    let result = ResultEnvelope {
        schema_version: 2,
        kind: "result",
        status: ResultStatus::InteractionRequired,
        session_id: "session-fixture".into(),
        turn_id: "turn-fixture".into(),
        provider: "fixture-provider".into(),
        model: "fixture-model".into(),
        output: String::new(),
        usage: UsageOutput {
            current_turn: UsageDelta::new(),
            session: UsageDelta::new(),
            synthetic_cache: SyntheticUsageOutput::default(),
            current_turn_provenance: UsageProvenance::Unknown,
            session_provenance: UsageProvenance::Unknown,
        },
        lifecycle: LifecycleOutput::default(),
        goal: None,
        goal_continuation_turns: None,
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: Some(InteractionOutput {
            request_id: "interaction-fixture".into(),
            question_count: 2,
        }),
        recovery: None,
        account: None,
        background_exit: None,
        reasoning: None,
        cache: None,
        resume_capsule: None,
        error: None,
    };
    let actual = serde_json::to_value(result).expect("serializable result");
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/interaction-required-v2.json"
    ))
    .expect("valid fixture");
    assert_eq!(actual, expected);
    assert!(!actual.to_string().contains("question prompt"));
}

#[test]
fn recovery_projection_is_metadata_only_and_keeps_the_monitor_seam_explicit() {
    let interruption = EphemeralWorkInterruption::process_exit(
        [agent_runtime_core::ids::ChildId::new("child-2")],
        std::iter::empty::<String>(),
        std::iter::empty::<String>(),
    );
    let actual = serde_json::to_value(RecoveryOutput::from(&interruption)).expect("serializable");
    assert_eq!(actual["reason"], "process_exit");
    assert_eq!(actual["interrupted_children"][0], "child-2");
    assert_eq!(
        actual["interrupted_monitors"],
        serde_json::json!([]),
        "this fixture contains no running monitor marker to reconcile"
    );
}

#[test]
fn sensitive_plan_event_content_is_removed_before_machine_projection() {
    let protected_item = "PROTECTED PLAN CONTENT";
    let projected = plan_output(
        7,
        PlanSensitivity::Sensitive,
        BTreeMap::from([("pending".to_owned(), 1)]),
        Some(vec![PlanItemProjection {
            id: "protected".to_owned(),
            text: protected_item.to_owned(),
            status: smith_runtime::client::PlanItemStatus::Pending,
            reason: None,
        }]),
    );

    let serialized = serde_json::to_string(&projected).expect("machine plan projection");
    assert!(!serialized.contains(protected_item));
    assert!(!serialized.contains("\"items\""));
}

#[test]
fn text_projection_reports_lifecycle_without_exposing_todo_or_argument_content() {
    let protected_item = "PROTECTED TODO CONTENT";
    let result = ResultEnvelope {
        account: None,
        schema_version: OUTPUT_SCHEMA_VERSION,
        kind: "result",
        status: ResultStatus::Ok,
        session_id: "session-fixture".into(),
        turn_id: "turn-fixture".into(),
        provider: "fixture-provider".into(),
        model: "fixture-model".into(),
        output: "answer".into(),
        usage: UsageOutput {
            current_turn: UsageDelta::new(),
            session: UsageDelta::new(),
            synthetic_cache: SyntheticUsageOutput::default(),
            current_turn_provenance: UsageProvenance::Unknown,
            session_provenance: UsageProvenance::Unknown,
        },
        lifecycle: LifecycleOutput {
            attempts_committed: 2,
            attempts_discarded: 1,
            activation: Some(ActivationOutput {
                epoch: 3,
                capabilities: vec!["tool:read".into(), "tool:write_todos".into()],
            }),
            plan: Some(PlanOutput {
                revision: 4,
                sensitivity: PlanSensitivity::Sensitive,
                counts: BTreeMap::from([("in_progress".to_owned(), 1), ("pending".to_owned(), 2)]),
                items: Some(vec![PlanItemProjection {
                    id: "protected".to_owned(),
                    text: protected_item.to_owned(),
                    status: smith_runtime::client::PlanItemStatus::InProgress,
                    reason: None,
                }]),
            }),
            children: Vec::new(),
            parent_state: None,
        },
        goal: None,
        goal_continuation_turns: None,
        artifacts: vec![ArtifactRef {
            id: ArtifactId::new("artifact-fixture").expect("valid artifact id"),
            digest: ArtifactDigest::new("sha256", "ab12").expect("valid digest"),
            media_type: "text/plain".into(),
            byte_length: 262_144,
            sensitivity: ArtifactSensitivity::Sensitive,
            retention: ArtifactRetention::Session,
            provenance: ArtifactProvenance::new(
                agent_runtime_core::ids::SessionId::new("session-fixture"),
                "tool-output",
            ),
        }],
        approval_required: None,
        interaction_required: None,
        recovery: Some(RecoveryOutput {
            interrupted_turn: None,
            reason: "process_exit",
            interrupted_children: vec!["child-1".into()],
            interrupted_monitors: vec!["monitor-1".into()],
            interrupted_tasks: vec!["task-1".into()],
        }),
        background_exit: None,
        reasoning: None,
        cache: None,
        resume_capsule: None,
        error: None,
    };
    let mut stderr = Vec::new();

    write_text_projection(&mut stderr, &result).expect("text projection");

    let rendered = String::from_utf8(stderr).expect("UTF-8 projection");
    assert!(rendered.contains("2 committed · 1 discarded"));
    assert!(rendered.contains("activation epoch 3 · tool:read, tool:write_todos"));
    assert!(rendered.contains("todo plan revision 4 · in_progress=1 · pending=2"));
    assert!(rendered.contains("artifact artifact-fixture · 262144 bytes · text/plain"));
    assert!(rendered.contains("1 child(ren) interrupted · 1 monitor(s) interrupted"));
    assert!(!rendered.contains(protected_item));
}

#[test]
fn canonical_cache_fixture_matches_tui_final_stream_and_text_surfaces() {
    let turn = TurnId::new("turn-cache");
    let event = |seq: u64, payload: RuntimeEvent| {
        EventEnvelope::new(
            seq,
            EventId::new(format!("cache-event-{seq}")),
            SessionId::new("session-cache"),
            Some(turn.clone()),
            Timestamp(seq.saturating_mul(60_000)),
            payload,
        )
    };
    let observation: RuntimeEvent = serde_json::from_value(serde_json::json!({
        "event": "cache_observation",
        "request": "request-cache",
        "attempt": "attempt-cache",
        "cache_plan": "plan-cache",
        "read_tokens": 0
    }))
    .expect("cache observation fixture");
    let state: RuntimeEvent = serde_json::from_value(serde_json::json!({
        "event": "cache_state_changed",
        "request": "request-cache",
        "attempt": "attempt-cache",
        "cache_plan": "plan-cache",
        "state": "miss_observed",
        "expected_read_tokens": 20_000,
        "observed_read_tokens": 0,
        "missed_tokens": 20_000,
        "confidence": "exact"
    }))
    .expect("cache state fixture");
    assert!(matches!(
        &state,
        RuntimeEvent::CacheStateChanged {
            state: CacheState::MissObserved,
            ..
        }
    ));
    let usage = UsageDelta::new().with(CounterKind::InputUncached, 20_000);
    let events = vec![
        event(
            1,
            RuntimeEvent::ProviderAttemptStarted {
                request: RequestId::new("request-cache"),
                attempt: AttemptId::new("attempt-cache"),
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
                        request: Some(RequestId::new("request-cache")),
                        attempt: Some(AttemptId::new("attempt-cache")),
                        ..Provenance::default()
                    },
                    delta: usage.clone(),
                },
            },
        ),
        event(3, observation),
        event(4, state),
        event(
            5,
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ),
    ];

    let mut tui_status = smith_tui::status::Status::new("fixture-model", "/fixture");
    for envelope in &events {
        tui_status.record_cache_event(envelope);
    }
    let tui_summary = tui_status.cache_summary().expect("TUI cache summary");

    let mut projection = CacheProjection::default();
    projection.replay(events.clone());
    let headless_summary = projection
        .latest_completed()
        .expect("headless cache summary")
        .clone();
    assert_eq!(tui_summary, headless_summary);
    assert_eq!(headless_summary.missed_tokens, Some(20_000));
    assert_eq!(headless_summary.rebilled_tokens, 20_000);

    let result = ResultEnvelope {
        schema_version: OUTPUT_SCHEMA_VERSION,
        kind: "result",
        status: ResultStatus::Ok,
        session_id: "session-cache".to_owned(),
        turn_id: "turn-cache".to_owned(),
        provider: "fixture-provider".to_owned(),
        model: "fixture-model".to_owned(),
        output: "fixture answer".to_owned(),
        usage: UsageOutput {
            current_turn: usage.clone(),
            session: usage,
            synthetic_cache: SyntheticUsageOutput::default(),
            current_turn_provenance: UsageProvenance::ProviderReported,
            session_provenance: UsageProvenance::ProviderReported,
        },
        lifecycle: LifecycleOutput::default(),
        goal: None,
        goal_continuation_turns: None,
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: None,
        recovery: None,
        account: None,
        background_exit: None,
        reasoning: None,
        cache: Some(CacheOutput::from_summary(&headless_summary, None)),
        resume_capsule: None,
        error: None,
    };
    let final_json = serde_json::to_value(&result).expect("final JSON");
    assert_eq!(final_json["cache"]["state"], "miss_observed");
    assert_eq!(final_json["cache"]["cache_read_percent"], 0);
    assert_eq!(final_json["cache"]["missed_tokens"], 20_000);
    assert_eq!(final_json["cache"]["rebilled_tokens"], 20_000);

    let stream_lines: Vec<String> = events
        .iter()
        .map(|envelope| {
            serde_json::to_string(&StreamEnvelope {
                schema_version: OUTPUT_SCHEMA_VERSION,
                kind: "event",
                event: envelope,
            })
            .expect("stream JSON")
        })
        .collect();
    let observation_at = stream_lines
        .iter()
        .position(|line| line.contains("cache_observation"))
        .expect("observation line");
    let state_at = stream_lines
        .iter()
        .position(|line| line.contains("cache_state_changed"))
        .expect("state line");
    assert!(observation_at < state_at);
    assert!(stream_lines[state_at].contains("\"missed_tokens\":20000"));

    let mut stderr = Vec::new();
    write_text_projection(&mut stderr, &result).expect("text cache projection");
    let text = String::from_utf8(stderr).expect("UTF-8 projection");
    assert!(text.contains("prompt cache: 0% of input read from cache"));
    assert!(!text.contains("miss_observed"));
    assert!(!text.contains("identity ?"));
}

#[test]
fn text_approval_diagnostic_identifies_exact_authority_without_argument_values() {
    let diagnostic = approval_diagnostic(&ApprovalOutput {
        call_id: "call-fixture".into(),
        tool: "edit".into(),
        argument_keys: vec!["new_string".into(), "path".into()],
        mutates: true,
        requires_authorization: true,
        permissions: vec!["fs.read".into(), "fs.write".into()],
        resource: SecurityResource::filesystem("/repo", vec!["src".into(), "lib.rs".into()]),
        authority_warnings: vec!["workspace_root_mutation".into()],
        deadline_at_ms: Some(1_750_000_000_000),
        preparation_fingerprint: "0123456789abcdef0123456789abcdef".into(),
    });

    assert!(diagnostic.contains("resource `/repo/src/lib.rs`"));
    assert!(diagnostic.contains("permissions fs.read, fs.write"));
    assert!(diagnostic.contains("workspace_root_mutation"));
    assert!(diagnostic.contains(" · deadline "));
    assert!(!diagnostic.contains("1750000000000"));
    assert!(diagnostic.contains("0123456789abcdef0123456789abcdef"));
    assert!(diagnostic.contains("argument values protected"));
    assert!(!diagnostic.contains("new_string"));
}

#[test]
fn legacy_v1_result_shapes_remain_frozen_as_migration_fixtures() {
    for fixture in [
        include_str!("../../../tests/fixtures/machine-result-v1.json"),
        include_str!("../../../tests/fixtures/approval-required-v1.json"),
    ] {
        let value: serde_json::Value = serde_json::from_str(fixture).expect("valid legacy fixture");
        assert_eq!(value["schema_version"], 1);
        assert!(value.get("interaction_required").is_none());
    }
}
