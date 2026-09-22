//! Independent root-idle compaction admission, execution, and projection.

use super::synthetic_projection::{
    artifact_sensitivity, counter_provenance, push_synthetic_attempt, summary_usage,
};
use super::*;

/// Returns the independent root-idle deadline. Parking, goal state, and child
/// activity are intentionally absent: they belong to the separate
/// cache-maintenance scheduler, not ordinary semantic compaction.
pub(super) fn idle_compaction_wait_ms(
    config: &CacheControllerConfig,
    state: &ControllerState,
    now: Timestamp,
) -> Option<u64> {
    if !config.policy.idle_compaction
        || state.shutting_down
        || state.snapshot.lifecycle.shutdown
        || state.event_gap
        || state.parent_turn_active
        || state.snapshot.operation_in_flight
        || state.snapshot.idle_compaction.attempted
    {
        return None;
    }
    state.snapshot.idle_compaction.interval_id.as_ref()?;
    let activity = state.last_meaningful_activity_at?;
    Some(
        activity
            .plus_millis(config.policy.inactivity_limit_ms)
            .0
            .saturating_sub(now.0),
    )
}

/// Evaluates the durable idle gate once at the final host boundary before
/// marker persistence or the Runtime call. Admission retires the old cache
/// lease immediately, including when a later step fails or returns Busy.
pub(super) fn admit_idle_compaction(
    config: &CacheControllerConfig,
    state: &Arc<Mutex<ControllerState>>,
    parking: Option<&ParkingSnapshot>,
    now: Timestamp,
    cancelled: bool,
) -> Option<String> {
    let mut state = state.lock().expect("cache controller state poisoned");
    let interval_id = state.snapshot.idle_compaction.interval_id.clone()?;
    let child_active = parking.is_some_and(|parking| !parking.pending_children.is_empty());
    let safe_boundary = !cancelled
        && !state.shutting_down
        && !state.snapshot.lifecycle.shutdown
        && !state.parent_turn_active
        && !state.snapshot.operation_in_flight
        && !state.event_gap;
    let last_meaningful_activity_at = state.last_meaningful_activity_at;
    let lifecycle_active = !state.shutting_down && !state.snapshot.lifecycle.shutdown;
    let shutdown = cancelled || state.shutting_down;
    let decision = state.snapshot.idle_compaction.evaluate(
        config.policy,
        &IdleCompactionInput {
            now,
            last_meaningful_activity_at,
            interval_id: interval_id.clone(),
            safe_boundary,
            lifecycle_active,
            shutdown,
            child_active,
        },
    );
    state.snapshot.idle_compaction_decision = Some(decision);
    if decision.disposition != IdleCompactionDisposition::Attempt {
        return None;
    }
    state.snapshot.scheduled_for = None;
    state.boundary_evaluated = true;
    state.snapshot.lifecycle.retire_after_compaction(now);
    Some(interval_id)
}

