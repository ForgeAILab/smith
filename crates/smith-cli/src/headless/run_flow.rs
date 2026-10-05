//! Headless turn event consumption and terminal stream output.

use std::io::Write;
use std::time::Duration;

use agent_runtime_core::content::{Role, UserInput};
use agent_runtime_core::usage::UsageDelta;
use anyhow::{Context, Result};
use futures_util::StreamExt;
use smith_client::cache::{CacheLifecycleSummary, CacheProjection};
use smith_client::plural;
use smith_config::model::BackgroundExit;
use smith_host::{HeadlessApproval, HeadlessInteraction, InteractionRequired};
use smith_runtime::ChildState;
use smith_runtime::cache_controller::CacheControllerSnapshot;
use smith_runtime::client::{SmithEventKind as RuntimeEvent, TurnFinish};
use smith_runtime::host::HostSession;

use super::background::apply_background_exit_policy;
use super::fold::HeadlessFold;
use super::output::{
    CacheControllerEnvelope, CacheOutput, LifecycleOutput, ReasoningOutput, ResultEnvelope,
    ResultStatus, StreamEnvelope, SyntheticUsageOutput, UsageOutput, UsageProvenance,
    account_output, activation_output, approval_diagnostic, child_session_outputs, outcome,
    recovery_output, terminal_error, write_json, write_text, write_text_projection,
};
use super::{HeadlessBrokers, INTERACTION_REQUIRED_EXIT, OUTPUT_SCHEMA_VERSION, Outcome};
use crate::cli::OutputFormat;

