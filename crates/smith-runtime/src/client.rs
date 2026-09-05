//! Versioned Smith client protocol projected from canonical Agent Runtime.
//!
//! The canonical event journal and execution state stay in Agent Runtime. This
//! module owns the presentation contract consumed by terminal, headless, GPUI,
//! Forge, and embedded clients. Projection is deterministic and carries only
//! fields already bounded by the canonical event contract.
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::pin::Pin;

use agent_runtime::registry::{Fingerprint, RegistryId, RegistryRevision};
use agent_runtime::runtime::SessionHandle;
use agent_runtime_core::cancel::CancelReason;
use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::content::{InternalTurnSource, UserInput};
use agent_runtime_core::delegation::WorkspacePolicy;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::event::EventEnvelope as CanonicalEvent;
use agent_runtime_core::goal::GoalProjection;
use agent_runtime_core::ids::{
    AttemptId, CacheOperationId, ChildId, EventId, InteractionRequestId, QuestionId, RequestId,
    SessionId, SteerId, ToolCallId, TurnId,
};
use agent_runtime_core::interaction::{InteractionOutcomeKind, InteractionSensitivity};
use agent_runtime_core::manifest::{ActivatedCapability, SegmentId, SegmentKind, SummaryCoverage};
use agent_runtime_core::metadata::Metadata;
use agent_runtime_core::provider::{
    CacheAvailabilityEvidence, CacheIdentity, FinishReason, ModelId, ProviderAttemptPurpose,
};
use agent_runtime_core::steer::{SteerDiscardReason, SteerRejectionReason};
use agent_runtime_core::usage::UsageRecord;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use agent_runtime_core::event::{
    CacheOperationOutcome, CacheOperationReason, CacheState, ChildPhase, ChildRecoveryState,
    CompactionReason, EstimationConfidence, GoalUpdateCause, LimitKind, PlanItemProjection,
    PlanItemStatus, PlanSensitivity, TurnFinish,
};

/// Current Smith client protocol revision.
pub const SMITH_CLIENT_PROTOCOL_VERSION: u32 = 1;
/// Oldest client protocol revision this build accepts.
pub const MIN_SUPPORTED_SMITH_CLIENT_PROTOCOL_VERSION: u32 = 1;

macro_rules! smith_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

smith_id!(SmithSessionId);
smith_id!(SmithTurnId);
smith_id!(SmithSteerId);
smith_id!(SmithToolCallId);
smith_id!(SmithChildId);
smith_id!(SmithInteractionRequestId);

/// Stable approval request projection for Smith clients. Raw arguments,
/// credentials, and unbounded resource values are deliberately absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmithApprovalRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub tool: String,
    pub operation: String,
    pub permissions: Vec<String>,
    pub resource_kind: String,
    pub resource_id: String,
    pub risk: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Smith-client decision for an out-of-band approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmithApprovalDecision {
    AllowOnce,
    Deny,
}

/// Metadata-only Smith client interaction request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmithInteractionRequest {
    pub protocol_version: u32,
    pub request: SmithInteractionRequestId,
    pub call: SmithToolCallId,
    pub question_count: u8,
    pub sensitivity: String,
}

/// Smith-owned usage projection. Its nested counters retain Agent Runtime's
/// stable value types during protocol v1 while the outer contract belongs to
/// Smith.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmithUsage {
    pub record: UsageRecord,
}

/// Smith-owned event envelope. `schema_version` preserves the canonical wire
/// value for existing machine-output compatibility; the Smith protocol version
/// is negotiated with [`SMITH_CLIENT_PROTOCOL_VERSION`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmithEvent {
    pub schema_version: u32,
    pub seq: u64,
    pub id: EventId,
    pub session: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnId>,
    pub timestamp: Timestamp,
    pub payload: SmithEventKind,
    #[serde(default, skip_serializing_if = "Metadata::is_empty")]
    pub metadata: Metadata,
}

impl SmithEvent {
    /// Constructs a Smith event for deterministic client fixtures.
    pub fn new(
        seq: u64,
        id: EventId,
        session: SessionId,
        turn: Option<TurnId>,
        timestamp: Timestamp,
        payload: SmithEventKind,
    ) -> Self {
        Self {
            schema_version: agent_runtime_core::event::SCHEMA_VERSION,
            seq,
            id,
            session,
            turn,
            timestamp,
            payload,
            metadata: Metadata::new(),
        }
    }