/// Runs one admitted ordinary idle-compaction attempt. The capsule marker is
/// persisted before entering Runtime, so a store failure cannot authorize a
/// provider retry.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_idle_compaction(
    session: &SessionHandle,
    config: &CacheControllerConfig,
    clock: &Arc<dyn Clock>,
    parking: Option<&ParkingSnapshot>,
    capsule: Option<&ResumeCapsuleSlot>,
    artifacts: Option<&dyn ArtifactStore>,
    state: &Arc<Mutex<ControllerState>>,
    cancel: &Cancellation,
) {
    let started_at = clock.now();
    let Some(interval_id) =
        admit_idle_compaction(config, state, parking, started_at, cancel.is_cancelled())
    else {
        return;
    };

    if let Err(reason) = persist_idle_attempt_marker(session, capsule, &interval_id).await {
        record_idle_outcome(
            state,
            IdleOutcomeRecord {
                outcome: IdleCompactionOutcome::Failed,
                reason: Some(reason.to_owned()),
                started_at,
                finished_at: clock.now(),
                model: None,
                revision: None,
                usage: &UsageDelta::new(),
            },
        );
        return;
    }

    let admission = match session.try_idle_compaction().await {
        Ok(admission) => admission,
        Err(_) => {
            let reason = "runtime_error";
            let usage = UsageDelta::new();
            let revision = idle_failure_revision(reason);
            let Some(_projection_admission) = OptionalProjectionAdmission::try_begin(state) else {
                record_idle_outcome(
                    state,
                    IdleOutcomeRecord {
                        outcome: IdleCompactionOutcome::Shutdown,
                        reason: Some("shutdown".to_owned()),
                        started_at,
                        finished_at: clock.now(),
                        model: None,
                        revision: None,
                        usage: &usage,
                    },
                );
                return;
            };
            let _ = persist_idle_failure(
                session,
                capsule,
                config,
                IdleFailureProjection {
                    model: config
                        .semantic_summary_model
                        .clone()
                        .expect("admitted idle compaction has a summary model"),
                    revision: revision.clone(),
                    generated_at: clock.now(),
                    coverage: idle_summary_coverage(capsule),
                    usage: &usage,
                },
            )
            .await;
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::Failed,
                    reason: Some(reason.to_owned()),
                    started_at,
                    finished_at: clock.now(),
                    model: None,
                    revision: None,
                    usage: &usage,
                },
            );
            return;
        }
    };

    // Runtime has already committed any accepted compaction and its usage.
    // Admission and stop_scheduling serialize through the same state mutex:
    // shutdown prevents a new optional projection, while a projection that
    // won first is drained before Runtime's final session save.
    let _projection_admission = match &admission {
        IdleCompactionAdmission::Accepted { .. } => {
            match OptionalProjectionAdmission::try_begin(state) {
                Some(admission) => Some(admission),
                None => {
                    let empty = UsageDelta::new();
                    let (usage, model, revision) = match &admission {
                        IdleCompactionAdmission::Accepted {
                            summary: Some(summary),
                            ..
                        } => (
                            &summary.usage,
                            Some(summary.model_id.clone()),
                            Some(summary.summary_revision.clone()),
                        ),
                        IdleCompactionAdmission::Accepted {
                            summary: None,
                            usage,
                            ..
                        } => (usage, None, None),
                        IdleCompactionAdmission::Busy | IdleCompactionAdmission::Shutdown => {
                            (&empty, None, None)
                        }
                    };
                    record_idle_outcome(
                        state,
                        IdleOutcomeRecord {
                            outcome: IdleCompactionOutcome::Shutdown,
                            reason: Some("shutdown".to_owned()),
                            started_at,
                            finished_at: clock.now(),
                            model,
                            revision,
                            usage,
                        },
                    );
                    return;
                }
            }
        }
        IdleCompactionAdmission::Busy | IdleCompactionAdmission::Shutdown => None,
    };

    match admission {
        IdleCompactionAdmission::Accepted {
            summary: Some(summary),
            ..
        } => {
            finish_idle_summary(
                session,
                config,
                capsule,
                artifacts,
                state,
                clock,
                started_at,
                &interval_id,
                summary,
            )
            .await;
        }
        IdleCompactionAdmission::Accepted {
            summary: None,
            fallback_reason,
            usage,
        } => {
            let reason = bounded_idle_reason(fallback_reason.as_deref())
                .unwrap_or_else(|| "summary_unavailable".to_owned());
            let revision = idle_failure_revision(&reason);
            let _ = persist_idle_failure(
                session,
                capsule,
                config,
                IdleFailureProjection {
                    model: config
                        .semantic_summary_model
                        .clone()
                        .expect("admitted idle compaction has a summary model"),
                    revision: revision.clone(),
                    generated_at: clock.now(),
                    coverage: idle_summary_coverage(capsule),
                    usage: &usage,
                },
            )
            .await;
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::NoSummary,
                    reason: Some(reason),
                    started_at,
                    finished_at: clock.now(),
                    model: None,
                    revision: None,
                    usage: &usage,
                },
            );
        }
        IdleCompactionAdmission::Busy => {
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::Busy,
                    reason: Some("runtime_busy".to_owned()),
                    started_at,
                    finished_at: clock.now(),
                    model: None,
                    revision: None,
                    usage: &UsageDelta::new(),
                },
            );
        }
        IdleCompactionAdmission::Shutdown => {
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::Shutdown,
                    reason: Some("shutdown".to_owned()),
                    started_at,
                    finished_at: clock.now(),
                    model: None,
                    revision: None,
                    usage: &UsageDelta::new(),
                },
            );
        }
    }
}

