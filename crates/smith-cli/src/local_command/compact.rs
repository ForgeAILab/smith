use smith_client::local_result::LocalResult;
use smith_client::message_report::MessageReport;
use smith_runtime::cache_controller::IdleCompactionOutcome;
use smith_runtime::host::HostSession;

pub(crate) async fn report(host: &HostSession, instructions: Option<String>) -> LocalResult {
    let outcome = host.compact_with_instructions(instructions).await;
    let earlier = host
        .cache_lifecycle()
        .and_then(|controller| controller.idle_compaction_outcome);
    LocalResult::Message(Box::new(outcome_report(outcome, earlier)))
}

/// `earlier` is the last recorded attempt's outcome, which an
/// `AlreadyAttempted` refusal describes.
pub(crate) fn outcome_report(
    outcome: Result<IdleCompactionOutcome, agent_runtime_core::error::RuntimeError>,
    earlier: Option<IdleCompactionOutcome>,
) -> MessageReport {
    let title = "compact".to_owned();
    match outcome {
        Ok(IdleCompactionOutcome::Completed) => MessageReport::Notice {
            title,
            message: "Context compacted. Older history is summarized; recent turns are retained."
                .to_owned(),
        },
        Ok(IdleCompactionOutcome::NoSummary) => MessageReport::Empty {
            title,
            message: "No eligible history to compact yet. Continue the conversation first."
                .to_owned(),
        },
        Ok(IdleCompactionOutcome::AlreadyAttempted) => MessageReport::Empty {
            title,
            message: format!(
                "Compaction already ran after the last turn: {}. Each idle interval allows one attempt; send another message, then run /compact again.",
                earlier_attempt(earlier)
            ),
        },
        Ok(IdleCompactionOutcome::NotIdle) => MessageReport::Empty {
            title,
            message: "A turn is still running, so nothing was compacted. Run /compact again after it finishes."
                .to_owned(),
        },
        Ok(IdleCompactionOutcome::Busy) => MessageReport::Empty {
            title,
            message: "The session was busy with a turn, recovery, or pending request, so nothing was compacted. Run /compact again once it is idle."
                .to_owned(),
        },
        Ok(IdleCompactionOutcome::RequestPending) => MessageReport::Empty {
            title,
            message: "Another compaction request is still waiting to run. Wait for its result before running /compact again."
                .to_owned(),
        },
        Ok(IdleCompactionOutcome::EventGap) => MessageReport::Error {
            title,
            message: "Compaction unavailable: Smith missed session events and cannot confirm a safe boundary. Restart Smith and resume this session to compact it."
                .to_owned(),
        },
        Ok(IdleCompactionOutcome::Shutdown) => MessageReport::Error {
            title,
            message: "Compaction unavailable: the session is shutting down.".to_owned(),
        },
        Ok(IdleCompactionOutcome::Failed) => MessageReport::Error {
            title,
            message: "Compaction failed. Original history and the previous summary are preserved. See /status --verbose for details."
                .to_owned(),
        },
        Err(error) => MessageReport::Error {
            title,
            message: error.to_string(),
        },
    }
}

fn earlier_attempt(outcome: Option<IdleCompactionOutcome>) -> &'static str {
    match outcome {
        Some(IdleCompactionOutcome::Completed) => "context was compacted",
        Some(IdleCompactionOutcome::NoSummary) => "it found no eligible history",
        Some(IdleCompactionOutcome::Failed) => "it failed (see /status --verbose)",
        Some(IdleCompactionOutcome::Shutdown) => "it was interrupted",
        _ => "its result was not recorded",
    }
}
