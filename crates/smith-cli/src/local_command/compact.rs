use smith_client::local_result::LocalResult;
use smith_client::message_report::MessageReport;
use smith_runtime::cache_controller::IdleCompactionOutcome;
use smith_runtime::host::HostSession;

pub(crate) async fn report(host: &HostSession, instructions: Option<String>) -> LocalResult {
    let title = "compact".to_owned();
    let report = match host.compact_with_instructions(instructions).await {
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
        Ok(IdleCompactionOutcome::Busy) => MessageReport::Empty {
            title,
            message: "Compaction is busy or was already attempted for this idle interval. Wait for active work to finish or complete another turn."
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
    };
    LocalResult::Message(Box::new(report))
}
