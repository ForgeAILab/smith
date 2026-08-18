//! Smith's provider-cache evidence projection.
//!
//! The runtime owns the cache contract and emits one attributed
//! `CacheStateChanged` event per provider attempt.  This module deliberately
//! does not re-derive a miss from consecutive usage records: it stores the
//! runtime's expectation, observation, and derived shortfall and only rolls
//! those facts up for presentation.  The same reducer is used by the live
//! TUI, journal replay, and headless output.

use std::collections::{BTreeMap, BTreeSet};

use agent_runtime_core::event::{
    CacheOperationOutcome, CacheOperationReason, CacheState, EstimationConfidence, EventEnvelope,
    RuntimeEvent,
};
use agent_runtime_core::provider::{CacheEvidenceKind, ProviderAttemptPurpose};
use agent_runtime_core::usage::CounterKind;
use serde::Serialize;

/// The fixed cache-miss notice threshold, in tokens.
pub const MISS_NOTICE_TOKENS: u64 = 20_000;
/// The fixed cache-miss notice threshold, in micro-USD.
pub const MISS_NOTICE_COST_MICRO_USD: u128 = 100_000;

/// Smith's presentation state. These are direct projections of canonical
/// runtime states. Smith also uses `Suspended` when an identity switch makes
/// the previous identity's evidence inapplicable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CacheVisibilityState {
    /// The provider cannot honor this plan.
    Unsupported,
    /// No cache evidence was supplied.
    #[default]
    Unknown,
    /// The plan is reusable but has no comparable positive result yet.
    Eligible,
    /// A provider read was observed.
    WarmObserved,
    /// The provider read was below the runtime expectation.
    MissObserved,
    /// The provider explicitly reported expiry for the exact identity.
    Expired,
    /// Runtime suspended maintenance, or an identity switch invalidated the
    /// previous identity's projection.
    Suspended,
}

impl From<CacheState> for CacheVisibilityState {
    fn from(value: CacheState) -> Self {
        match value {
            CacheState::Unsupported => Self::Unsupported,
            CacheState::Unknown => Self::Unknown,
            CacheState::Eligible => Self::Eligible,
            CacheState::WarmObserved => Self::WarmObserved,
            CacheState::MissObserved => Self::MissObserved,
            CacheState::Expired => Self::Expired,
            CacheState::Suspended => Self::Suspended,
        }
    }
}

impl CacheVisibilityState {
    /// Stable wire/display label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
            Self::Eligible => "eligible",
            Self::WarmObserved => "warm_observed",
            Self::MissObserved => "miss_observed",
            Self::Expired => "expired",
            Self::Suspended => "suspended",
        }
    }
}

/// A resolved per-model price used only for derived cache-miss cost.
///
/// Rates are micro-USD per million tokens.  `None` means the catalog did not
/// publish a compatible rate; callers must keep extra cost unknown rather
/// than substituting another model's price.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CachePrice {
    /// Uncached input rate.
    pub input: Option<u64>,
    /// Cache-read rate.
    pub cache_read: Option<u64>,
    /// Cache-write rate.
    pub cache_write: Option<u64>,
}

/// A completed root-turn cache projection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CacheTurnSummary {
    /// The completed root turn identity.
    pub turn: String,
    /// Aggregate canonical state for the turn.
    pub state: CacheVisibilityState,
    /// Exact redaction-safe Runtime cache-identity digest, when all evidence
    /// for the turn was correlated to the same identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_identity: Option<String>,
    /// Expected reusable read tokens, when every cache-evidence-bearing
    /// attempt supplied it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_read_tokens: Option<u64>,
    /// Observed cache-read tokens, preserving explicit zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_read_tokens: Option<u64>,
    /// Observed cache-write tokens, preserving explicit zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_write_tokens: Option<u64>,
    /// Runtime-derived missed tokens, when canonical evidence supplied them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missed_tokens: Option<u64>,
    /// Planner confidence, omitted when attempts disagree or no evidence was
    /// supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<EstimationConfidence>,
    /// Provider-reported cache-read share of prompt input, rounded to a whole
    /// percent.  `Some(0)` is an explicit zero; `None` means evidence is
    /// absent or prompt input could not be attributed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_percent: Option<u8>,
    /// Number of billed attempts with a positive canonical shortfall.
    pub miss_count: u32,
    /// Missed tokens as a separate derived diagnostic, never a usage counter.
    pub rebilled_tokens: u64,
    /// Factual elapsed idle context before the first miss-bearing logical
    /// request, in whole minutes.  This never implies expiry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_minutes: Option<u64>,
    /// Derived extra cost when a compatible price was supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_cost_micro_usd: Option<u128>,
}

/// The latest canonical phase observed for one Runtime cache operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheOperationDisposition {
    /// Runtime preflight accepted the operation.
    Prepared,
    /// Runtime rejected the operation before provider I/O.
    Rejected,
    /// The operation crossed provider admission.
    Started,
    /// The operation reached a terminal result.
    Completed,
    /// Runtime suspended maintenance for the exact identity.
    Suspended,
}

/// Bounded projection of one canonical Runtime cache-operation lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheOperationSummary {
    /// Stable upstream operation identity.
    pub operation: String,
    /// Exact redaction-safe Runtime cache-identity digest.
    pub cache_identity: String,
    /// Typed provider-attempt purpose.
    pub purpose: ProviderAttemptPurpose,
    /// Latest canonical lifecycle phase.
    pub disposition: CacheOperationDisposition,
    /// Logical request, when Runtime allocated one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    /// Provider attempt, when the operation crossed provider admission.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<String>,
    /// Terminal result, when completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CacheOperationOutcome>,
    /// Structured rejection, failure, or suspension reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CacheOperationReason>,
    /// Bounded Runtime metrics; never provider bodies.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metrics: BTreeMap<String, u64>,
}

/// Canonical provider evidence and operation facts accumulated for a session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CacheLifecycleSummary {
    /// Latest exact redaction-safe Runtime cache-identity digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_identity: Option<String>,
    /// Latest typed provider evidence kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<CacheEvidenceKind>,
    /// Provider-declared guarantee boundary, in Runtime clock milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guaranteed_until_ms: Option<u64>,
    /// Explicit resource existence, preserving omitted versus false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_exists: Option<bool>,
    /// Number of canonical operations that crossed provider admission.
    pub maintenance_calls_used: u32,
    /// Latest canonical operation lifecycle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_operation: Option<CacheOperationSummary>,
    /// Latest canonical maintenance-suspension reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suspension_reason: Option<CacheOperationReason>,
}

impl CacheTurnSummary {
    /// The footer's compact cache-hit metric.
    pub fn render_ch(&self) -> String {
        self.cache_read_percent
            .map_or_else(|| "?".to_owned(), |percent| format!("{percent}%"))
    }

    /// Whether this summary crosses Smith's fixed notice threshold.
    pub fn significant(&self) -> bool {
        self.rebilled_tokens >= MISS_NOTICE_TOKENS
            || self
                .extra_cost_micro_usd
                .is_some_and(|cost| cost >= MISS_NOTICE_COST_MICRO_USD)
    }

