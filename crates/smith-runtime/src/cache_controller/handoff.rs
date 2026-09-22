//! One cache operation dispatch and its optional continuation-capsule handoff.

use super::*;

pub(super) async fn dispatch_once(
    session: &SessionHandle,
    config: &CacheControllerConfig,
    clock: &Arc<dyn Clock>,
    state: &Arc<Mutex<ControllerState>>,
    cancel: &Cancellation,
    decision: &CacheSchedulerDecision,
) -> Result<CacheOperationResult, String> {
    let identity = state
        .lock()
        .expect("cache controller state poisoned")
        .snapshot
        .lifecycle
        .current_identity
        .clone()
        .ok_or_else(|| "cache_identity_unavailable".to_owned())?;
    let interval = state
        .lock()
        .expect("cache controller state poisoned")
        .snapshot
        .parked_interval
        .unwrap_or_default();
    let action = decision
        .action
        .ok_or_else(|| "cache_action_unavailable".to_owned())?;
    let purpose = action.purpose();
    let operation = CacheOperationId::new(format!(
        "smith-cache-{interval}-{}-{}",
        purpose.as_str(),
        &identity.digest().as_str()[..identity.digest().as_str().len().min(16)]
    ));
    let authority = CacheAuthority::new(format!("smith-cache-authority:{}", session.id()));
    let budget = CacheOperationBudget {
        max_input_tokens: if config.policy.max_maintenance_input_tokens == 0 {
            config.model_input_limit
        } else {
            config
                .policy
                .max_maintenance_input_tokens
                .min(config.model_input_limit)
        },
        max_output_bytes: 16 * 1024,
        max_output_tokens: config.policy.max_maintenance_output_tokens,
    };
    let operation_cancel = cancel.child();
    let deadline = Deadline::after(clock.as_ref(), config.policy.maintenance_deadline_ms);
    let request = match action {
        CacheMaintenanceAction::HandoffCheckpoint => session.cache_handoff_from_last_plan(
            operation,
            CacheHandoffSuffix::new(HANDOFF_SUFFIX).map_err(|_| "handoff_suffix_invalid")?,
            authority,
            budget,
            operation_cancel,
            deadline,
        ),
        CacheMaintenanceAction::Keepalive => session.cache_operation_from_last_plan(
            operation,
            ProviderAttemptPurpose::CacheKeepalive,
            authority,
            budget,
            operation_cancel,
            deadline,
        ),
    }
    .map_err(|error| format!("cache_preflight:{:?}", error.kind))?;
    session
        .dispatch_cache_operation(request)
        .await
        .map_err(|error| format!("cache_dispatch:{:?}", error.kind))
}

