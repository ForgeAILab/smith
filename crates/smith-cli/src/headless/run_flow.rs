//! Headless turn event consumption and terminal stream output.

use super::*;

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
    let mut finish = None;
    let mut turn_usage = UsageDelta::new();
    let mut cache: Option<CacheOutput> = None;
    let mut last_error = None;
    let mut last_attempt_error = None;
    let mut last_sequence = None;
    let mut sequence_error = None;
    let mut pending_interaction: Option<InteractionRequired> = None;
    let mut event_interaction_required: Option<InteractionRequired> = None;
    let mut lifecycle = LifecycleOutput {
        activation: initial_activation,
        ..LifecycleOutput::default()
    };
    let mut goal_continuation_turns = 0_u32;
    let mut active_goal_turns = BTreeSet::new();
    let mut active_child_completion_turns = BTreeSet::new();
    let mut pending_child_completion_delivery = false;

    while let Some(event) = events.next().await {
        cache_projection.apply(&event);
        observe_sequence(&mut last_sequence, event.seq, &mut sequence_error);
        if matches!(
            &event.payload,
            RuntimeEvent::InternalTurnStarted { source } if source.kind == "goal"
        ) {
            goal_continuation_turns = goal_continuation_turns.saturating_add(1);
            if let Some(turn) = &event.turn {
                active_goal_turns.insert(turn.as_str().to_owned());
            }
        }
        if matches!(
            &event.payload,
            RuntimeEvent::InternalTurnStarted { source }
                if source.kind == "delegation.child-completion"
        ) {
            pending_child_completion_delivery = false;
            if let Some(turn) = &event.turn {
                active_child_completion_turns.insert(turn.as_str().to_owned());
            }
        }
        if matches!(
            &event.payload,
            RuntimeEvent::ChildCompleted { .. } | RuntimeEvent::ChildNeedsInput { .. }
        ) {
            pending_child_completion_delivery = true;
        }
        let belongs_to_turn = event.turn.as_ref() == Some(&turn_id);
        let belongs_to_goal_turn = event
            .turn
            .as_ref()
            .is_some_and(|turn| active_goal_turns.contains(turn.as_str()));
        if belongs_to_turn {
            match &event.payload {
                RuntimeEvent::Usage { record } if !is_synthetic_usage(record) => {
                    turn_usage.merge(&record.delta)
                }
                RuntimeEvent::CachePlanChanged {
                    preserved_prefix_tokens,
                    invalidated_prefix_tokens,
                    provider_cache_supported,
                    ..
                } => {
                    cache = Some(CacheOutput {
                        provider_cache_supported: *provider_cache_supported,
                        preserved_prefix_tokens: *preserved_prefix_tokens,
                        invalidated_prefix_tokens: *invalidated_prefix_tokens,
                        state: None,
                        cache_identity: None,
                        expected_read_tokens: None,
                        observed_read_tokens: None,
                        observed_write_tokens: None,
                        missed_tokens: None,
                        confidence: None,
                        cache_read_percent: None,
                        miss_count: None,
                        rebilled_tokens: None,
                        idle_minutes: None,
                        extra_cost_micro_usd: None,
                        lifecycle: None,
                        controller: None,
                        notice: None,
                    });
                }
                RuntimeEvent::Error { error } => last_error = Some(error.to_string()),
                // An attempt that ended in an error reports its own cause
                // even when the retry loop then spends the attempt budget
                // without a terminal error event; it is the only account of
                // why a `limit_reached` turn produced nothing.
                RuntimeEvent::ProviderAttemptFinished {
                    error: Some(error), ..
                } => last_attempt_error = Some(error.to_string()),
                RuntimeEvent::ProviderAttemptOutputCommitted { .. } => {
                    lifecycle.attempts_committed = lifecycle.attempts_committed.saturating_add(1);
                }
                RuntimeEvent::ProviderAttemptOutputDiscarded { .. } => {
                    lifecycle.attempts_discarded = lifecycle.attempts_discarded.saturating_add(1);
                }
                RuntimeEvent::CapabilitiesActivated { epoch, activation } => {
                    lifecycle.activation = Some(ActivationOutput {
                        epoch: u64::from(*epoch),
                        capabilities: activation
                            .iter()
                            .map(|capability| capability.id.to_string())
                            .collect(),
                    });
                }
                RuntimeEvent::PlanUpdated {
                    revision,
                    sensitivity,
                    counts,
                    items,
                } => {
                    lifecycle.plan = Some(plan_output(
                        *revision,
                        *sensitivity,
                        counts.clone(),
                        items.clone(),
                    ));
                }
                RuntimeEvent::TurnCompleted {
                    finish: completed, ..
                } => {
                    if let TurnFinish::NeedsInput { request } = completed {
                        let required = pending_interaction
                            .take()
                            .filter(|pending| pending.request_id == request.as_str())
                            .unwrap_or_else(|| InteractionRequired {
                                request_id: request.as_str().to_owned(),
                                question_count: 0,
                            });
                        event_interaction_required.get_or_insert(required);
                    }
                    finish = Some(completed.clone());
                }
                RuntimeEvent::InteractionRequested {
                    request,
                    question_count,
                    ..
                } => {
                    pending_interaction = Some(InteractionRequired {
                        request_id: request.as_str().to_owned(),
                        question_count: usize::from(*question_count),
                    });
                }
                RuntimeEvent::InteractionResolved {
                    request,
                    outcome: InteractionOutcomeKind::Unavailable,
                    ..
                } => {
                    if let Some(required) = pending_interaction.take()
                        && required.request_id == request.as_str()
                    {
                        event_interaction_required.get_or_insert(required);
                    }
                }
                _ => {}
            }
        }
        if belongs_to_goal_turn {
            match &event.payload {
                RuntimeEvent::Error { error } => last_error = Some(error.to_string()),
                RuntimeEvent::InteractionRequested {
                    request,
                    question_count,
                    ..
                } => {
                    pending_interaction = Some(InteractionRequired {
                        request_id: request.as_str().to_owned(),
                        question_count: usize::from(*question_count),
                    });
                }
                RuntimeEvent::InteractionResolved {
                    request,
                    outcome: InteractionOutcomeKind::Unavailable,
                    ..
                } => {
                    if let Some(required) = pending_interaction.take()
                        && required.request_id == request.as_str()
                    {
                        event_interaction_required.get_or_insert(required);
                    }
                }
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::NeedsInput { request },
                    ..
                } => {
                    let required = pending_interaction
                        .take()
                        .filter(|pending| pending.request_id == request.as_str())
                        .unwrap_or_else(|| InteractionRequired {
                            request_id: request.as_str().to_owned(),
                            question_count: 0,
                        });
                    event_interaction_required.get_or_insert(required);
                }
                _ => {}
            }
        }

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
        if belongs_to_goal_turn
            && matches!(&event.payload, RuntimeEvent::TurnCompleted { .. })
            && let Some(turn) = &event.turn
        {
            active_goal_turns.remove(turn.as_str());
        }
        if matches!(&event.payload, RuntimeEvent::TurnCompleted { .. })
            && let Some(turn) = &event.turn
        {
            active_child_completion_turns.remove(turn.as_str());
        }
        if finish
            .as_ref()
            .is_some_and(|finish| !finish_waits_for_required_follow_up(finish))
        {
            break;
        }
        if finish.is_some() {
            match host.goal() {
                Ok(Some(goal)) if goal.status == GoalStatus::Active => {}
                Ok(_)
                    if active_goal_turns.is_empty()
                        && active_child_completion_turns.is_empty()
                        && !pending_child_completion_delivery
                        && !has_required_child_work(host) =>
                {
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    sequence_error.get_or_insert_with(|| {
                        format!("persistent goal state became unavailable: {error}")
                    });
                    break;
                }
            }
        }
    }

    let stream_error = finish
        .is_none()
        .then(|| "the runtime event stream ended before the turn completed".to_owned());

    lifecycle.children = child_session_outputs(host);
    lifecycle.parent_state = host
        .delegation_parking()
        .map(|snapshot| snapshot.state.as_str());

    let final_goal = match host.goal() {
        Ok(goal) => goal,
        Err(error) => {
            sequence_error
                .get_or_insert_with(|| format!("persistent goal state unavailable: {error}"));
            None
        }
    };
    let goal_continuation_turns = final_goal.as_ref().map(|_| goal_continuation_turns);

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
            observe_sequence(&mut last_sequence, event.seq, &mut sequence_error);
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

    let snapshot = session.snapshot();
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
    if let Some(summary) = cache_projection.completed_turn(turn_id.as_str()) {
        let summary = cache_price
            .map(|price| cache_projection.with_price(summary, price))
            .unwrap_or_else(|| summary.clone());
        cache = Some(CacheOutput::from_summary(&summary, cache));
    }
    let cache_lifecycle = cache_projection.lifecycle().clone();
    if cache_lifecycle != CacheLifecycleSummary::default() {
        cache = Some(CacheOutput::from_lifecycle(cache_lifecycle, cache));
    }
    if let Some(controller) = cache_controller {
        cache = Some(CacheOutput::from_controller(controller, cache));
    }
    let approval_required = approval.and_then(HeadlessApproval::required);
    let interaction_required = interaction
        .and_then(HeadlessInteraction::required)
        .or(event_interaction_required);
    let lifecycle_error = shutdown_error.or(stream_error).or(sequence_error);
    let error = background_exit_error
        .or(lifecycle_error)
        .or_else(|| terminal_error(finish.as_ref(), last_error, last_attempt_error));
    let (status, exit_code) = outcome(
        finish.as_ref(),
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
            current_turn_provenance: UsageProvenance::of(&turn_usage),
            session_provenance: UsageProvenance::of(&session_usage),
            current_turn: turn_usage,
            session: session_usage,
            synthetic_cache,
        },
        lifecycle,
        goal: final_goal,
        goal_continuation_turns,
        artifacts,
        account: account_output(credential_pool, rotation),
        approval_required: approval_required.map(Into::into),
        interaction_required: interaction_required.map(Into::into),
        recovery: recovery_output(host),
        background_exit: background_exit_output,
        reasoning: Some(ReasoningOutput::of(&host.runtime().policy().reasoning)),
        cache,
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
                    "interaction required for request `{}` ({} question(s)); \
                     rerun in an interactive terminal",
                    required.request_id, required.question_count
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
    let snapshot = session.snapshot();
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
                "smith: interaction required for request `{}` ({} question(s)); \
                 rerun in an interactive terminal",
                required.request_id, required.question_count
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
    let snapshot = session.snapshot();
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
