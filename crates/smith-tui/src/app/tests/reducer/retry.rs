use super::*;

#[test]
fn budget_failures_are_retained_as_transcript_notices() {
    let mut app = app();
    for (category, name) in [
        (smith_runtime::client::BudgetCategory::Input, "input"),
        (smith_runtime::client::BudgetCategory::Context, "context"),
        (smith_runtime::client::BudgetCategory::Output, "output"),
    ] {
        app.apply(&event(RuntimeEvent::BudgetFailure {
            category,
            requested_tokens: 130_000,
            limit_tokens: 124_000,
        }));
        assert!(matches!(
            app.transcript.blocks().last(),
            Some(Block::Notice { kind: NoticeKind::Budget, text })
                if text == &format!("{name} budget exceeded · 130k requested / 124k allowed")
        ));
    }
    assert_eq!(app.transcript.blocks().len(), 3);
    assert_eq!(NoticeKind::Budget.label(), "budget");
    assert_eq!(
        NoticeKind::Budget.persistence(),
        smith_client::NoticePersistence::Transcript,
    );
}

#[test]
fn rate_limit_observations_leave_the_transcript_and_status_unchanged() {
    let mut app = app();
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.status.account = Some(crate::status::AccountStatus {
        label: "keychain:smith/work".into(),
        used_percent: Some(40.0),
    });
    app.transcript.push_user("keep this conversation");
    let blocks = app.transcript.blocks().to_vec();
    let activity = app.status.activity;
    let account = app.status.account.clone();
    let mut snapshot = agent_runtime_core::provider::RateLimitSnapshot::new();
    snapshot.push(agent_runtime_core::provider::RateLimitWindow {
        used_percent: Some(82.0),
        ..agent_runtime_core::provider::RateLimitWindow::new("requests")
    });

    app.apply(&event(RuntimeEvent::RateLimitObservation {
        attempt: AttemptId::new("attempt"),
        snapshot,
    }));

    assert_eq!(app.transcript.blocks(), blocks);
    assert_eq!(app.status.activity, activity);
    assert_eq!(app.status.account, account);
}

#[test]
fn reset_live_turn_clears_progress_and_keeps_the_conversation_and_draft() {
    let mut app = app();
    app.composer.replace("keep this draft");
    app.transcript.push_user("keep this conversation");
    app.apply(&turn_event("turn-reset", RuntimeEvent::TurnStarted));
    app.apply(&turn_event(
        "turn-reset",
        tool_requested("call-reset", "shell"),
    ));
    app.apply(&turn_event(
        "turn-reset",
        RuntimeEvent::ProviderAttemptFinished {
            attempt: AttemptId::new("attempt-reset"),
            index: Some(0),
            max_attempts: Some(3),
            finish: agent_runtime_core::provider::FinishReason::Error,
            retryable: true,
            error: None,
            retry_delay_ms: Some(200),
        },
    ));
    app.apply(&turn_event(
        "turn-reset",
        RuntimeEvent::ExternalText {
            text: "committed answer".to_owned(),
        },
    ));
    let blocks = app.transcript.blocks().to_vec();
    assert!(app.provider_retry().is_some());
    assert!(app.provider_phase().is_some());

    app.reset_live_turn();

    assert_eq!(app.status.activity, Activity::Idle);
    assert!(app.active_turn().is_none());
    assert!(app.turn_elapsed().is_none());
    assert!(app.provider_phase().is_none());
    assert!(app.provider_retry().is_none());
    assert!(app.work_detail_lines().is_empty());
    assert_eq!(app.transcript.blocks(), blocks);
    assert_eq!(app.composer.text(), "keep this draft");
}

#[test]
fn a_retryable_attempt_failure_is_visible_while_retrying() {
    let mut app = app();
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
        attempt: AttemptId::new("attempt-fixture"),
        index: Some(0),
        max_attempts: Some(3),
        finish: agent_runtime_core::provider::FinishReason::Error,
        retryable: true,
        error: Some(agent_runtime_core::provider::ProviderError::new(
            agent_runtime_core::provider::ProviderErrorKind::Server,
            "upstream 503",
        )),
        retry_delay_ms: Some(200),
    }));
    let notices = app
        .transcript
        .blocks()
        .iter()
        .filter_map(|block| match block {
            Block::Notice { kind: source, text } if source.label() == "provider" => {
                Some(text.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        notices.len(),
        1,
        "a retrying failure is visible while the loop still works: {notices:?}"
    );
    assert!(notices[0].contains("upstream 503"));
    assert_eq!(notices[0], "retrying 2/3 in 200ms: Server: upstream 503");
    assert_eq!(
        app.provider_retry().map(|retry| (
            retry.next_attempt,
            retry.max_attempts,
            retry.backoff_remaining.is_some()
        )),
        Some((2, 3, true))
    );

    // A terminal attempt failure is not retrying: the turn's own error
    // event reports it, so the transcript must not duplicate it.
    app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
        attempt: AttemptId::new("attempt-fixture-2"),
        index: Some(2),
        max_attempts: Some(3),
        finish: agent_runtime_core::provider::FinishReason::Error,
        retryable: false,
        error: Some(agent_runtime_core::provider::ProviderError::new(
            agent_runtime_core::provider::ProviderErrorKind::Server,
            "terminal failure",
        )),
        retry_delay_ms: None,
    }));
    assert_eq!(
        app.transcript
            .blocks()
            .iter()
            .filter_map(|block| match block {
                Block::Notice { kind: source, text } if source.label() == "provider" =>
                    Some(text.clone()),
                _ => None,
            })
            .count(),
        1,
        "only retryable failures announce themselves"
    );
    assert!(app.provider_retry().is_none());
}