    /// A bounded factual local notice.  It intentionally says idle, never
    /// expired or likely expired.
    pub fn render_notice(&self) -> String {
        let mut text = String::from("Cache miss");
        if let Some(minutes) = self.idle_minutes.filter(|minutes| *minutes > 0) {
            text.push_str(&format!(" after {minutes}m idle"));
        }
        text.push_str(&format!(
            " · re-billed {}",
            compact_tokens(self.rebilled_tokens)
        ));
        if let (Some(expected), Some(observed)) =
            (self.expected_read_tokens, self.observed_read_tokens)
        {
            text.push_str(&format!(
                " · expected {} · observed {}",
                compact_tokens(expected),
                compact_tokens(observed)
            ));
        }
        if let Some(cost) = self.extra_cost_micro_usd {
            text.push_str(&format!(" · +{} derived", format_usd(cost)));
        }
        text
    }
}

/// One canonical request/attempt/cache-plan identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AttemptKey {
    turn: String,
    request: String,
    attempt: String,
    cache_plan: String,
}

/// The fields Smith accumulates for one billed provider attempt.
#[derive(Debug, Clone, Default)]
struct AttemptProjection {
    turn: String,
    request: String,
    attempt: String,
    cache_plan: String,
    cache_identity: Option<String>,
    input_uncached: u64,
    input_cached: u64,
    cache_write: u64,
    state: Option<CacheVisibilityState>,
    expected: Option<u64>,
    observed_read: Option<u64>,
    observed_write: Option<u64>,
    missed: Option<u64>,
    confidence: Option<EstimationConfidence>,
    read_present: bool,
    write_present: bool,
    usage_attributed: bool,
    idle_ms: Option<u64>,
}

impl AttemptProjection {
    fn input_tokens(&self) -> u64 {
        self.input_uncached
            .saturating_add(self.input_cached)
            .saturating_add(self.cache_write)
    }
}

/// Retry-safe Smith cache projection.
#[derive(Debug, Clone, Default)]
pub struct CacheProjection {
    attempts: BTreeMap<AttemptKey, AttemptProjection>,
    completed: BTreeMap<String, CacheTurnSummary>,
    seen_events: BTreeSet<String>,
    request_starts: BTreeMap<String, u64>,
    seen_requests: BTreeSet<String>,
    last_request_start: Option<u64>,
    current_plan: BTreeMap<String, String>,
    plan_support: BTreeMap<String, bool>,
    active_identity: Option<String>,
    internal_turns: BTreeSet<String>,
    latest_turn: Option<String>,
    session_misses: u32,
    session_rebilled: u64,
    session_observed_read: Option<u64>,
    legacy_read: Option<u64>,
    operations: BTreeMap<String, CacheOperationSummary>,
    lifecycle: CacheLifecycleSummary,
}