pub(super) async fn persist_idle_attempt_marker(
    session: &SessionHandle,
    capsule: Option<&ResumeCapsuleSlot>,
    interval_id: &str,
) -> Result<(), &'static str> {
    let Some(capsule) = capsule else {
        return Err("attempt_marker_unavailable");
    };
    let (previous, expected) = capsule
        .try_update_atomic(|capsule| {
            capsule.cache.idle_compaction_interval_id = Some(interval_id.to_owned());
            capsule.cache.idle_compaction_attempted = true;
            Ok(())
        })
        .map_err(|_| "attempt_marker_projection_failed")?;
    match session.persist().await {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = capsule.restore_if_current(&expected, previous);
            Err("attempt_marker_persist_failed")
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn finish_idle_summary(
    session: &SessionHandle,
    config: &CacheControllerConfig,
    capsule: Option<&ResumeCapsuleSlot>,
    artifacts: Option<&dyn ArtifactStore>,
    state: &Arc<Mutex<ControllerState>>,
    clock: &Arc<dyn Clock>,
    started_at: Timestamp,
    interval_id: &str,
    summary: ProtectedSemanticSummary,
) {
    // Runtime has already committed the accepted compaction (including its
    // extension state and usage) before returning this protected result. Any
    // failure below is therefore a bounded loss/warning for the optional
    // capsule projection, never a failed compaction and never a retry signal.
    let model = summary.model_id.clone();
    let revision = summary.summary_revision.clone();
    let usage = summary.usage.clone();
    let sensitivity = match artifact_sensitivity(summary.sensitivity) {
        Ok(sensitivity) => sensitivity,
        Err(reason) => {
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::Completed,
                    reason: Some(reason.to_owned()),
                    started_at,
                    finished_at: clock.now(),
                    model: Some(model),
                    revision: Some(revision),
                    usage: &usage,
                },
            );
            return;
        }
    };
    let body = summary.body.as_str();
    if body.is_empty() || body.len() > MAX_SUMMARY_BYTES {
        record_idle_outcome(
            state,
            IdleOutcomeRecord {
                outcome: IdleCompactionOutcome::Completed,
                reason: Some("summary_output_invalid".to_owned()),
                started_at,
                finished_at: clock.now(),
                model: Some(model),
                revision: Some(revision),
                usage: &usage,
            },
        );
        return;
    }
    let Some(artifacts) = artifacts else {
        record_idle_outcome(
            state,
            IdleOutcomeRecord {
                outcome: IdleCompactionOutcome::Completed,
                reason: Some("summary_artifact_store_unavailable".to_owned()),
                started_at,
                finished_at: clock.now(),
                model: Some(model),
                revision: Some(revision),
                usage: &usage,
            },
        );
        return;
    };
    let write = ArtifactWrite {
        bytes: body.as_bytes().to_vec(),
        media_type: RESUME_SUMMARY_MEDIA_TYPE.to_owned(),
        sensitivity,
        retention: ArtifactRetention::Session,
        provenance: ArtifactProvenance::new(
            session.id().clone(),
            RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
        ),
        idempotency_key: format!(
            "idle-summary:{}:{}",
            Fingerprint::of(interval_id.as_bytes()),
            revision.as_str()
        ),
    };
    let reference = match artifacts.put(write).await {
        Ok(reference) => reference,
        Err(_) => {
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::Completed,
                    reason: Some("summary_artifact_write_failed".to_owned()),
                    started_at,
                    finished_at: clock.now(),
                    model: Some(model),
                    revision: Some(revision),
                    usage: &usage,
                },
            );
            return;
        }
    };

    let coverage = idle_summary_coverage(capsule);
    let Some(capsule) = capsule else {
        record_idle_outcome(
            state,
            IdleOutcomeRecord {
                outcome: IdleCompactionOutcome::Completed,
                reason: Some("attempt_marker_unavailable".to_owned()),
                started_at,
                finished_at: clock.now(),
                model: Some(model),
                revision: Some(revision),
                usage: &usage,
            },
        );
        return;
    };
    let projection = capsule.try_update_atomic(|capsule| {
        capsule.record_ordinary_summary(
            config
                .semantic_summary_provider
                .clone()
                .expect("completed idle compaction has a summary provider"),
            model.clone(),
            revision.clone(),
            clock.now(),
            body,
            coverage.clone(),
        )?;
        if let Some(summary) = capsule.semantic_summary.as_mut() {
            summary.provenance.usage = summary_usage(&usage);
        }
        capsule.attach_summary_artifact(reference.clone())?;
        if capsule.exact_state.artifacts.len() < MAX_ARTIFACTS {
            capsule.exact_state.artifacts.push(ArtifactProjection {
                artifact: reference.id.to_string(),
                digest: Some(Fingerprint::of(reference.digest.hex.as_bytes())),
            });
        }
        Ok::<(), crate::resume_capsule::ResumeCapsuleError>(())
    });
    let (previous, expected) = match projection {
        Ok(projection) => projection,
        Err(_) => {
            record_idle_outcome(
                state,
                IdleOutcomeRecord {
                    outcome: IdleCompactionOutcome::Completed,
                    reason: Some("summary_capsule_projection_failed".to_owned()),
                    started_at,
                    finished_at: clock.now(),
                    model: Some(model),
                    revision: Some(revision),
                    usage: &usage,
                },
            );
            return;
        }
    };
    if session.persist().await.is_err() {
        let _ = capsule.restore_if_current(&expected, previous);
        record_idle_outcome(
            state,
            IdleOutcomeRecord {
                outcome: IdleCompactionOutcome::Completed,
                reason: Some("capsule_persist_failed".to_owned()),
                started_at,
                finished_at: clock.now(),
                model: Some(model),
                revision: Some(revision),
                usage: &usage,
            },
        );
        return;
    }
    record_idle_outcome(
        state,
        IdleOutcomeRecord {
            outcome: IdleCompactionOutcome::Completed,
            reason: None,
            started_at,
            finished_at: clock.now(),
            model: Some(model),
            revision: Some(revision),
            usage: &usage,
        },
    );
}