    /// Projects one canonical envelope without changing its identity or order.
    pub fn project(canonical: &CanonicalEvent) -> Result<Self, ClientProjectionError> {
        let value = serde_json::to_value(canonical)?;
        Ok(serde_json::from_value(value)?)
    }

    /// Projects a canonical envelope without ever dropping its causal slot.
    ///
    /// A payload added by a newer Agent Runtime becomes `Unknown`; if an
    /// otherwise malformed projection is encountered, the original envelope
    /// identity, sequence, timestamp, turn, and bounded metadata are retained.
    pub fn project_or_unknown(canonical: &CanonicalEvent) -> Self {
        Self::project(canonical).unwrap_or_else(|_| Self {
            schema_version: canonical.schema_version,
            seq: canonical.seq,
            id: canonical.id.clone(),
            session: canonical.session.clone(),
            turn: canonical.turn.clone(),
            timestamp: canonical.timestamp,
            payload: SmithEventKind::Unknown,
            metadata: canonical.metadata.clone(),
        })
    }

    /// Attaches client presentation metadata.
    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Smith-owned presentation vocabulary.
///
/// Variants not useful to presentation clients are intentionally projected to
/// `Unknown`. This includes future canonical additions, so compatible runtime
/// upgrades do not force unchanged clients to deserialize an upstream enum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SmithEventKind {
    SessionStarted,
    TurnStarted,
    TurnSteerCommitted {
        steer: SteerId,
        ordinal: u64,
    },
    TurnSteerDiscarded {
        steer: SteerId,
        ordinal: u64,
        reason: SteerDiscardReason,
    },
    InternalTurnStarted {
        source: InternalTurnSource,
    },
    RegistrySnapshotSealed {
        snapshot: Fingerprint,
        entries: u32,
    },
    ScopedViewDerived {
        snapshot: Fingerprint,
        view: Fingerprint,
        visible_entries: u32,
    },
    ModelProfileResolved {
        provider: String,
        model: ModelId,
        profile: Fingerprint,
    },
    CapabilityRetrievalPerformed {
        resolver_revision: RegistryRevision,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index_revision: Option<RegistryRevision>,
        candidates: Vec<RegistryId>,
    },
    CapabilitiesActivated {
        epoch: u32,
        activation: Vec<ActivatedCapability>,
    },
    ContextPlanned {
        context: Fingerprint,
        cache_plan: Fingerprint,
        segment_count: u32,
        totals: BTreeMap<SegmentKind, u32>,
        #[serde(default)]
        input_tokens: u32,
        input_budget_tokens: u32,
        reserved_tokens: u32,
        confidence: EstimationConfidence,
    },
    ContextCompacted {
        context: Fingerprint,
        reason: CompactionReason,
        evicted: Vec<SegmentId>,
        summaries: Vec<SummaryCoverage>,
        reclaimed_tokens: u32,
    },
    PlanUpdated {
        revision: u64,
        sensitivity: PlanSensitivity,
        counts: BTreeMap<String, u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        items: Option<Vec<PlanItemProjection>>,
    },
    GoalUpdated {
        cause: GoalUpdateCause,
        sensitivity: PlanSensitivity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        goal: Option<GoalProjection>,
    },
    CachePlanChanged {
        cache_plan: Fingerprint,
        preserved_prefix_tokens: u32,
        invalidated_prefix_tokens: u32,
        provider_cache_supported: bool,
    },
    ProviderAttemptStarted {
        request: RequestId,
        attempt: AttemptId,
        index: u32,
        model: String,
    },
    /// A turn is running on an installed coding agent rather than Smith's own
    /// loop. Surfaces carry this so a harness turn is never mistaken for one
    /// Smith executed itself.
    ExternalSessionStarted {
        session: String,
    },
    /// Assistant prose from a harness turn.
    ExternalText {
        text: String,
    },
    /// Reasoning from a harness turn.
    ExternalReasoning {
        text: String,
    },
    /// A tool the installed agent ran itself.
    ///
    /// Smith did not dispatch it, did not approve it, and cannot vouch for it,
    /// so it is deliberately distinct from `ToolCallRequested`.
    ExternalToolInvoked {
        id: String,
        name: String,
    },
    /// The outcome of a tool the installed agent ran itself.
    ExternalToolCompleted {
        id: String,
        ok: bool,
    },
    TextDelta {
        request: RequestId,
        attempt: AttemptId,
        text: String,
    },
    ReasoningDelta {
        request: RequestId,
        attempt: AttemptId,
        text: String,
        redacted: bool,
    },
    ProviderAttemptOutputCommitted {
        request: RequestId,
        attempt: AttemptId,
    },
    ProviderAttemptOutputDiscarded {
        request: RequestId,
        attempt: AttemptId,
    },
    ToolCallRequested {
        call: ToolCallId,
        name: String,
        argument_keys: Vec<String>,
        argument_fingerprint: Fingerprint,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments: Option<Value>,
    },
    InteractionRequested {
        request: InteractionRequestId,
        call: ToolCallId,
        question_count: u8,
        sensitivity: InteractionSensitivity,
    },
    InteractionResolved {
        request: InteractionRequestId,
        call: ToolCallId,
        outcome: InteractionOutcomeKind,
    },
    ToolCallCompleted {
        call: ToolCallId,
        name: String,
        is_error: bool,
    },
    Downgrade {
        capability: String,
        detail: String,
    },
    Usage {
        record: UsageRecord,
    },
    CacheObservation {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<RequestId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt: Option<AttemptId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_plan: Option<Fingerprint>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_identity: Option<CacheIdentity>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        read_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        write_tokens: Option<u64>,
    },
    CacheStateChanged {
        request: RequestId,
        attempt: AttemptId,
        cache_plan: Fingerprint,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_identity: Option<CacheIdentity>,
        state: CacheState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_read_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        observed_read_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        observed_write_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        missed_tokens: Option<u64>,
        confidence: EstimationConfidence,
    },
    CacheOperationPrepared {
        operation: CacheOperationId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<RequestId>,
        identity: CacheIdentity,
        purpose: ProviderAttemptPurpose,
    },
    CacheOperationRejected {
        operation: CacheOperationId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<RequestId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt: Option<AttemptId>,
        identity: CacheIdentity,
        purpose: ProviderAttemptPurpose,
        reason: CacheOperationReason,
    },
    CacheOperationStarted {
        operation: CacheOperationId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<RequestId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt: Option<AttemptId>,
        identity: CacheIdentity,
        purpose: ProviderAttemptPurpose,
    },
    CacheOperationCompleted {
        operation: CacheOperationId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<RequestId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt: Option<AttemptId>,
        identity: CacheIdentity,
        purpose: ProviderAttemptPurpose,
        outcome: CacheOperationOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<CacheOperationReason>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        metrics: BTreeMap<String, u64>,
    },
    CacheAvailabilityEvidenceRecorded {
        evidence: CacheAvailabilityEvidence,
    },
    CacheOperationSuspended {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<RequestId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt: Option<AttemptId>,
        identity: CacheIdentity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operation: Option<CacheOperationId>,
        reason: CacheOperationReason,
    },
    ProviderAttemptFinished {
        attempt: AttemptId,
        finish: FinishReason,
        retryable: bool,
    },
    LimitReached {
        limit: LimitKind,
    },
    Error {
        error: RuntimeError,
    },
    TurnCompleted {
        finish: TurnFinish,
        #[serde(default = "default_true", skip_serializing_if = "is_true")]
        visible_output: bool,
    },
    ChildSpawned {
        child: ChildId,
        workspace: WorkspacePolicy,
        max_turns: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deadline_ms: Option<u64>,
    },
    ChildProgress {
        child: ChildId,
        phase: ChildPhase,
    },
    ChildNeedsInput {
        child: ChildId,
        child_session: SessionId,
        turn: TurnId,
        call: ToolCallId,
        request: InteractionRequestId,
        question_ids: Vec<QuestionId>,
        sensitivity: InteractionSensitivity,
    },
    ChildCompleted {
        child: ChildId,
        result: String,
    },
    ChildStopped {
        child: ChildId,
        reason: CancelReason,
    },
    ChildFailed {
        child: ChildId,
        error: RuntimeError,
    },
    SessionShutdown,
    #[serde(other)]
    Unknown,
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/// Client projection failure. It contains no canonical event body.
#[derive(Debug, thiserror::Error)]
#[error(
    "canonical event could not be projected to Smith client protocol v{SMITH_CLIENT_PROTOCOL_VERSION}"
)]
pub struct ClientProjectionError {
    #[source]
    source: serde_json::Error,
}