impl CacheProjection {
    /// Applies one canonical event exactly once.  The event id/sequence pair
    /// makes live delivery followed by journal replay idempotent.
    pub fn apply(&mut self, envelope: &EventEnvelope) {
        let event_key = format!("{}:{}", envelope.seq, envelope.id);
        if !self.seen_events.insert(event_key) {
            return;
        }
        let turn = envelope
            .turn
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        match &envelope.payload {
            RuntimeEvent::ModelProfileResolved {
                provider, model, ..
            } => {
                let identity = format!("{provider}/{model}");
                if self.active_identity.as_deref() != Some(identity.as_str()) {
                    self.suspend();
                    self.active_identity = Some(identity);
                }
            }
            RuntimeEvent::InternalTurnStarted { .. } => {
                if !turn.is_empty() {
                    self.internal_turns.insert(turn);
                }
            }
            RuntimeEvent::ContextPlanned { cache_plan, .. }
            | RuntimeEvent::CachePlanChanged { cache_plan, .. } => {
                if !turn.is_empty() {
                    self.current_plan
                        .insert(turn.clone(), cache_plan.to_string());
                }
                if let RuntimeEvent::CachePlanChanged {
                    provider_cache_supported,
                    cache_plan,
                    ..
                } = &envelope.payload
                {
                    self.plan_support
                        .insert(cache_plan.to_string(), *provider_cache_supported);
                }
            }
            RuntimeEvent::ProviderAttemptStarted {
                request, attempt, ..
            } => {
                let request = request.to_string();
                let started = envelope.timestamp.as_millis();
                let idle_ms = if self.seen_requests.insert(request.clone()) {
                    let idle = self
                        .last_request_start
                        .map(|previous| started.saturating_sub(previous));
                    self.last_request_start = Some(started);
                    self.request_starts.insert(request.clone(), started);
                    idle
                } else {
                    // Retries share one logical request. Preserve the factual
                    // idle interval on every attempt so a miss observed only
                    // on a later retry does not lose that request context.
                    self.attempts
                        .values()
                        .find(|entry| entry.request == request)
                        .and_then(|entry| entry.idle_ms)
                };
                let plan = self.current_plan.get(&turn).cloned().unwrap_or_default();
                let key = AttemptKey {
                    turn: turn.clone(),
                    request: request.clone(),
                    attempt: attempt.to_string(),
                    cache_plan: plan,
                };
                let entry = self
                    .attempts
                    .entry(key)
                    .or_insert_with(|| AttemptProjection {
                        turn,
                        request,
                        attempt: attempt.to_string(),
                        ..AttemptProjection::default()
                    });
                entry.idle_ms = idle_ms;
            }
            RuntimeEvent::Usage { record } => {
                let Some(request) = record.provenance.request.as_ref() else {
                    return;
                };
                let Some(attempt) = record.provenance.attempt.as_ref() else {
                    return;
                };
                let request = request.to_string();
                let attempt_id = attempt.to_string();
                let key = self.find_or_create(&turn, &request, &attempt_id, None);
                let entry = self.attempts.get_mut(&key).expect("inserted attempt");
                entry.usage_attributed = true;
                entry.input_uncached = entry
                    .input_uncached
                    .saturating_add(record.delta.get(CounterKind::InputUncached));
                entry.input_cached = entry
                    .input_cached
                    .saturating_add(record.delta.get(CounterKind::InputCached));
                entry.cache_write = entry
                    .cache_write
                    .saturating_add(record.delta.get(CounterKind::CacheWrite));
                // A failed ProviderAttempt usage record remains in the same
                // billed attempt projection; no committed-output filter is
                // applied here.
            }
            RuntimeEvent::CacheObservation {
                request,
                attempt,
                cache_plan,
                cache_identity,
                read_tokens,
                write_tokens,
            } => {
                let Some(request) = request.as_ref() else {
                    if let Some(read) = read_tokens {
                        self.legacy_read =
                            Some(self.legacy_read.unwrap_or_default().saturating_add(*read));
                    }
                    return;
                };
                let Some(attempt) = attempt.as_ref() else {
                    return;
                };
                let request = request.to_string();
                let attempt_id = attempt.to_string();
                let plan = cache_plan.as_ref().map(ToString::to_string);
                let key = self.find_or_create(&turn, &request, &attempt_id, plan.as_deref());
                let entry = self.attempts.get_mut(&key).expect("inserted attempt");
                if let Some(identity) = cache_identity {
                    let digest = identity.digest().to_string();
                    entry.cache_identity = Some(digest.clone());
                    self.lifecycle.cache_identity = Some(digest);
                }
                if let Some(read) = read_tokens {
                    entry.observed_read = Some(*read);
                    entry.read_present = true;
                }
                if let Some(write) = write_tokens {
                    entry.observed_write = Some(*write);
                    entry.write_present = true;
                }
            }
            RuntimeEvent::CacheStateChanged {
                request,
                attempt,
                cache_plan,
                cache_identity,
                state,
                expected_read_tokens,
                observed_read_tokens,
                observed_write_tokens,
                missed_tokens,
                confidence,
            } => {
                let request = request.to_string();
                let attempt_id = attempt.to_string();
                let plan = cache_plan.to_string();
                let key = self.find_or_create(&turn, &request, &attempt_id, Some(&plan));
                let entry = self.attempts.get_mut(&key).expect("inserted attempt");
                entry.cache_plan = plan;
                if let Some(identity) = cache_identity {
                    let digest = identity.digest().to_string();
                    entry.cache_identity = Some(digest.clone());
                    self.lifecycle.cache_identity = Some(digest);
                }
                entry.state = Some((*state).into());
                entry.expected = *expected_read_tokens;
                entry.observed_read = *observed_read_tokens;
                entry.observed_write = *observed_write_tokens;
                entry.missed = *missed_tokens;
                entry.confidence = Some(*confidence);
                entry.read_present = observed_read_tokens.is_some();
                entry.write_present = observed_write_tokens.is_some();
            }
            RuntimeEvent::CacheOperationPrepared {
                operation,
                request,
                identity,
                purpose,
            } => {
                self.publish_operation(CacheOperationSummary {
                    operation: operation.to_string(),
                    cache_identity: identity.digest().to_string(),
                    purpose: *purpose,
                    disposition: CacheOperationDisposition::Prepared,
                    request: request.as_ref().map(ToString::to_string),
                    attempt: None,
                    outcome: None,
                    reason: None,
                    metrics: BTreeMap::new(),
                });
            }
            RuntimeEvent::CacheOperationRejected {
                operation,
                request,
                attempt,
                identity,
                purpose,
                reason,
            } => {
                self.publish_operation(CacheOperationSummary {
                    operation: operation.to_string(),
                    cache_identity: identity.digest().to_string(),
                    purpose: *purpose,
                    disposition: CacheOperationDisposition::Rejected,
                    request: request.as_ref().map(ToString::to_string),
                    attempt: attempt.as_ref().map(ToString::to_string),
                    outcome: Some(CacheOperationOutcome::Rejected),
                    reason: Some(*reason),
                    metrics: BTreeMap::new(),
                });
            }
            RuntimeEvent::CacheOperationStarted {
                operation,
                request,
                attempt,
                identity,
                purpose,
            } => {
                self.lifecycle.maintenance_calls_used =
                    self.lifecycle.maintenance_calls_used.saturating_add(1);
                self.publish_operation(CacheOperationSummary {
                    operation: operation.to_string(),
                    cache_identity: identity.digest().to_string(),
                    purpose: *purpose,
                    disposition: CacheOperationDisposition::Started,
                    request: request.as_ref().map(ToString::to_string),
                    attempt: attempt.as_ref().map(ToString::to_string),
                    outcome: None,
                    reason: None,
                    metrics: BTreeMap::new(),
                });
            }
            RuntimeEvent::CacheOperationCompleted {
                operation,
                request,
                attempt,
                identity,
                purpose,
                outcome,
                reason,
                metrics,
            } => {
                self.publish_operation(CacheOperationSummary {
                    operation: operation.to_string(),
                    cache_identity: identity.digest().to_string(),
                    purpose: *purpose,
                    disposition: CacheOperationDisposition::Completed,
                    request: request.as_ref().map(ToString::to_string),
                    attempt: attempt.as_ref().map(ToString::to_string),
                    outcome: Some(*outcome),
                    reason: *reason,
                    metrics: metrics.clone(),
                });
            }
            RuntimeEvent::CacheAvailabilityEvidenceRecorded { evidence } => {
                self.lifecycle.cache_identity = Some(evidence.identity.digest().to_string());
                self.lifecycle.evidence = Some(evidence.kind);
                if evidence.suspends_maintenance() {
                    self.lifecycle.guaranteed_until_ms = None;
                } else if let Some(guaranteed_until) = evidence.guaranteed_until {
                    self.lifecycle.guaranteed_until_ms = Some(guaranteed_until.as_millis());
                }
                if evidence.exists.is_some() {
                    self.lifecycle.resource_exists = evidence.exists;
                }
            }
            RuntimeEvent::CacheOperationSuspended {
                identity,
                operation,
                reason,
                ..
            } => {
                let digest = identity.digest().to_string();
                self.lifecycle.cache_identity = Some(digest.clone());
                self.lifecycle.suspension_reason = Some(*reason);
                if let Some(operation) = operation {
                    let key = operation.to_string();
                    if let Some(mut summary) = self.operations.remove(&key) {
                        summary.cache_identity = digest;
                        summary.disposition = CacheOperationDisposition::Suspended;
                        summary.reason = Some(*reason);
                        self.publish_operation(summary);
                    }
                }
            }
            RuntimeEvent::ProviderAttemptFinished { attempt, .. } => {
                let attempt_id = attempt.to_string();
                for entry in self
                    .attempts
                    .values_mut()
                    .filter(|entry| entry.turn == turn && entry.attempt == attempt_id)
                {
                    // The usage record itself is the billed-attempt authority;
                    // finish disposition is intentionally not used to hide it.
                    entry.usage_attributed |= entry.input_tokens() > 0;
                }
            }
            RuntimeEvent::TurnCompleted { .. } if !turn.is_empty() => {
                let is_internal = self.internal_turns.contains(&turn);
                self.finish_turn(&turn, !is_internal);
            }
            _ => {}
        }
    }

    /// Restores a sequence of canonical journal events using the same reducer
    /// as live delivery.
    pub fn replay<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = EventEnvelope>,
    {
        for event in events {
            self.apply(&event);
        }
    }

    /// Returns the latest completed root turn, if any.
    pub fn latest_completed(&self) -> Option<&CacheTurnSummary> {
        self.latest_turn
            .as_deref()
            .and_then(|turn| self.completed.get(turn))
    }

    /// Returns a completed turn by identity.
    pub fn completed_turn(&self, turn: &str) -> Option<&CacheTurnSummary> {
        self.completed.get(turn)
    }

    /// Positive legacy cache-read evidence, if an old unattributed journal
    /// entry supplied it.  It cannot produce expected, miss, idle, or CH
    /// values.
    pub fn legacy_read(&self) -> Option<u64> {
        self.legacy_read
    }

    /// Cumulative miss count over all attributed billed attempts.
    pub fn session_miss_count(&self) -> u32 {
        self.session_misses
    }

    /// Cumulative derived missed tokens over all attributed billed attempts.
    pub fn session_rebilled_tokens(&self) -> u64 {
        self.session_rebilled
    }

    /// Cumulative attributed provider cache-read evidence. Explicit zero is
    /// represented as `Some(0)` after a completed turn.
    pub fn session_observed_read(&self) -> Option<u64> {
        self.session_observed_read.or(self.legacy_read)
    }

    /// Latest canonical provider evidence and cache-operation lifecycle.
    pub fn lifecycle(&self) -> &CacheLifecycleSummary {
        &self.lifecycle
    }