pub(super) async fn persist_handoff(
    session: &SessionHandle,
    config: &CacheControllerConfig,
    capsule: Option<&ResumeCapsuleSlot>,
    artifacts: Option<&dyn ArtifactStore>,
    result: &CacheOperationResult,
    expected_identity: &CacheIdentity,
    now: Timestamp,
) -> Result<(), String> {
    if &result.identity != expected_identity || !identity_matches_config(&result.identity, config) {
        return Err("handoff_identity_mismatch".to_owned());
    }
    let Some(capsule) = capsule else {
        return Ok(());
    };
    let Some(_identity_lease) = session.lock_current_cache_identity(&result.identity).await else {
        return Err("handoff_identity_retired".to_owned());
    };
    if result.purpose != ProviderAttemptPurpose::CacheHandoffCheckpoint
        || result.outcome != CacheOperationOutcome::Completed
    {
        let (previous, expected) = capsule
            .try_update_atomic(|candidate| {
                project_handoff_cache_state(candidate, result);
                Ok(())
            })
            .map_err(|error| error.to_string())?;
        return persist_capsule_after_update(session, capsule, &expected, previous)
            .await
            .map_err(|_| "capsule_persist_failed".to_owned());
    }
    // Persist the Runtime state that this handoff supersedes before changing
    // summary purpose. RedactingSessionStore records its protected artifact
    // reference, allowing the post-handoff save to distinguish the same stale
    // ordinary state from a genuinely newer Runtime summary.
    session
        .persist()
        .await
        .map_err(|_| "capsule_baseline_persist_failed".to_owned())?;
    let revision = RegistryRevision::from_content(format!(
        "{}\n{}",
        result.operation,
        result
            .captured_output
            .as_ref()
            .map_or("", |output| output.as_str().trim())
    ));
    let Some(output) = result.captured_output.as_ref() else {
        return persist_failed_handoff(
            session,
            capsule,
            config,
            result,
            revision,
            now,
            "handoff_output_missing",
        )
        .await;
    };
    let body = output.as_str().trim();
    if body.is_empty() || body.len() > MAX_SUMMARY_BYTES {
        return persist_failed_handoff(
            session,
            capsule,
            config,
            result,
            revision,
            now,
            "handoff_output_invalid",
        )
        .await;
    }
    let Some(artifacts) = artifacts else {
        return persist_failed_handoff(
            session,
            capsule,
            config,
            result,
            revision,
            now,
            "handoff_artifact_store_unavailable",
        )
        .await;
    };
    let write = ArtifactWrite {
        bytes: body.as_bytes().to_vec(),
        media_type: RESUME_SUMMARY_MEDIA_TYPE.to_owned(),
        sensitivity: ArtifactSensitivity::Sensitive,
        retention: ArtifactRetention::Session,
        provenance: ArtifactProvenance::new(session.id().clone(), RESUME_SUMMARY_ARTIFACT_PURPOSE),
        idempotency_key: result.operation.to_string(),
    };
    let reference = match artifacts.put(write).await {
        Ok(reference) => reference,
        Err(_) => {
            return persist_failed_handoff(
                session,
                capsule,
                config,
                result,
                revision,
                now,
                "handoff_artifact_write_failed",
            )
            .await;
        }
    };
    let (previous, expected) = capsule
        .try_update_atomic(|candidate| {
            project_handoff_cache_state(candidate, result);
            let coverage = vec![SummaryCoverage::new(
                "canonical_events",
                0,
                candidate.exact_state.watermark,
            )];
            candidate.record_handoff_summary(
                config.provider.clone(),
                config.model.clone(),
                revision,
                result.identity.clone(),
                now,
                body,
                coverage,
            )?;
            candidate.attach_summary_artifact(reference.clone())?;
            let artifact_id = reference.id.to_string();
            if candidate.exact_state.artifacts.len() < MAX_ARTIFACTS
                && !candidate
                    .exact_state
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.artifact == artifact_id)
            {
                let projection = ArtifactProjection {
                    artifact: artifact_id,
                    digest: Some(Fingerprint::of(reference.digest.hex.as_bytes())),
                };
                candidate.exact_state.artifacts.push(projection);
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    persist_capsule_after_update(session, capsule, &expected, previous)
        .await
        .map_err(|_| "capsule_persist_failed".to_owned())
}

pub(super) fn project_handoff_cache_state(
    capsule: &mut ResumeCapsule,
    result: &CacheOperationResult,
) {
    capsule.cache.prior_identity = Some(result.identity.clone());
    capsule.cache.provider_warmth = match result.state {
        agent_runtime_core::event::CacheState::WarmObserved => ResumeCacheWarmth::WarmObserved,
        agent_runtime_core::event::CacheState::MissObserved => ResumeCacheWarmth::MissObserved,
        agent_runtime_core::event::CacheState::Expired => ResumeCacheWarmth::ExpiredObserved,
        _ => ResumeCacheWarmth::Unknown,
    };
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn persist_failed_handoff(
    session: &SessionHandle,
    capsule: &ResumeCapsuleSlot,
    config: &CacheControllerConfig,
    result: &CacheOperationResult,
    revision: RegistryRevision,
    now: Timestamp,
    reason: &'static str,
) -> Result<(), String> {
    let (previous, expected) = capsule
        .try_update_atomic(|candidate| {
            project_handoff_cache_state(candidate, result);
            let coverage = vec![SummaryCoverage::new(
                "canonical_events",
                0,
                candidate.exact_state.watermark,
            )];
            candidate.record_failed_handoff_summary(
                config.provider.clone(),
                config.model.clone(),
                revision,
                result.identity.clone(),
                now,
                coverage,
            )
        })
        .map_err(|error| error.to_string())?;
    persist_capsule_after_update(session, capsule, &expected, previous)
        .await
        .map_err(|_| "capsule_persist_failed".to_owned())?;
    Err(reason.to_owned())
}

/// Persists one already-mutated capsule and restores its previous projection
/// when the store rejects the write.  The conditional restore prevents a
/// concurrent newer event from being erased by an older failed save.
pub(super) async fn persist_capsule_after_update(
    session: &SessionHandle,
    capsule: &ResumeCapsuleSlot,
    expected: &ResumeCapsule,
    previous: ResumeCapsule,
) -> Result<(), ()> {
    match session.persist().await {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = capsule.restore_if_current(expected, previous);
            Err(())
        }
    }
}
