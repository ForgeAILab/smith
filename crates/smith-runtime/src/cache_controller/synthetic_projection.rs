//! Bounded accounting projections for synthetic cache and idle attempts.

use super::*;

pub(super) fn record_cache_attempt_projection(
    state: &Arc<Mutex<ControllerState>>,
    result: &CacheOperationResult,
    usage: Option<&UsageDelta>,
    started_at: Timestamp,
    finished_at: Timestamp,
) {
    let usage = usage.cloned().unwrap_or_default();
    let mut state = state.lock().expect("cache controller state poisoned");
    let status = result
        .terminal_reason
        .or(result.rejection_reason)
        .map_or_else(
            || format!("{:?}", result.outcome).to_ascii_lowercase(),
            |reason| {
                format!(
                    "{}:{}",
                    format!("{:?}", result.outcome).to_ascii_lowercase(),
                    format!("{reason:?}").to_ascii_lowercase()
                )
            },
        );
    push_synthetic_attempt(
        &mut state.snapshot,
        SyntheticCacheAttemptProjection {
            operation: Some(result.operation.to_string()),
            attempt: result.attempt.as_ref().map(ToString::to_string),
            purpose: result.purpose,
            provider: bounded_metadata_string(result.identity.provider().to_owned()),
            model: bounded_metadata_string(result.identity.model().as_str().to_owned()),
            cache_identity: Some(result.identity.digest().to_string()),
            usage: summary_usage(&usage),
            counter_provenance: counter_provenance(&usage),
            cost_micro_usd: None,
            cost_provenance: SyntheticCostProvenance::Unknown,
            latency_ms: finished_at.0.saturating_sub(started_at.0),
            status: bounded_metadata_string(status),
        },
    );
}

pub(super) fn push_synthetic_attempt(
    snapshot: &mut CacheControllerSnapshot,
    attempt: SyntheticCacheAttemptProjection,
) {
    if let Some(operation) = attempt.operation.as_deref()
        && let Some(existing) = snapshot
            .synthetic_attempts
            .iter_mut()
            .find(|existing| existing.operation.as_deref() == Some(operation))
    {
        *existing = attempt;
        return;
    }
    if snapshot.synthetic_attempts.len() >= MAX_SYNTHETIC_ATTEMPT_PROJECTIONS {
        snapshot.synthetic_attempts.remove(0);
    }
    snapshot.synthetic_attempts.push(attempt);
}

pub(super) fn counter_provenance(
    usage: &UsageDelta,
) -> BTreeMap<CounterKind, SyntheticCounterProvenance> {
    usage
        .iter()
        .map(|(kind, _)| (kind, SyntheticCounterProvenance::ProviderReported))
        .collect()
}

pub(super) fn summary_usage(usage: &UsageDelta) -> SummaryUsage {
    SummaryUsage {
        input_uncached: usage.get(CounterKind::InputUncached),
        input_cached: usage.get(CounterKind::InputCached),
        cache_write: usage.get(CounterKind::CacheWrite),
        output: usage.get(CounterKind::Output),
        reasoning: usage.get(CounterKind::Reasoning),
        cost_micro_usd: None,
        cost_is_estimate: false,
    }
}

pub(super) fn artifact_sensitivity(
    sensitivity: Sensitivity,
) -> Result<ArtifactSensitivity, &'static str> {
    match sensitivity {
        Sensitivity::Public => Ok(ArtifactSensitivity::Public),
        Sensitivity::Internal | Sensitivity::Sensitive => Ok(ArtifactSensitivity::Sensitive),
        Sensitivity::Secret => Err("summary_sensitivity_invalid"),
    }
}
