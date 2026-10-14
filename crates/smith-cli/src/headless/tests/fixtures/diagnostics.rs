use super::*;
use agent_runtime_core::error::RuntimeError;

#[test]
fn completed_turn_retains_runtime_diagnostics_without_a_terminal_error() {
    let mut fold = crate::headless::fold::HeadlessFold::new(
        TurnId::new("turn-fixture"),
        CacheProjection::default(),
        None,
        0,
    );
    let diagnostic = RuntimeError::config("Claude Code did not load Smith's tools for this turn");
    let message = diagnostic.to_string();
    for (seq, payload) in [
        RuntimeEvent::Error { error: diagnostic },
        RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        },
    ]
    .into_iter()
    .enumerate()
    {
        fold.apply(&EventEnvelope::new(
            u64::try_from(seq).expect("sequence"),
            EventId::new(format!("event-{seq}")),
            SessionId::new("session-fixture"),
            Some(TurnId::new("turn-fixture")),
            Timestamp::ZERO,
            payload,
        ));
    }
    let mut result = fixture_result();
    result.error = terminal_error(
        fold.finish.as_ref(),
        fold.last_error,
        fold.last_attempt_error,
    );
    assert_eq!(
        outcome(
            fold.finish.as_ref(),
            None,
            None,
            None,
            result.error.as_ref()
        ),
        (ResultStatus::Ok, 0)
    );
    assert_eq!(fold.diagnostics, [message]);
    result.diagnostics = fold.diagnostics;
    record_projection("completed-diagnostic", &result, &[]);
}

#[test]
fn failed_turn_keeps_the_existing_terminal_error() {
    let error = Some("bridge turn failed".to_owned());
    assert_eq!(
        terminal_error(Some(&TurnFinish::Failed), error.clone(), None),
        error
    );
    assert_eq!(
        outcome(Some(&TurnFinish::Failed), None, None, None, error.as_ref()),
        (ResultStatus::Failed, 1)
    );
}