    /// Suspends the latest identity after a provider/model switch. Historical
    /// miss totals remain session diagnostics, but the old CH value cannot be
    /// presented as belonging to the new cache identity.
    pub fn suspend(&mut self) {
        if let Some(turn) = self.latest_turn.as_deref()
            && let Some(summary) = self.completed.get_mut(turn)
        {
            summary.state = CacheVisibilityState::Suspended;
            summary.cache_identity = None;
            summary.expected_read_tokens = None;
            summary.observed_read_tokens = None;
            summary.observed_write_tokens = None;
            summary.missed_tokens = None;
            summary.cache_read_percent = None;
            summary.miss_count = 0;
            summary.rebilled_tokens = 0;
            summary.idle_minutes = None;
            summary.extra_cost_micro_usd = None;
        }
        self.session_observed_read = None;
        self.legacy_read = None;
        self.current_plan.clear();
    }

    /// Computes a known derived extra cost for a summary without changing its
    /// canonical fields.
    pub fn with_price(&self, summary: &CacheTurnSummary, price: CachePrice) -> CacheTurnSummary {
        if summary.state == CacheVisibilityState::Suspended {
            return summary.clone();
        }
        let mut result = summary.clone();
        result.extra_cost_micro_usd = self.extra_cost_for_turn(&summary.turn, price);
        result
    }

    /// Computes cumulative derived extra cost for all completed attempts.
    pub fn session_extra_cost(&self, price: CachePrice) -> Option<u128> {
        let mut total = 0_u128;
        let mut saw_miss = false;
        for entry in self.attempts.values() {
            if entry.missed.unwrap_or_default() == 0 {
                continue;
            }
            saw_miss = true;
            total = total.checked_add(extra_cost(entry, price)?)?;
        }
        saw_miss.then_some(total)
    }

    fn extra_cost_for_turn(&self, turn: &str, price: CachePrice) -> Option<u128> {
        let mut total = 0_u128;
        let mut saw_miss = false;
        for entry in self.attempts.values().filter(|entry| entry.turn == turn) {
            if entry.missed.unwrap_or_default() == 0 {
                continue;
            }
            saw_miss = true;
            total = total.checked_add(extra_cost(entry, price)?)?;
        }
        saw_miss.then_some(total)
    }

    fn publish_operation(&mut self, summary: CacheOperationSummary) {
        self.lifecycle.cache_identity = Some(summary.cache_identity.clone());
        if matches!(
            summary.disposition,
            CacheOperationDisposition::Prepared | CacheOperationDisposition::Started
        ) {
            self.operations
                .insert(summary.operation.clone(), summary.clone());
        } else {
            self.operations.remove(&summary.operation);
        }
        self.lifecycle.last_operation = Some(summary);
    }

    fn find_or_create(
        &mut self,
        turn: &str,
        request: &str,
        attempt: &str,
        cache_plan: Option<&str>,
    ) -> AttemptKey {
        if let Some(plan) = cache_plan {
            let exact = AttemptKey {
                turn: turn.to_owned(),
                request: request.to_owned(),
                attempt: attempt.to_owned(),
                cache_plan: plan.to_owned(),
            };
            if !self.attempts.contains_key(&exact)
                && let Some(previous) = self
                    .attempts
                    .keys()
                    .find(|key| {
                        key.turn == turn && key.request == request && key.attempt == attempt
                    })
                    .cloned()
                && let Some(mut entry) = self.attempts.remove(&previous)
            {
                entry.cache_plan = plan.to_owned();
                self.attempts.insert(exact.clone(), entry);
                return exact;
            }
            self.attempts
                .entry(exact.clone())
                .or_insert_with(|| AttemptProjection {
                    turn: turn.to_owned(),
                    request: request.to_owned(),
                    attempt: attempt.to_owned(),
                    cache_plan: plan.to_owned(),
                    ..AttemptProjection::default()
                });
            return exact;
        }
        if let Some(existing) = self
            .attempts
            .keys()
            .find(|key| key.turn == turn && key.request == request && key.attempt == attempt)
        {
            return existing.clone();
        }
        let key = AttemptKey {
            turn: turn.to_owned(),
            request: request.to_owned(),
            attempt: attempt.to_owned(),
            cache_plan: self.current_plan.get(turn).cloned().unwrap_or_default(),
        };
        let idle_ms = self.request_starts.get(request).and_then(|start| {
            self.last_request_start
                .map(|last| last.saturating_sub(*start))
        });
        self.attempts
            .entry(key.clone())
            .or_insert_with(|| AttemptProjection {
                turn: turn.to_owned(),
                request: request.to_owned(),
                attempt: attempt.to_owned(),
                cache_plan: key.cache_plan.clone(),
                idle_ms,
                ..AttemptProjection::default()
            });
        key
    }