#[test]
fn a_started_retry_keeps_its_identity_until_success() {
    let mut app = app();
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
        attempt: AttemptId::new("attempt-1"),
        index: Some(0),
        max_attempts: Some(3),
        finish: agent_runtime_core::provider::FinishReason::Error,
        retryable: true,
        error: Some(agent_runtime_core::provider::ProviderError::new(
            agent_runtime_core::provider::ProviderErrorKind::Server,
            "upstream 503",
        )),
        retry_delay_ms: Some(0),
    }));
    app.apply(&event(RuntimeEvent::ProviderAttemptStarted {
        request: RequestId::new("request-1"),
        attempt: AttemptId::new("attempt-2"),
        index: 1,
        model: "gpt-5.3".to_owned(),
    }));
    let retry = app.provider_retry().expect("retry remains visible");
    assert_eq!((retry.next_attempt, retry.max_attempts), (2, 3));
    assert!(retry.backoff_remaining.is_none());
    assert_eq!(
        app.provider_phase().map(|(phase, _)| phase),
        Some(ProviderPhase::Sending)
    );

    app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
        attempt: AttemptId::new("attempt-2"),
        index: Some(1),
        max_attempts: Some(3),
        finish: agent_runtime_core::provider::FinishReason::Stop,
        retryable: false,
        error: None,
        retry_delay_ms: None,
    }));
    assert!(app.provider_retry().is_none());
}

#[test]
fn an_exhausted_retryable_attempt_has_one_attributed_final_error() {
    let mut app = app();
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
        attempt: AttemptId::new("attempt-final"),
        index: Some(2),
        max_attempts: Some(3),
        finish: agent_runtime_core::provider::FinishReason::Error,
        retryable: true,
        error: Some(agent_runtime_core::provider::ProviderError::new(
            agent_runtime_core::provider::ProviderErrorKind::Server,
            "upstream 503",
        )),
        retry_delay_ms: None,
    }));
    app.apply(&event(RuntimeEvent::LimitReached {
        limit: smith_runtime::client::LimitKind::ProviderAttempts,
    }));
    let errors = app
        .transcript
        .blocks()
        .iter()
        .filter_map(|block| match block {
            Block::Error { message } => Some(message.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        errors,
        ["provider · failed after 3/3 attempts: Server: upstream 503"]
    );
    assert!(
        app.transcript
            .blocks()
            .iter()
            .all(|block| !matches!(block, Block::Notice { text, .. } if text.contains("retrying")))
    );
}

#[test]
fn legacy_retry_events_keep_generic_wording_without_inventing_progress() {
    let mut app = app();
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
        attempt: AttemptId::new("legacy-attempt"),
        index: None,
        max_attempts: None,
        finish: agent_runtime_core::provider::FinishReason::Error,
        retryable: true,
        error: Some(agent_runtime_core::provider::ProviderError::new(
            agent_runtime_core::provider::ProviderErrorKind::Server,
            "legacy outage",
        )),
        retry_delay_ms: None,
    }));
    let text = app
        .transcript
        .blocks()
        .iter()
        .find_map(|block| match block {
            Block::Notice { kind: source, text } if source.label() == "provider" => {
                Some(text.as_str())
            }
            _ => None,
        })
        .expect("legacy provider diagnostic");
    assert_eq!(text, "attempt failed, retrying: Server: legacy outage");
    assert!(!text.contains("/"));
    assert!(!text.contains("ms"));
    assert!(app.provider_retry().is_none());
}

#[test]
fn cancellation_and_new_turn_clear_retry_progress() {
    let mut app = app();
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
        attempt: AttemptId::new("attempt-1"),
        index: Some(0),
        max_attempts: Some(3),
        finish: agent_runtime_core::provider::FinishReason::Error,
        retryable: true,
        error: None,
        retry_delay_ms: Some(200),
    }));
    assert!(app.provider_retry().is_some());
    app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
        attempt: AttemptId::new("attempt-1"),
        index: Some(0),
        max_attempts: Some(3),
        finish: agent_runtime_core::provider::FinishReason::Cancelled,
        retryable: false,
        error: None,
        retry_delay_ms: None,
    }));
    assert!(app.provider_retry().is_none());

    app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
        attempt: AttemptId::new("attempt-1"),
        index: Some(0),
        max_attempts: Some(3),
        finish: agent_runtime_core::provider::FinishReason::Error,
        retryable: true,
        error: None,
        retry_delay_ms: Some(200),
    }));
    assert!(app.provider_retry().is_some());
    app.apply(&event(RuntimeEvent::TurnStarted));
    assert!(app.provider_retry().is_none());
}