pub(super) fn idle_summary_coverage(capsule: Option<&ResumeCapsuleSlot>) -> Vec<SummaryCoverage> {
    let watermark = capsule
        .map(|capsule| capsule.snapshot().exact_state.watermark)
        .unwrap_or_default();
    vec![SummaryCoverage::new("canonical_events", 0, watermark)]
}

pub(super) struct IdleFailureProjection<'a> {
    model: String,
    revision: RegistryRevision,
    generated_at: Timestamp,
    coverage: Vec<SummaryCoverage>,
    usage: &'a UsageDelta,
}

pub(super) async fn persist_idle_failure(
    session: &SessionHandle,
    capsule: Option<&ResumeCapsuleSlot>,
    config: &CacheControllerConfig,
    failure: IdleFailureProjection<'_>,
) -> Result<(), &'static str> {
    let IdleFailureProjection {
        model,
        revision,
        generated_at,
        coverage,
        usage,
    } = failure;
    let Some(capsule) = capsule else {
        return Err("attempt_marker_unavailable");
    };
    // Establish the exact protected Runtime-summary state that this failed
    // attempt follows before publishing failure metadata. The subsequent save
    // can then prove that an unchanged Runtime state is stale and must not
    // overwrite the newer failure projection.
    session
        .persist()
        .await
        .map_err(|_| "capsule_baseline_persist_failed")?;
    let (previous, expected) = capsule
        .try_update_atomic(|capsule| {
            capsule.record_failed_ordinary_summary(
                config
                    .semantic_summary_provider
                    .clone()
                    .expect("admitted idle compaction has a summary provider"),
                model,
                revision,
                generated_at,
                coverage,
            )?;
            if let Some(summary) = capsule.semantic_summary.as_mut() {
                summary.provenance.usage = summary_usage(usage);
            }
            Ok::<(), crate::resume_capsule::ResumeCapsuleError>(())
        })
        .map_err(|_| "summary_failure_projection_failed")?;
    match session.persist().await {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = capsule.restore_if_current(&expected, previous);
            Err("capsule_persist_failed")
        }
    }
}

