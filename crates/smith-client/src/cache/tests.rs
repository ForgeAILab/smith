use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::ids::{AttemptId, CacheOperationId, EventId, RequestId, SessionId, TurnId};
use agent_runtime_core::provider::{
    CacheAvailabilityEvidence, CacheIdentity, ModelId, PromptCacheControl,
};
use agent_runtime_core::usage::{Provenance, UsageDelta, UsageRecord, UsageSource};
use agent_runtime_registry::Fingerprint;
use smith_runtime::client::SmithEvent as EventEnvelope;
use smith_runtime::client::TurnFinish;

use super::*;

fn envelope(seq: u64, turn: &str, payload: RuntimeEvent) -> EventEnvelope {
    EventEnvelope::new(
        seq,
        EventId::new(format!("event-{seq}")),
        SessionId::new("session"),
        Some(TurnId::new(turn)),
        Timestamp(seq.saturating_mul(60_000)),
        payload,
    )
}

fn usage(seq: u64, turn: &str, input: u64, cached: u64, failed: bool) -> EventEnvelope {
    EventEnvelope::new(
        seq,
        EventId::new(format!("event-{seq}")),
        SessionId::new("session"),
        Some(TurnId::new(turn)),
        Timestamp(seq.saturating_mul(60_000)),
        RuntimeEvent::Usage {
            record: UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance {
                    request: Some(RequestId::new("request")),
                    attempt: Some(AttemptId::new(if failed { "retry" } else { "attempt" })),
                    failed,
                    ..Provenance::default()
                },
                delta: UsageDelta::new()
                    .with(CounterKind::InputUncached, input.saturating_sub(cached))
                    .with(CounterKind::InputCached, cached),
            },
        },
    )
}

fn cache_identity() -> CacheIdentity {
    CacheIdentity::legacy(
        Fingerprint::of("profile"),
        "provider",
        ModelId::new("model"),
        Vec::new(),
        PromptCacheControl::Implicit,
    )
}

mod costs;
mod evidence;
mod retries;