impl From<serde_json::Error> for ClientProjectionError {
    fn from(source: serde_json::Error) -> Self {
        Self { source }
    }
}

/// Smith-owned input wrapper. Exact content stays private to submission and
/// canonical runtime history; the client event stream never echoes it.
#[derive(Clone)]
pub struct SmithInput(UserInput);

impl SmithInput {
    /// Plain text input.
    pub fn text(text: impl Into<String>) -> Self {
        Self(UserInput::text(text))
    }

    /// Compatibility adapter for structured Smith host materialization.
    pub fn from_canonical(input: UserInput) -> Self {
        Self(input)
    }

    fn into_canonical(self) -> UserInput {
        self.0
    }
}

impl std::fmt::Debug for SmithInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("SmithInput")
            .field(&"[redacted]")
            .finish()
    }
}

/// Stable receipt for one accepted whole-turn submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnReceipt {
    /// Accepted turn identity.
    pub turn: SmithTurnId,
}

/// Stable receipt for one accepted steering submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteerReceipt {
    pub steer: SmithSteerId,
    pub turn: SmithTurnId,
    pub ordinal: u64,
}

/// Owned steering rejection retaining the caller's exact input.
#[derive(Debug)]
pub struct SteerRejection {
    pub reason: SteerRejectionReason,
    pub input: SmithInput,
}