    fn finish_turn(&mut self, turn: &str, update_latest_root: bool) {
        let entries: Vec<&AttemptProjection> = self
            .attempts
            .values()
            .filter(|entry| entry.turn == turn)
            .collect();
        if entries.is_empty() {
            self.completed.insert(
                turn.to_owned(),
                CacheTurnSummary {
                    turn: turn.to_owned(),
                    ..CacheTurnSummary::default()
                },
            );
            if update_latest_root {
                self.latest_turn = Some(turn.to_owned());
            }
            return;
        }

        // CH is a usage-derived denominator, so only attempts with
        // attributed disjoint usage participate in it. Canonical state and
        // missed-token diagnostics remain visible even if an attempt's usage
        // record was unavailable or output-only.
        let billed: Vec<&AttemptProjection> = entries
            .iter()
            .copied()
            .filter(|entry| entry.usage_attributed)
            .collect();

        // An attempt can finish before the provider reports any response or
        // cache field. Such a transport-only attempt is not cache evidence
        // and must not erase a later retry's observed values or confidence.
        let evidence: Vec<&AttemptProjection> = entries
            .iter()
            .copied()
            .filter(|entry| {
                entry.state.is_some()
                    || entry.read_present
                    || entry.write_present
                    || entry.expected.is_some()
                    || entry.missed.is_some()
                    || entry.confidence.is_some()
            })
            .collect();

        let expected = (!evidence.is_empty()
            && evidence.iter().all(|entry| entry.expected.is_some()))
        .then(|| {
            evidence
                .iter()
                .filter_map(|entry| entry.expected)
                .fold(0_u64, u64::saturating_add)
        });
        let observed_read = (!evidence.is_empty()
            && evidence.iter().all(|entry| entry.read_present))
        .then(|| {
            evidence
                .iter()
                .map(|entry| entry.observed_read)
                .sum::<Option<u64>>()
        })
        .flatten();
        let observed_write = (!evidence.is_empty()
            && evidence.iter().all(|entry| entry.write_present))
        .then(|| {
            evidence
                .iter()
                .map(|entry| entry.observed_write)
                .sum::<Option<u64>>()
        })
        .flatten();
        let missed = entries
            .iter()
            .filter_map(|entry| entry.missed)
            .reduce(u64::saturating_add);
        let prompt_input: u64 = billed.iter().map(|entry| entry.input_tokens()).sum();
        let billed_read_present =
            !billed.is_empty() && billed.iter().all(|entry| entry.read_present);
        // Use the disjoint provider-reported InputCached counter as the
        // numerator. A raw observation can exceed the bounded input ledger;
        // it is evidence for presence, not a second billing counter.
        let cached_input: u64 = billed.iter().map(|entry| entry.input_cached).sum();
        let cache_read_percent = (prompt_input > 0 && billed_read_present).then(|| {
            let read: u64 = cached_input;
            u8::try_from(
                read.saturating_mul(100)
                    .saturating_add(prompt_input / 2)
                    .checked_div(prompt_input)
                    .unwrap_or_default()
                    .min(100),
            )
            .unwrap_or(100)
        });
        let state = aggregate_state(&evidence);
        let cache_identity = evidence.first().and_then(|first| {
            let identity = first.cache_identity.as_ref()?;
            evidence
                .iter()
                .all(|entry| entry.cache_identity.as_ref() == Some(identity))
                .then(|| identity.clone())
        });
        let confidence = evidence
            .iter()
            .map(|entry| entry.confidence)
            .collect::<Option<Vec<_>>>()
            .and_then(|values| {
                let first = values.first().copied();
                first.filter(|first| values.iter().all(|value| value == first))
            });
        let miss_count = entries
            .iter()
            .filter(|entry| entry.missed.is_some_and(|missed| missed > 0))
            .count() as u32;
        let rebilled_tokens = missed.unwrap_or_default();
        self.session_misses = self.session_misses.saturating_add(miss_count);
        self.session_rebilled = self.session_rebilled.saturating_add(rebilled_tokens);
        let miss_requests: BTreeSet<&str> = entries
            .iter()
            .filter(|entry| entry.missed.is_some_and(|missed| missed > 0))
            .map(|entry| entry.request.as_str())
            .collect();
        let idle_minutes = (miss_requests.len() == 1)
            .then(|| {
                entries
                    .iter()
                    .filter(|entry| {
                        entry.missed.is_some_and(|missed| missed > 0)
                            && miss_requests.contains(entry.request.as_str())
                    })
                    .filter_map(|entry| entry.idle_ms)
                    .map(|millis| millis / 60_000)
                    .next()
            })
            .flatten();
        let known_read = evidence
            .iter()
            .filter(|entry| entry.read_present)
            .filter_map(|entry| entry.observed_read)
            .reduce(u64::saturating_add);
        if let Some(read) = known_read {
            self.session_observed_read = Some(
                self.session_observed_read
                    .unwrap_or_default()
                    .saturating_add(read),
            );
        }
        self.completed.insert(
            turn.to_owned(),
            CacheTurnSummary {
                turn: turn.to_owned(),
                state,
                cache_identity,
                expected_read_tokens: expected,
                observed_read_tokens: observed_read,
                observed_write_tokens: observed_write,
                missed_tokens: missed,
                confidence,
                cache_read_percent,
                miss_count,
                rebilled_tokens,
                idle_minutes,
                extra_cost_micro_usd: None,
            },
        );
        if update_latest_root {
            self.latest_turn = Some(turn.to_owned());
        }
    }
}

fn aggregate_state(entries: &[&AttemptProjection]) -> CacheVisibilityState {
    if entries
        .iter()
        .any(|entry| entry.state == Some(CacheVisibilityState::Suspended))
    {
        CacheVisibilityState::Suspended
    } else if entries
        .iter()
        .any(|entry| entry.state == Some(CacheVisibilityState::Expired))
    {
        CacheVisibilityState::Expired
    } else if entries
        .iter()
        .any(|entry| entry.state == Some(CacheVisibilityState::MissObserved))
    {
        CacheVisibilityState::MissObserved
    } else if entries
        .iter()
        .any(|entry| entry.state == Some(CacheVisibilityState::WarmObserved))
    {
        CacheVisibilityState::WarmObserved
    } else if entries
        .iter()
        .any(|entry| entry.state == Some(CacheVisibilityState::Eligible))
    {
        CacheVisibilityState::Eligible
    } else if entries
        .iter()
        .any(|entry| entry.state == Some(CacheVisibilityState::Unsupported))
    {
        CacheVisibilityState::Unsupported
    } else {
        CacheVisibilityState::Unknown
    }
}

fn extra_cost(entry: &AttemptProjection, price: CachePrice) -> Option<u128> {
    let missed = entry.missed?;
    let paid_tokens = entry.input_uncached.saturating_add(entry.cache_write);
    if missed == 0 {
        return Some(0);
    }
    if paid_tokens == 0 || !entry.usage_attributed || entry.confidence.is_none() {
        return None;
    }
    let input_rate = u128::from(price.input.unwrap_or_default());
    let write_rate = u128::from(price.cache_write.unwrap_or_default());
    if entry.input_uncached > 0 && price.input.is_none() {
        return None;
    }
    if entry.cache_write > 0 && price.cache_write.is_none() {
        return None;
    }
    let read_rate = u128::from(price.cache_read?);
    let paid_numerator = u128::from(entry.input_uncached)
        .saturating_mul(input_rate)
        .saturating_add(u128::from(entry.cache_write).saturating_mul(write_rate));
    let paid_cost = u128::from(missed)
        .saturating_mul(paid_numerator)
        .checked_div(u128::from(paid_tokens))?
        .checked_div(1_000_000)?;
    let cache_cost = u128::from(missed)
        .saturating_mul(read_rate)
        .checked_div(1_000_000)?;
    Some(paid_cost.saturating_sub(cache_cost))
}

fn compact_tokens(value: u64) -> String {
    match value {
        0..1_000 => value.to_string(),
        1_000..1_000_000 => {
            let tenths = value / 100;
            if tenths.is_multiple_of(10) {
                format!("{}k", tenths / 10)
            } else {
                format!("{}.{}k", tenths / 10, tenths % 10)
            }
        }
        _ => {
            let tenths = value / 100_000;
            if tenths.is_multiple_of(10) {
                format!("{}M", tenths / 10)
            } else {
                format!("{}.{}M", tenths / 10, tenths % 10)
            }
        }
    }
}

fn format_usd(micro_usd: u128) -> String {
    let dollars = micro_usd / 1_000_000;
    let thousandths = (micro_usd / 1_000) % 1_000;
    if micro_usd > 0 && dollars == 0 && thousandths == 0 {
        return format!("$0.{:06}", micro_usd % 1_000_000);
    }
    format!("${dollars}.{thousandths:03}")
}

#[cfg(test)]
mod tests {
    use agent_runtime_core::clock::Timestamp;
    use agent_runtime_core::event::{EventEnvelope, TurnFinish};
    use agent_runtime_core::ids::{
        AttemptId, CacheOperationId, EventId, RequestId, SessionId, TurnId,
    };
    use agent_runtime_core::provider::{
        CacheAvailabilityEvidence, CacheIdentity, ModelId, PromptCacheControl,
    };
    use agent_runtime_core::usage::{Provenance, UsageDelta, UsageRecord, UsageSource};
    use agent_runtime_registry::Fingerprint;

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