pub(super) async fn run_with_io(
    host: &HostSession,
    prompt: String,
    format: OutputFormat,
    brokers: HeadlessBrokers<'_>,
    background_exit: BackgroundExit,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<Outcome> {
    let HeadlessBrokers {
        approval,
        interaction,
        rotation,
        credential_pool,
        cache_price,
        cache_miss_notices,
    } = brokers;
    if let Some(restored) = host.restored_interaction() {
        let required = interaction
            .and_then(HeadlessInteraction::required)
            .filter(|required| required.request_id == restored.request_id().as_str())
            .unwrap_or_else(|| InteractionRequired {
                request_id: restored.request_id().as_str().to_owned(),
                question_count: restored.question_count(),
            });
        return write_restored_interaction_required(host, format, required, stdout, stderr).await;
    }

    let session = host.session();
    let mut events = host.client().events();
    let mut cache_projection = CacheProjection::default();
    if let Ok(history) = host.client_timeline_events().await {
        cache_projection.replay(history);
    }
    let initial_activation = activation_output(session);
    let history_start = session.history().len();
    let turn = match session.send(UserInput::text(prompt)) {
        Ok(turn) => turn,
        Err(error) => {
            return write_submission_failure(host, format, stdout, stderr, error.to_string()).await;
        }
    };
    let turn_id = turn.id().clone();
    let mut fold = HeadlessFold::new(turn_id.clone(), cache_projection, initial_activation);

    while let Some(event) = events.next().await {
        fold.apply(&event);

        if format == OutputFormat::StreamJson
            && let Err(error) = write_json(
                stdout,
                &StreamEnvelope {
                    schema_version: OUTPUT_SCHEMA_VERSION,
                    kind: "runtime_event",
                    event: &event,
                },
            )
        {
            let _ = host.shutdown().await;
            return Err(error);
        }
        if fold.exit(|| host.goal(), || has_required_child_work(host)) {
            break;
        }
    }

    let stream_error = fold
        .finish
        .is_none()
        .then(|| "the runtime event stream ended before the turn completed".to_owned());

    fold.lifecycle.children = child_session_outputs(host);
    fold.lifecycle.parent_state = host
        .delegation_parking()
        .map(|snapshot| snapshot.state.as_str());

    let final_goal = match host.goal() {
        Ok(goal) => goal,
        Err(error) => {
            fold.sequence_error
                .get_or_insert_with(|| format!("persistent goal state unavailable: {error}"));
            None
        }
    };
    let goal_continuation_turns = final_goal.as_ref().map(|_| fold.goal_continuation_turns);

    let (background_exit_error, background_exit_output) =
        apply_background_exit_policy(host.background_tasks(), session.id(), background_exit).await;

    let shutdown_error = host.shutdown().await.err().map(|error| error.to_string());

    // SessionShutdown is queued before shutdown returns. Include it in the
    // stream without risking an indefinite wait if a future runtime changes
    // that lifecycle detail.
    if format == OutputFormat::StreamJson {
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(100), events.next()).await
        {
            observe_sequence(&mut fold.last_sequence, event.seq, &mut fold.sequence_error);
            let terminal = matches!(event.payload, RuntimeEvent::SessionShutdown);
            write_json(
                stdout,
                &StreamEnvelope {
                    schema_version: OUTPUT_SCHEMA_VERSION,
                    kind: "runtime_event",
                    event: &event,
                },
            )?;
            if terminal {
                break;
            }
        }
    }

    // Host shutdown freezes and drains the controller, then retains this
    // final redaction-safe snapshot. Publish that exact terminal projection
    // in both the stream envelope and final result.
    let cache_controller = host.cache_lifecycle();
    let resume_capsule = host.resume_capsule();
    if format == OutputFormat::StreamJson
        && let Some(controller) = cache_controller.as_ref()
    {
        write_json(
            stdout,
            &CacheControllerEnvelope {
                schema_version: OUTPUT_SCHEMA_VERSION,
                kind: "cache_controller",
                controller,
            },
        )?;
    }

    let snapshot = host.snapshot();
    let artifacts = session.artifacts_for_turn(&turn_id);
    let output = snapshot
        .history
        .iter()
        .skip(history_start)
        .rev()
        .find(|message| message.role == Role::Assistant && !message.joined_text().is_empty())
        .map(|message| message.joined_text())
        .unwrap_or_default();
    let session_usage = snapshot.usage.total();
    let synthetic_cache = SyntheticUsageOutput::from_records(snapshot.usage.records());
    if let Some(summary) = fold.cache_projection.completed_turn(turn_id.as_str()) {
        let summary = cache_price
            .map(|price| fold.cache_projection.with_price(summary, price))
            .unwrap_or_else(|| summary.clone());
        fold.cache = Some(CacheOutput::from_summary(&summary, fold.cache));
    }
    let cache_lifecycle = fold.cache_projection.lifecycle().clone();
    if cache_lifecycle != CacheLifecycleSummary::default() {
        fold.cache = Some(CacheOutput::from_lifecycle(cache_lifecycle, fold.cache));
    }
    if let Some(controller) = cache_controller {
        fold.cache = Some(CacheOutput::from_controller(controller, fold.cache));
    }
    let approval_required = approval.and_then(HeadlessApproval::required);
    let interaction_required = interaction
        .and_then(HeadlessInteraction::required)
        .or(fold.event_interaction_required);
    let lifecycle_error = shutdown_error.or(stream_error).or(fold.sequence_error);
    let error = background_exit_error.or(lifecycle_error).or_else(|| {
        terminal_error(
            fold.finish.as_ref(),
            fold.last_error,
            fold.last_attempt_error,
        )
    });
    let (status, exit_code) = outcome(
        fold.finish.as_ref(),
        final_goal.as_ref(),
        approval_required.as_ref(),
        interaction_required.as_ref(),
        error.as_ref(),
    );

    let result = ResultEnvelope {
        schema_version: OUTPUT_SCHEMA_VERSION,
        kind: "result",
        status,
        session_id: session.id().as_str().to_owned(),
        turn_id: turn_id.as_str().to_owned(),
        provider: host.runtime().policy().provider_name.clone(),
        model: host.runtime().policy().model.as_str().to_owned(),
        output: output.clone(),
        usage: UsageOutput {
            current_turn_provenance: UsageProvenance::of(&fold.turn_usage),
            session_provenance: UsageProvenance::of(&session_usage),
            current_turn: fold.turn_usage,
            session: session_usage,
            synthetic_cache,
        },
        lifecycle: fold.lifecycle,
        goal: final_goal,
        goal_continuation_turns,
        artifacts,
        account: account_output(credential_pool, rotation),
        approval_required: approval_required.map(Into::into),
        interaction_required: interaction_required.map(Into::into),
        recovery: recovery_output(host),
        background_exit: background_exit_output,
        reasoning: Some(ReasoningOutput::of(&host.runtime().policy().reasoning)),
        cache: fold.cache,
        resume_capsule,
        error: error.clone(),
    };

    match format {
        OutputFormat::Text if exit_code == 0 => {
            write_text(stdout, &output)?;
            write_text_projection(stderr, &result)?;
            if cache_miss_notices
                && let Some(notice) = result
                    .cache
                    .as_ref()
                    .and_then(|cache| cache.notice.as_ref())
            {
                writeln!(stderr, "smith: {notice}").context("writing cache notice")?;
                stderr.flush().context("flushing cache notice")?;
            }
        }
        OutputFormat::Text => {
            let diagnostic = match (
                &result.approval_required,
                &result.interaction_required,
                error,
            ) {
                (Some(required), _, _) => approval_diagnostic(required),
                (_, Some(required), _) => format!(
                    "interaction required for request `{}` ({}); \
                     rerun in an interactive terminal",
                    required.request_id,
                    plural(required.question_count, "question", "questions")
                ),
                (_, _, Some(error)) => error,
                _ => format!("turn ended with status {:?}", result.status),
            };
            write_text_projection(stderr, &result)?;
            if cache_miss_notices
                && let Some(notice) = result
                    .cache
                    .as_ref()
                    .and_then(|cache| cache.notice.as_ref())
            {
                writeln!(stderr, "smith: {notice}").context("writing cache notice")?;
            }
            writeln!(stderr, "smith: {diagnostic}").context("writing diagnostic to stderr")?;
            stderr.flush().context("flushing diagnostic stderr")?;
        }
        OutputFormat::Json | OutputFormat::StreamJson => write_json(stdout, &result)?,
    }

    Ok(Outcome { exit_code })
}