/// Smith client event stream.
pub type SmithEventStream = Pin<Box<dyn Stream<Item = SmithEvent> + Send>>;

/// Product-level session adapter over one canonical Agent Runtime session.
#[derive(Debug, Clone)]
pub struct SmithSession {
    canonical: SessionHandle,
}

impl SmithSession {
    pub(crate) fn new(canonical: SessionHandle) -> Self {
        Self { canonical }
    }

    /// Stable session identity.
    pub fn id(&self) -> SmithSessionId {
        SmithSessionId::new(self.canonical.id().as_str())
    }

    /// Submits one whole turn through Agent Runtime.
    pub fn submit(&self, input: SmithInput) -> Result<TurnReceipt, RuntimeError> {
        let turn = self.canonical.send(input.into_canonical())?;
        Ok(TurnReceipt {
            turn: SmithTurnId::new(turn.id().as_str()),
        })
    }

    /// Targets additional input to the serving turn.
    pub fn steer(
        &self,
        expected_turn: Option<&SmithTurnId>,
        input: SmithInput,
    ) -> Result<SteerReceipt, SteerRejection> {
        let expected_turn = expected_turn.map(|turn| TurnId::new(turn.as_str()));
        self.canonical
            .steer_current_turn(expected_turn.as_ref(), input.into_canonical())
            .map(|receipt| SteerReceipt {
                steer: SmithSteerId::new(receipt.id.as_str()),
                turn: SmithTurnId::new(receipt.turn.as_str()),
                ordinal: receipt.ordinal,
            })
            .map_err(|rejection| SteerRejection {
                reason: rejection.reason,
                input: SmithInput::from_canonical(rejection.input),
            })
    }

    /// Interrupts the serving turn.
    pub fn cancel(&self, reason: CancelReason) -> Result<(), RuntimeError> {
        self.canonical.interrupt_current_turn(reason)
    }