    #[test]
    fn exact_identity_and_expiry_remain_distinct_in_the_turn_projection() {
        let identity = cache_identity();
        let expected_digest = identity.digest().to_string();
        let mut projection = CacheProjection::default();
        projection.apply(&envelope(
            1,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: Some(identity),
                state: CacheState::Expired,
                expected_read_tokens: Some(100),
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(100),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            2,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));

        let summary = projection.latest_completed().expect("summary");
        assert_eq!(summary.state, CacheVisibilityState::Expired);
        assert_eq!(
            summary.cache_identity.as_deref(),
            Some(expected_digest.as_str())
        );
    }

    #[test]
    fn canonical_operation_lifecycle_and_guarantee_project_once() {
        let identity = cache_identity();
        let operation = CacheOperationId::new("cache-operation");
        let request = RequestId::new("request");
        let attempt = AttemptId::new("attempt");
        let mut projection = CacheProjection::default();

        projection.apply(&envelope(
            1,
            "turn",
            RuntimeEvent::CacheOperationPrepared {
                operation: operation.clone(),
                request: Some(request.clone()),
                identity: identity.clone(),
                purpose: ProviderAttemptPurpose::CacheKeepalive,
            },
        ));
        let started = envelope(
            2,
            "turn",
            RuntimeEvent::CacheOperationStarted {
                operation: operation.clone(),
                request: Some(request.clone()),
                attempt: Some(attempt.clone()),
                identity: identity.clone(),
                purpose: ProviderAttemptPurpose::CacheKeepalive,
            },
        );
        projection.apply(&started);
        projection.apply(&started);
        projection.apply(&envelope(
            3,
            "turn",
            RuntimeEvent::CacheAvailabilityEvidenceRecorded {
                evidence: CacheAvailabilityEvidence::stream(
                    identity.clone(),
                    request.clone(),
                    attempt.clone(),
                    0,
                    Some(10),
                    Some(0),
                )
                .with_guaranteed_until(Timestamp(9_000)),
            },
        ));
        projection.apply(&envelope(
            4,
            "turn",
            RuntimeEvent::CacheOperationCompleted {
                operation,
                request: Some(request),
                attempt: Some(attempt),
                identity: identity.clone(),
                purpose: ProviderAttemptPurpose::CacheKeepalive,
                outcome: CacheOperationOutcome::Completed,
                reason: None,
                metrics: BTreeMap::from([("latency_ms".to_owned(), 12)]),
            },
        ));

        let lifecycle = projection.lifecycle();
        assert_eq!(lifecycle.maintenance_calls_used, 1);
        assert_eq!(lifecycle.guaranteed_until_ms, Some(9_000));
        assert_eq!(
            lifecycle.cache_identity.as_deref(),
            Some(identity.digest().as_str())
        );
        let operation = lifecycle.last_operation.as_ref().expect("operation");
        assert_eq!(operation.disposition, CacheOperationDisposition::Completed);
        assert_eq!(operation.outcome, Some(CacheOperationOutcome::Completed));
        assert_eq!(operation.metrics.get("latency_ms"), Some(&12));
    }