/// Whether a terminal root turn may keep a headless process alive for required
/// automatic work. Only normal completion can be extended by goal or child
/// continuations; every non-success finish must proceed to cleanup and its
/// structured terminal result.
pub(super) fn finish_waits_for_required_follow_up(finish: &TurnFinish) -> bool {
    match finish {
        TurnFinish::Completed => true,
        TurnFinish::Cancelled { .. }
        | TurnFinish::LimitReached { .. }
        | TurnFinish::NeedsInput { .. }
        | TurnFinish::Failed => false,
    }
}

pub(super) fn has_required_child_work(host: &HostSession) -> bool {
    if host.delegation_parking().is_some_and(|parking| {
        parking.admission_in_flight
            || !parking.pending_children.is_empty()
            || !parking.ready_outcomes.is_empty()
    }) {
        return true;
    }
    let Some(coordinator) = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
    else {
        return false;
    };
    coordinator
        .list()
        .into_iter()
        .any(|status| matches!(status.state, ChildState::Running))
        || !coordinator.take_ready_task_outcomes().is_empty()
}

pub(super) async fn write_restored_interaction_required(
    host: &HostSession,
    format: OutputFormat,
    required: InteractionRequired,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<Outcome> {
    let restored = host
        .restored_interaction()
        .expect("restored interaction checked before rendering");
    let session = host.session();
    let session_id = session.id().as_str().to_owned();
    let turn_id = restored.turn_id().as_str().to_owned();
    let final_goal = host.goal().ok().flatten();
    let goal_continuation_turns = final_goal.as_ref().map(|_| 0);
    let (shutdown_error, cache_controller) =
        shutdown_and_write_stream_tail(host, format, stdout).await?;
    let snapshot = host.snapshot();
    let session_usage = snapshot.usage.total();
    let synthetic_cache = SyntheticUsageOutput::from_records(snapshot.usage.records());
    let resume_capsule = host.resume_capsule();
    let cache = cache_controller.map(|controller| CacheOutput::from_controller(controller, None));
    let result = ResultEnvelope {
        schema_version: OUTPUT_SCHEMA_VERSION,
        kind: "result",
        status: ResultStatus::InteractionRequired,
        session_id,
        turn_id,
        provider: host.runtime().policy().provider_name.clone(),
        model: host.runtime().policy().model.as_str().to_owned(),
        output: String::new(),
        usage: UsageOutput {
            current_turn: UsageDelta::new(),
            session_provenance: UsageProvenance::of(&session_usage),
            current_turn_provenance: UsageProvenance::Unknown,
            session: session_usage,
            synthetic_cache,
        },
        lifecycle: LifecycleOutput {
            activation: activation_output(session),
            children: child_session_outputs(host),
            ..LifecycleOutput::default()
        },
        goal: final_goal,
        goal_continuation_turns,
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: Some(required.into()),
        recovery: recovery_output(host),
        account: None,
        background_exit: None,
        reasoning: Some(ReasoningOutput::of(&host.runtime().policy().reasoning)),
        cache,
        resume_capsule,
        error: shutdown_error,
    };

    match format {
        OutputFormat::Text => {
            write_text_projection(stderr, &result)?;
            let required = result
                .interaction_required
                .as_ref()
                .expect("interaction-required result metadata");
            writeln!(
                stderr,
                "smith: interaction required for request `{}` ({}); \
                 rerun in an interactive terminal",
                required.request_id,
                plural(required.question_count, "question", "questions")
            )
            .context("writing diagnostic to stderr")?;
            stderr.flush().context("flushing diagnostic stderr")?;
        }
        OutputFormat::Json | OutputFormat::StreamJson => write_json(stdout, &result)?,
    }

    Ok(Outcome {
        exit_code: INTERACTION_REQUIRED_EXIT,
    })
}

pub(super) async fn write_submission_failure(
    host: &HostSession,
    format: OutputFormat,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    submission_error: String,
) -> Result<Outcome> {
    let session = host.session();
    let final_goal = host.goal().ok().flatten();
    let goal_continuation_turns = final_goal.as_ref().map(|_| 0);
    let (shutdown_error, cache_controller) =
        shutdown_and_write_stream_tail(host, format, stdout).await?;
    let error = match shutdown_error {
        None => submission_error,
        Some(shutdown) => format!("{submission_error}; shutdown also failed: {shutdown}"),
    };
    let snapshot = host.snapshot();
    let session_usage = snapshot.usage.total();
    let synthetic_cache = SyntheticUsageOutput::from_records(snapshot.usage.records());
    let resume_capsule = host.resume_capsule();
    let cache = cache_controller.map(|controller| CacheOutput::from_controller(controller, None));
    let result = ResultEnvelope {
        schema_version: OUTPUT_SCHEMA_VERSION,
        kind: "result",
        status: ResultStatus::Failed,
        session_id: session.id().as_str().to_owned(),
        // Submission was rejected before the runtime minted an accepted turn
        // handle. The versioned schema retains its required string field and
        // uses the empty value to state that no turn exists.
        turn_id: String::new(),
        provider: host.runtime().policy().provider_name.clone(),
        model: host.runtime().policy().model.as_str().to_owned(),
        output: String::new(),
        usage: UsageOutput {
            current_turn: UsageDelta::new(),
            session_provenance: UsageProvenance::of(&session_usage),
            current_turn_provenance: UsageProvenance::Unknown,
            session: session_usage,
            synthetic_cache,
        },
        lifecycle: LifecycleOutput {
            activation: activation_output(session),
            children: child_session_outputs(host),
            ..LifecycleOutput::default()
        },
        goal: final_goal,
        goal_continuation_turns,
        artifacts: Vec::new(),
        approval_required: None,
        interaction_required: None,
        recovery: recovery_output(host),
        account: None,
        background_exit: None,
        reasoning: Some(ReasoningOutput::of(&host.runtime().policy().reasoning)),
        cache,
        resume_capsule,
        error: Some(error.clone()),
    };

    match format {
        OutputFormat::Text => {
            write_text_projection(stderr, &result)?;
            writeln!(stderr, "smith: {error}").context("writing diagnostic to stderr")?;
            stderr.flush().context("flushing diagnostic stderr")?;
        }
        OutputFormat::Json | OutputFormat::StreamJson => write_json(stdout, &result)?,
    }
    Ok(Outcome { exit_code: 1 })
}

/// Freezes a non-success host and emits the same terminal stream tail as a
/// completed turn: canonical shutdown first, then Smith's exact retained
/// controller projection. The returned controller is reused byte-for-byte in
/// the terminal result so stream and final JSON cannot diverge.
pub(super) async fn shutdown_and_write_stream_tail(
    host: &HostSession,
    format: OutputFormat,
    stdout: &mut impl Write,
) -> Result<(Option<String>, Option<CacheControllerSnapshot>)> {
    let mut events = (format == OutputFormat::StreamJson).then(|| host.client().events());
    let shutdown_error = host.shutdown().await.err().map(|error| error.to_string());

    if let Some(events) = events.as_mut() {
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(100), events.next()).await
        {
            let terminal = matches!(event.payload, RuntimeEvent::SessionShutdown);
            write_json(
                stdout,
                &StreamEnvelope {
                    schema_version: OUTPUT_SCHEMA_VERSION,
                    kind: "runtime_event",
                    event: &event,
                },
            )?;
            if terminal {
                break;
            }
        }
    }

    let cache_controller = host.cache_lifecycle();
    if format == OutputFormat::StreamJson
        && let Some(controller) = cache_controller.as_ref()
    {
        write_json(
            stdout,
            &CacheControllerEnvelope {
                schema_version: OUTPUT_SCHEMA_VERSION,
                kind: "cache_controller",
                controller,
            },
        )?;
    }

    Ok((shutdown_error, cache_controller))
}

pub(super) fn observe_sequence(last: &mut Option<u64>, current: u64, error: &mut Option<String>) {
    if let Some(previous) = *last
        && current != previous.saturating_add(1)
        && error.is_none()
    {
        *error = Some(format!(
            "runtime event stream lost events between sequence {previous} and {current}"
        ));
    }
    *last = Some(current);
}