    /// Subscribes to the Smith-owned deterministic event projection.
    pub fn events(&self) -> SmithEventStream {
        Box::pin(
            self.canonical
                .subscribe()
                .map(|canonical| SmithEvent::project_or_unknown(&canonical)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    use agent_runtime::provider::fake::FakeProvider;
    use agent_runtime::runtime::{RuntimeBuilder, StartSession};
    use agent_runtime_core::approval::AllowAll;
    use agent_runtime_core::event::RuntimeEvent;
    use agent_runtime_testkit::scenarios::fake_model_profile;

    #[test]
    fn canonical_projection_preserves_identity_and_known_payloads() {
        let canonical = CanonicalEvent::new(
            7,
            EventId::new("event-7"),
            SessionId::new("session"),
            Some(TurnId::new("turn")),
            Timestamp(9),
            RuntimeEvent::TextDelta {
                request: RequestId::new("request"),
                attempt: AttemptId::new("attempt"),
                text: "hello".into(),
            },
        );
        let projected = SmithEvent::project(&canonical).unwrap();
        assert_eq!(projected.seq, canonical.seq);
        assert_eq!(projected.id, canonical.id);
        assert!(matches!(
            projected.payload,
            SmithEventKind::TextDelta { ref text, .. } if text == "hello"
        ));
    }

    #[test]
    fn unknown_future_payloads_deserialize_as_a_compatible_event() {
        let value = serde_json::json!({
            "schema_version": 999,
            "seq": 1,
            "id": "event-1",
            "session": "session",
            "timestamp": 0,
            "payload": {"event": "future_runtime_event", "bounded": true}
        });
        let event: SmithEvent = serde_json::from_value(value).unwrap();
        assert_eq!(event.payload, SmithEventKind::Unknown);
    }

    #[test]
    fn durable_replay_projects_identically_to_the_live_envelope() {
        let canonical = CanonicalEvent::new(
            11,
            EventId::new("event-11"),
            SessionId::new("session"),
            Some(TurnId::new("turn")),
            Timestamp(15),
            RuntimeEvent::TurnCompleted {
                finish: TurnFinish::Completed,
                visible_output: true,
            },
        );
        let durable = serde_json::to_vec(&canonical).unwrap();
        let replayed: CanonicalEvent = serde_json::from_slice(&durable).unwrap();
        assert_eq!(
            SmithEvent::project_or_unknown(&canonical),
            SmithEvent::project_or_unknown(&replayed)
        );
    }

    #[tokio::test]
    async fn two_client_sessions_do_not_leak_events() {
        async fn session(reply: &str) -> (SessionHandle, SmithSession) {
            let runtime = RuntimeBuilder::new(ModelId::new("fake"))
                .model_profile(fake_model_profile())
                .provider(Arc::new(FakeProvider::text_reply(reply)))
                .approval(Arc::new(AllowAll))
                .build()
                .unwrap();
            let canonical = runtime
                .start_session(StartSession::new())
                .await
                .expect("a session");
            let client = SmithSession::new(canonical.clone());
            (canonical, client)
        }

        async fn turn(client: &SmithSession, prompt: &str) -> Vec<SmithEvent> {
            let mut events = client.events();
            client
                .submit(SmithInput::text(prompt))
                .expect("accepted turn");
            let mut observed = Vec::new();
            loop {
                let event = tokio::time::timeout(Duration::from_secs(2), events.next())
                    .await
                    .expect("event timeout")
                    .expect("event stream");
                let terminal = matches!(event.payload, SmithEventKind::TurnCompleted { .. });
                observed.push(event);
                if terminal {
                    return observed;
                }
            }
        }

        let (canonical_a, client_a) = session("alpha").await;
        let (canonical_b, client_b) = session("beta").await;
        let (events_a, events_b) = tokio::join!(turn(&client_a, "one"), turn(&client_b, "two"));
        assert_ne!(client_a.id(), client_b.id());
        assert!(
            events_a
                .iter()
                .all(|event| event.session.as_str() == client_a.id().as_str())
        );
        assert!(
            events_b
                .iter()
                .all(|event| event.session.as_str() == client_b.id().as_str())
        );
        assert!(events_a.iter().any(
            |event| matches!(&event.payload, SmithEventKind::TextDelta { text, .. } if text == "alpha")
        ));
        assert!(events_b.iter().any(
            |event| matches!(&event.payload, SmithEventKind::TextDelta { text, .. } if text == "beta")
        ));
        canonical_a.shutdown().await.unwrap();
        canonical_b.shutdown().await.unwrap();
    }
}