    #[test]
    fn explicit_zero_is_zero_not_unknown() {
        let mut projection = CacheProjection::default();
        projection.apply(&envelope(
            1,
            "turn",
            RuntimeEvent::ProviderAttemptStarted {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                index: 0,
                model: "model".to_owned(),
            },
        ));
        projection.apply(&usage(2, "turn", 100, 0, false));
        projection.apply(&envelope(
            3,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::MissObserved,
                expected_read_tokens: Some(100),
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(100),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            4,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        let summary = projection.latest_completed().expect("summary");
        assert_eq!(summary.cache_read_percent, Some(0));
        assert_eq!(summary.missed_tokens, Some(100));
        assert_eq!(summary.rebilled_tokens, 100);
    }

    #[test]
    fn first_eligible_zero_does_not_become_a_miss() {
        let mut projection = CacheProjection::default();
        projection.apply(&usage(1, "turn", 100, 0, false));
        projection.apply(&envelope(
            2,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::Eligible,
                expected_read_tokens: None,
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(0),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            3,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        let summary = projection.latest_completed().expect("summary");
        assert_eq!(summary.state, CacheVisibilityState::Eligible);
        assert_eq!(summary.cache_read_percent, Some(0));
        assert_eq!(summary.missed_tokens, Some(0));
        assert_eq!(summary.miss_count, 0);
        assert_eq!(summary.rebilled_tokens, 0);
    }

    #[test]
    fn a_full_hit_uses_provider_cached_input_for_ch() {
        let mut projection = CacheProjection::default();
        projection.apply(&usage(1, "turn", 100, 100, false));
        projection.apply(&envelope(
            2,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::WarmObserved,
                expected_read_tokens: Some(100),
                observed_read_tokens: Some(100),
                observed_write_tokens: None,
                missed_tokens: Some(0),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            3,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        assert_eq!(
            projection
                .latest_completed()
                .expect("summary")
                .cache_read_percent,
            Some(100)
        );
    }

    #[test]
    fn a_write_only_observation_keeps_read_ch_at_zero() {
        let mut projection = CacheProjection::default();
        projection.apply(&envelope(
            1,
            "turn",
            RuntimeEvent::Usage {
                record: UsageRecord {
                    source: UsageSource::ProviderAttempt,
                    provenance: Provenance {
                        request: Some(RequestId::new("request")),
                        attempt: Some(AttemptId::new("attempt")),
                        ..Provenance::default()
                    },
                    delta: UsageDelta::new().with(CounterKind::CacheWrite, 100),
                },
            },
        ));
        projection.apply(&envelope(
            2,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::WarmObserved,
                expected_read_tokens: None,
                observed_read_tokens: Some(0),
                observed_write_tokens: Some(100),
                missed_tokens: Some(0),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            3,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        let summary = projection.latest_completed().expect("summary");
        assert_eq!(summary.observed_write_tokens, Some(100));
        assert_eq!(summary.cache_read_percent, Some(0));
    }

    #[test]
    fn switching_identity_suspends_the_prior_root_projection_without_new_misses() {
        let mut projection = CacheProjection::default();
        projection.apply(&envelope(
            1,
            "identity",
            RuntimeEvent::ModelProfileResolved {
                provider: "old-provider".to_owned(),
                model: ModelId::new("old-model"),
                profile: Fingerprint::of("profile-a"),
            },
        ));
        projection.apply(&usage(2, "turn", 100, 0, false));
        projection.apply(&envelope(
            3,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                cache_plan: Fingerprint::of("plan-a"),
                cache_identity: None,
                state: CacheState::MissObserved,
                expected_read_tokens: Some(100),
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(100),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            4,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        projection.suspend();
        let suspended = projection.latest_completed().expect("suspended summary");
        assert_eq!(suspended.state, CacheVisibilityState::Suspended);
        assert_eq!(suspended.cache_read_percent, None);
        assert_eq!(suspended.missed_tokens, None);
        assert_eq!(suspended.rebilled_tokens, 0);
        assert_eq!(projection.session_miss_count(), 1);
        assert_eq!(projection.session_rebilled_tokens(), 100);
        assert_eq!(
            projection
                .with_price(
                    suspended,
                    CachePrice {
                        input: Some(100_000),
                        cache_read: Some(50_000),
                        cache_write: Some(100_000),
                    },
                )
                .extra_cost_micro_usd,
            None
        );

        projection.apply(&envelope(
            5,
            "new-turn",
            RuntimeEvent::ModelProfileResolved {
                provider: "new-provider".to_owned(),
                model: ModelId::new("new-model"),
                profile: Fingerprint::of("profile-b"),
            },
        ));
        assert_eq!(
            projection
                .latest_completed()
                .expect("suspended prior summary")
                .state,
            CacheVisibilityState::Suspended
        );
        assert_eq!(projection.session_miss_count(), 1);
    }

    #[test]
    fn absent_observation_is_unknown_and_never_a_miss() {
        let mut projection = CacheProjection::default();
        projection.apply(&usage(1, "turn", 100, 0, false));
        projection.apply(&envelope(
            2,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        let summary = projection.latest_completed().expect("summary");
        assert_eq!(summary.cache_read_percent, None);
        assert_eq!(summary.missed_tokens, None);
        assert_eq!(summary.state, CacheVisibilityState::Unknown);
    }

    #[test]
    fn canonical_miss_without_usage_keeps_miss_diagnostics_but_ch_is_unknown() {
        let mut projection = CacheProjection::default();
        projection.apply(&envelope(
            1,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::MissObserved,
                expected_read_tokens: Some(105_000),
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(105_000),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            2,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        let summary = projection.latest_completed().expect("summary");
        assert_eq!(summary.cache_read_percent, None);
        assert_eq!(summary.missed_tokens, Some(105_000));
        assert_eq!(summary.rebilled_tokens, 105_000);
        assert_eq!(summary.extra_cost_micro_usd, None);
    }

    #[test]
    fn ch_uses_bounded_cached_usage_not_an_unbounded_raw_observation() {
        let mut projection = CacheProjection::default();
        projection.apply(&usage(1, "turn", 100, 20, false));
        projection.apply(&envelope(
            2,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::WarmObserved,
                expected_read_tokens: Some(20),
                observed_read_tokens: Some(200),
                observed_write_tokens: None,
                missed_tokens: Some(0),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            3,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        assert_eq!(
            projection
                .latest_completed()
                .expect("summary")
                .cache_read_percent,
            Some(20)
        );
    }

    #[test]
    fn failed_retry_remains_in_billed_denominator_and_rebilling() {
        let mut projection = CacheProjection::default();
        projection.apply(&usage(1, "turn", 100, 0, true));
        projection.apply(&usage(2, "turn", 100, 0, false));
        for (seq, attempt) in [(3, "retry"), (4, "attempt")] {
            projection.apply(&envelope(
                seq,
                "turn",
                RuntimeEvent::CacheStateChanged {
                    request: RequestId::new("request"),
                    attempt: AttemptId::new(attempt),
                    cache_plan: Fingerprint::of("plan"),
                    cache_identity: None,
                    state: CacheState::MissObserved,
                    expected_read_tokens: Some(100),
                    observed_read_tokens: Some(0),
                    observed_write_tokens: None,
                    missed_tokens: Some(100),
                    confidence: EstimationConfidence::Exact,
                },
            ));
        }
        projection.apply(&envelope(
            5,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Failed,
                visible_output: true,
            },
        ));
        let summary = projection.latest_completed().expect("summary");
        assert_eq!(summary.cache_read_percent, Some(0));
        assert_eq!(summary.miss_count, 2);
        assert_eq!(summary.rebilled_tokens, 200);
    }

    #[test]
    fn a_pre_response_retry_does_not_erase_later_cache_evidence() {
        let mut projection = CacheProjection::default();
        for (seq, attempt, index) in [(1, "transport", 0), (2, "served", 1)] {
            projection.apply(&envelope(
                seq,
                "turn",
                RuntimeEvent::ProviderAttemptStarted {
                    request: RequestId::new("request"),
                    attempt: AttemptId::new(attempt),
                    index,
                    model: "model".to_owned(),
                },
            ));
        }
        projection.apply(&envelope(
            3,
            "turn",
            RuntimeEvent::Usage {
                record: UsageRecord {
                    source: UsageSource::ProviderAttempt,
                    provenance: Provenance {
                        request: Some(RequestId::new("request")),
                        attempt: Some(AttemptId::new("served")),
                        ..Provenance::default()
                    },
                    delta: UsageDelta::new()
                        .with(CounterKind::InputUncached, 20)
                        .with(CounterKind::InputCached, 80),
                },
            },
        ));
        projection.apply(&envelope(
            4,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("served"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::WarmObserved,
                expected_read_tokens: Some(80),
                observed_read_tokens: Some(80),
                observed_write_tokens: None,
                missed_tokens: Some(0),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            5,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));

        let summary = projection.latest_completed().expect("summary");
        assert_eq!(summary.state, CacheVisibilityState::WarmObserved);
        assert_eq!(summary.expected_read_tokens, Some(80));
        assert_eq!(summary.observed_read_tokens, Some(80));
        assert_eq!(summary.confidence, Some(EstimationConfidence::Exact));
        assert_eq!(summary.cache_read_percent, Some(80));
        assert_eq!(projection.session_observed_read(), Some(80));
    }

    #[test]
    fn a_retry_inherits_its_logical_requests_idle_context() {
        let mut projection = CacheProjection::default();
        projection.apply(&envelope(
            1,
            "previous",
            RuntimeEvent::ProviderAttemptStarted {
                request: RequestId::new("previous-request"),
                attempt: AttemptId::new("previous-attempt"),
                index: 0,
                model: "model".to_owned(),
            },
        ));
        for (seq, attempt, index) in [(12, "first-attempt", 0), (13, "retry-attempt", 1)] {
            projection.apply(&envelope(
                seq,
                "turn",
                RuntimeEvent::ProviderAttemptStarted {
                    request: RequestId::new("retry-request"),
                    attempt: AttemptId::new(attempt),
                    index,
                    model: "model".to_owned(),
                },
            ));
        }
        projection.apply(&envelope(
            14,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("retry-request"),
                attempt: AttemptId::new("retry-attempt"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::MissObserved,
                expected_read_tokens: Some(20_000),
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(20_000),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            15,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));

        assert_eq!(
            projection.latest_completed().expect("summary").idle_minutes,
            Some(11)
        );
    }

    #[test]
    fn duplicate_replay_does_not_double_count() {
        let mut projection = CacheProjection::default();
        let events = vec![
            usage(1, "turn", 100, 0, false),
            envelope(
                2,
                "turn",
                RuntimeEvent::CacheStateChanged {
                    request: RequestId::new("request"),
                    attempt: AttemptId::new("attempt"),
                    cache_plan: Fingerprint::of("plan"),
                    cache_identity: None,
                    state: CacheState::MissObserved,
                    expected_read_tokens: Some(100),
                    observed_read_tokens: Some(0),
                    observed_write_tokens: None,
                    missed_tokens: Some(100),
                    confidence: EstimationConfidence::Exact,
                },
            ),
            envelope(
                3,
                "turn",
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ),
        ];
        projection.replay(events.clone());
        projection.replay(events);
        assert_eq!(projection.session_miss_count(), 1);
        assert_eq!(projection.session_rebilled_tokens(), 100);
    }

    #[test]
    fn internal_turns_do_not_replace_the_latest_root_turn() {
        let mut projection = CacheProjection::default();
        projection.internal_turns.insert("internal".to_owned());
        projection.apply(&usage(1, "internal", 100, 0, false));
        projection.apply(&envelope(
            2,
            "internal",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::MissObserved,
                expected_read_tokens: Some(100),
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(100),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            3,
            "internal",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: false,
            },
        ));
        assert!(projection.latest_completed().is_none());
        assert_eq!(projection.session_miss_count(), 1);

        projection.apply(&usage(4, "root", 100, 100, false));
        projection.apply(&envelope(
            5,
            "root",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::WarmObserved,
                expected_read_tokens: Some(100),
                observed_read_tokens: Some(100),
                observed_write_tokens: None,
                missed_tokens: Some(0),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            6,
            "root",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        let summary = projection.latest_completed().expect("root summary");
        assert_eq!(summary.turn, "root");
        assert_eq!(summary.cache_read_percent, Some(100));
        assert_eq!(projection.session_miss_count(), 1);
    }

    #[test]
    fn idle_context_is_only_rendered_for_one_miss_bearing_request() {
        let mut projection = CacheProjection::default();
        projection.apply(&envelope(
            1,
            "previous",
            RuntimeEvent::ProviderAttemptStarted {
                request: RequestId::new("previous-request"),
                attempt: AttemptId::new("attempt-previous"),
                index: 0,
                model: "model".to_owned(),
            },
        ));
        projection.apply(&envelope(
            2,
            "previous",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        projection.apply(&envelope(
            11,
            "one-request",
            RuntimeEvent::ProviderAttemptStarted {
                request: RequestId::new("one-request"),
                attempt: AttemptId::new("attempt-one"),
                index: 0,
                model: "model".to_owned(),
            },
        ));
        projection.apply(&envelope(
            12,
            "one-request",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("one-request"),
                attempt: AttemptId::new("attempt-one"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::MissObserved,
                expected_read_tokens: Some(20_000),
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(20_000),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            13,
            "one-request",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        assert_eq!(
            projection
                .latest_completed()
                .expect("one-request summary")
                .idle_minutes,
            Some(10)
        );

        let mut multiple = CacheProjection::default();
        for (seq, request, attempt) in [
            (1, "first-request", "attempt-first"),
            (2, "second-request", "attempt-second"),
        ] {
            multiple.apply(&envelope(
                seq,
                "multiple",
                RuntimeEvent::ProviderAttemptStarted {
                    request: RequestId::new(request),
                    attempt: AttemptId::new(attempt),
                    index: 0,
                    model: "model".to_owned(),
                },
            ));
            multiple.apply(&envelope(
                seq + 2,
                "multiple",
                RuntimeEvent::CacheStateChanged {
                    request: RequestId::new(request),
                    attempt: AttemptId::new(attempt),
                    cache_plan: Fingerprint::of("plan"),
                    cache_identity: None,
                    state: CacheState::MissObserved,
                    expected_read_tokens: Some(20_000),
                    observed_read_tokens: Some(0),
                    observed_write_tokens: None,
                    missed_tokens: Some(20_000),
                    confidence: EstimationConfidence::Exact,
                },
            ));
        }
        multiple.apply(&envelope(
            5,
            "multiple",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        assert_eq!(
            multiple
                .latest_completed()
                .expect("multiple summary")
                .idle_minutes,
            None
        );
    }

    #[test]
    fn known_misses_are_kept_when_another_attributed_attempt_has_unknown_usage() {
        let mut projection = CacheProjection::default();
        projection.apply(&usage(1, "turn", 100, 0, false));
        for (seq, attempt, missed) in [(2, "attempt", 100), (3, "unknown", 200)] {
            projection.apply(&envelope(
                seq,
                "turn",
                RuntimeEvent::CacheStateChanged {
                    request: RequestId::new("request"),
                    attempt: AttemptId::new(attempt),
                    cache_plan: Fingerprint::of("plan"),
                    cache_identity: None,
                    state: CacheState::MissObserved,
                    expected_read_tokens: Some(missed),
                    observed_read_tokens: Some(0),
                    observed_write_tokens: None,
                    missed_tokens: Some(missed),
                    confidence: EstimationConfidence::Exact,
                },
            ));
        }
        projection.apply(&envelope(
            4,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));

        let summary = projection.latest_completed().expect("summary");
        assert_eq!(summary.cache_read_percent, Some(0));
        assert_eq!(summary.missed_tokens, Some(300));
        assert_eq!(summary.miss_count, 2);
        assert_eq!(summary.rebilled_tokens, 300);
    }

    #[test]
    fn legacy_unattributed_zero_is_preserved_without_creating_cache_diagnostics() {
        let mut projection = CacheProjection::default();
        projection.apply(&envelope(
            1,
            "turn",
            RuntimeEvent::CacheObservation {
                request: None,
                attempt: None,
                cache_plan: None,
                cache_identity: None,
                read_tokens: Some(0),
                write_tokens: None,
            },
        ));
        projection.apply(&envelope(
            2,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));

        assert_eq!(projection.legacy_read(), Some(0));
        assert_eq!(projection.session_observed_read(), Some(0));
        let summary = projection.latest_completed().expect("summary");
        assert_eq!(summary.cache_read_percent, None);
        assert_eq!(summary.missed_tokens, None);
        assert_eq!(summary.miss_count, 0);
    }

    #[test]
    fn missing_rates_are_required_only_for_positive_paid_categories() {
        let mut projection = CacheProjection::default();
        projection.apply(&usage(1, "turn", 100, 0, false));
        projection.apply(&envelope(
            2,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::MissObserved,
                expected_read_tokens: Some(100),
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(100),
                confidence: EstimationConfidence::Exact,
            },
        ));
        projection.apply(&envelope(
            3,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        let summary = projection.latest_completed().expect("summary");
        let priced = projection.with_price(
            summary,
            CachePrice {
                input: Some(100_000),
                cache_read: Some(50_000),
                cache_write: None,
            },
        );
        assert_eq!(priced.extra_cost_micro_usd, Some(5));

        let mut write_projection = CacheProjection::default();
        write_projection.apply(&envelope(
            1,
            "turn",
            RuntimeEvent::Usage {
                record: UsageRecord {
                    source: UsageSource::ProviderAttempt,
                    provenance: Provenance {
                        request: Some(RequestId::new("request")),
                        attempt: Some(AttemptId::new("attempt")),
                        ..Provenance::default()
                    },
                    delta: UsageDelta::new().with(CounterKind::CacheWrite, 100),
                },
            },
        ));
        write_projection.apply(&envelope(
            2,
            "turn",
            RuntimeEvent::CacheStateChanged {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                cache_plan: Fingerprint::of("plan"),
                cache_identity: None,
                state: CacheState::MissObserved,
                expected_read_tokens: Some(100),
                observed_read_tokens: Some(0),
                observed_write_tokens: None,
                missed_tokens: Some(100),
                confidence: EstimationConfidence::Exact,
            },
        ));
        write_projection.apply(&envelope(
            3,
            "turn",
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        ));
        let summary = write_projection.latest_completed().expect("summary");
        assert_eq!(
            write_projection
                .with_price(
                    summary,
                    CachePrice {
                        input: Some(100_000),
                        cache_read: Some(50_000),
                        cache_write: None,
                    },
                )
                .extra_cost_micro_usd,
            None
        );
    }
}