pub(super) struct IdleOutcomeRecord<'a> {
    pub(super) outcome: IdleCompactionOutcome,
    pub(super) reason: Option<String>,
    pub(super) started_at: Timestamp,
    pub(super) finished_at: Timestamp,
    pub(super) model: Option<String>,
    pub(super) revision: Option<RegistryRevision>,
    pub(super) usage: &'a UsageDelta,
}

pub(super) fn record_idle_outcome(
    state: &Arc<Mutex<ControllerState>>,
    record: IdleOutcomeRecord<'_>,
) {
    let IdleOutcomeRecord {
        outcome,
        reason,
        started_at,
        finished_at,
        model,
        revision,
        usage,
    } = record;
    let mut state = state.lock().expect("cache controller state poisoned");
    let bounded_reason = reason.map(bounded_metadata_string);
    let bounded_model = model.map(bounded_metadata_string);
    let bounded_revision = revision.map(|revision| {
        RegistryRevision::new(bounded_metadata_string(revision.as_str().to_owned()))
    });
    state.snapshot.idle_compaction_outcome = Some(outcome);
    state.snapshot.idle_compaction_reason = bounded_reason.clone();
    state.snapshot.idle_compaction_latency_ms = Some(finished_at.0.saturating_sub(started_at.0));
    if let Some(model) = bounded_model {
        state.snapshot.idle_compaction_model = Some(model);
    }
    state.snapshot.idle_compaction_revision = bounded_revision;
    state.snapshot.idle_compaction_usage = summary_usage(usage);
    state.snapshot.scheduled_for = None;
    let status = bounded_reason.map_or_else(
        || format!("{outcome:?}").to_ascii_lowercase(),
        |reason| format!("{}:{reason}", format!("{outcome:?}").to_ascii_lowercase()),
    );
    let provider = state
        .snapshot
        .idle_compaction_provider
        .clone()
        .expect("idle outcome has a summary provider route");
    let model = state
        .snapshot
        .idle_compaction_model
        .clone()
        .expect("idle outcome has a summary model route");
    push_synthetic_attempt(
        &mut state.snapshot,
        SyntheticCacheAttemptProjection {
            operation: None,
            attempt: None,
            purpose: ProviderAttemptPurpose::IdleCompaction,
            provider,
            model,
            cache_identity: None,
            usage: summary_usage(usage),
            counter_provenance: counter_provenance(usage),
            cost_micro_usd: None,
            cost_provenance: SyntheticCostProvenance::Unknown,
            latency_ms: finished_at.0.saturating_sub(started_at.0),
            status: bounded_metadata_string(status),
        },
    );
}
pub(super) fn bounded_idle_reason(reason: Option<&str>) -> Option<String> {
    let reason = reason?;
    let category = match reason {
        "original_store_integrity_failed"
        | "summary_model_unavailable"
        | "summary_output_invalid"
        | "summary_usage_limit_exceeded" => reason,
        _ => "summary_fallback",
    };
    Some(category.to_owned())
}

pub(super) fn idle_failure_revision(reason: &str) -> RegistryRevision {
    RegistryRevision::from_content(format!("smith-idle-compaction:{reason}"))
}
