//! Host-independent accumulation of one headless run's runtime events.

use std::collections::BTreeSet;
use std::fmt::Display;

use agent_runtime_core::goal::{GoalProjection, GoalStatus};
use agent_runtime_core::ids::TurnId;
use agent_runtime_core::interaction::InteractionOutcomeKind;
use agent_runtime_core::usage::UsageDelta;
use smith_client::cache::CacheProjection;
use smith_host::InteractionRequired;
use smith_runtime::client::{
    SmithEvent as EventEnvelope, SmithEventKind as RuntimeEvent, TurnFinish,
};

use super::output::{
    ActivationOutput, CacheOutput, LifecycleOutput, is_synthetic_usage, plan_output,
};
use super::run_flow::{finish_waits_for_required_follow_up, observe_sequence};

pub(super) struct HeadlessFold {
    pub(super) turn_id: TurnId,
    pub(super) cache_projection: CacheProjection,
    pub(super) finish: Option<TurnFinish>,
    pub(super) turn_usage: UsageDelta,
    pub(super) cache: Option<CacheOutput>,
    pub(super) last_error: Option<String>,
    pub(super) last_attempt_error: Option<String>,
    pub(super) last_sequence: Option<u64>,
    pub(super) sequence_error: Option<String>,
    pub(super) pending_interaction: Option<InteractionRequired>,
    pub(super) event_interaction_required: Option<InteractionRequired>,
    pub(super) lifecycle: LifecycleOutput,
    pub(super) goal_continuation_turns: u32,
    pub(super) active_goal_turns: BTreeSet<String>,
    pub(super) active_child_completion_turns: BTreeSet<String>,
    /// Child-outcome cursor revision of the last delivery turn seen on the
    /// stream. Runtime advances the cursor when it admits a delivery turn,
    /// before that turn's start event arrives here, so a live cursor ahead of
    /// this value means a delivery is still on its way.
    pub(super) observed_delivery_revision: u64,
}

impl HeadlessFold {
    pub(super) fn new(
        turn_id: TurnId,
        cache_projection: CacheProjection,
        initial_activation: Option<ActivationOutput>,
        delivery_revision: u64,
    ) -> Self {
        Self {
            turn_id,
            cache_projection,
            finish: None,
            turn_usage: UsageDelta::new(),
            cache: None,
            last_error: None,
            last_attempt_error: None,
            last_sequence: None,
            sequence_error: None,
            pending_interaction: None,
            event_interaction_required: None,
            lifecycle: LifecycleOutput {
                activation: initial_activation,
                ..LifecycleOutput::default()
            },
            goal_continuation_turns: 0_u32,
            active_goal_turns: BTreeSet::new(),
            active_child_completion_turns: BTreeSet::new(),
            observed_delivery_revision: delivery_revision,
        }
    }

    pub(super) fn apply(&mut self, event: &EventEnvelope) {
        self.cache_projection.apply(event);
        observe_sequence(&mut self.last_sequence, event.seq, &mut self.sequence_error);
        if matches!(
            &event.payload,
            RuntimeEvent::InternalTurnStarted { source } if source.kind == "goal"
        ) {
            self.goal_continuation_turns = self.goal_continuation_turns.saturating_add(1);
            if let Some(turn) = &event.turn {
                self.active_goal_turns.insert(turn.as_str().to_owned());
            }
        }
        if let RuntimeEvent::InternalTurnStarted { source } = &event.payload
            && source.kind == "delegation.child-completion"
        {
            // The source id is `cursor-<revision>`; each admission advances
            // the revision by one.
            let revision = source
                .id
                .strip_prefix("cursor-")
                .and_then(|revision| revision.parse::<u64>().ok())
                .unwrap_or(self.observed_delivery_revision.saturating_add(1));
            self.observed_delivery_revision = self.observed_delivery_revision.max(revision);
            if let Some(turn) = &event.turn {
                self.active_child_completion_turns
                    .insert(turn.as_str().to_owned());
            }
        }
        let belongs_to_turn = event.turn.as_ref() == Some(&self.turn_id);
        let belongs_to_goal_turn = event
            .turn
            .as_ref()
            .is_some_and(|turn| self.active_goal_turns.contains(turn.as_str()));
        if belongs_to_turn {
            match &event.payload {
                RuntimeEvent::Usage { record } if !is_synthetic_usage(record) => {
                    self.turn_usage.merge(&record.delta)
                }
                RuntimeEvent::CachePlanChanged {
                    preserved_prefix_tokens,
                    invalidated_prefix_tokens,
                    provider_cache_supported,
                    ..
                } => {
                    self.cache = Some(CacheOutput {
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
                RuntimeEvent::Error { error } => self.last_error = Some(error.to_string()),
                // An attempt that ended in an error reports its own cause
                // even when the retry loop then spends the attempt budget
                // without a terminal error event; it is the only account of
                // why a `limit_reached` turn produced nothing.
                RuntimeEvent::ProviderAttemptFinished {
                    error: Some(error), ..
                } => self.last_attempt_error = Some(error.to_string()),
                RuntimeEvent::ProviderAttemptOutputCommitted { .. } => {
                    self.lifecycle.attempts_committed =
                        self.lifecycle.attempts_committed.saturating_add(1);
                }
                RuntimeEvent::ProviderAttemptOutputDiscarded { .. } => {
                    self.lifecycle.attempts_discarded =
                        self.lifecycle.attempts_discarded.saturating_add(1);
                }
                RuntimeEvent::CapabilitiesActivated { epoch, activation } => {
                    self.lifecycle.activation = Some(ActivationOutput {
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
                    self.lifecycle.plan = Some(plan_output(
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
                        let required = self
                            .pending_interaction
                            .take()
                            .filter(|pending| pending.request_id == request.as_str())
                            .unwrap_or_else(|| InteractionRequired {
                                request_id: request.as_str().to_owned(),
                                question_count: 0,
                            });
                        self.event_interaction_required.get_or_insert(required);
                    }
                    self.finish = Some(completed.clone());
                }
                RuntimeEvent::InteractionRequested {
                    request,
                    question_count,
                    ..
                } => {
                    self.pending_interaction = Some(InteractionRequired {
                        request_id: request.as_str().to_owned(),
                        question_count: usize::from(*question_count),
                    });
                }
                RuntimeEvent::InteractionResolved {
                    request,
                    outcome: InteractionOutcomeKind::Unavailable,
                    ..
                } => {
                    if let Some(required) = self.pending_interaction.take()
                        && required.request_id == request.as_str()
                    {
                        self.event_interaction_required.get_or_insert(required);
                    }
                }
                _ => {}
            }
        }
        if belongs_to_goal_turn {
            match &event.payload {
                RuntimeEvent::Error { error } => self.last_error = Some(error.to_string()),
                RuntimeEvent::InteractionRequested {
                    request,
                    question_count,
                    ..
                } => {
                    self.pending_interaction = Some(InteractionRequired {
                        request_id: request.as_str().to_owned(),
                        question_count: usize::from(*question_count),
                    });
                }
                RuntimeEvent::InteractionResolved {
                    request,
                    outcome: InteractionOutcomeKind::Unavailable,
                    ..
                } => {
                    if let Some(required) = self.pending_interaction.take()
                        && required.request_id == request.as_str()
                    {
                        self.event_interaction_required.get_or_insert(required);
                    }
                }
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::NeedsInput { request },
                    ..
                } => {
                    let required = self
                        .pending_interaction
                        .take()
                        .filter(|pending| pending.request_id == request.as_str())
                        .unwrap_or_else(|| InteractionRequired {
                            request_id: request.as_str().to_owned(),
                            question_count: 0,
                        });
                    self.event_interaction_required.get_or_insert(required);
                }
                _ => {}
            }
        }

        // Completed-turn removals now precede the driver's stream write. A
        // write failure returns immediately, so no later code observes them.
        if belongs_to_goal_turn
            && matches!(&event.payload, RuntimeEvent::TurnCompleted { .. })
            && let Some(turn) = &event.turn
        {
            self.active_goal_turns.remove(turn.as_str());
        }
        if matches!(&event.payload, RuntimeEvent::TurnCompleted { .. })
            && let Some(turn) = &event.turn
        {
            self.active_child_completion_turns.remove(turn.as_str());
        }
    }

    /// The driver calls this after writing the event. Lazy callbacks preserve
    /// exactly when the host's goal and required child work are consulted.
    pub(super) fn exit<E: Display>(
        &mut self,
        goal: impl FnOnce() -> Result<Option<GoalProjection>, E>,
        child_work: impl FnOnce() -> bool,
    ) -> bool {
        if self
            .finish
            .as_ref()
            .is_some_and(|finish| !finish_waits_for_required_follow_up(finish))
        {
            return true;
        }
        if self.finish.is_some() {
            match goal() {
                Ok(Some(goal)) if goal.status == GoalStatus::Active => {}
                Ok(_)
                    if self.active_goal_turns.is_empty()
                        && self.active_child_completion_turns.is_empty()
                        && !child_work() =>
                {
                    return true;
                }
                Ok(_) => {}
                Err(error) => {
                    self.sequence_error.get_or_insert_with(|| {
                        format!("persistent goal state became unavailable: {error}")
                    });
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use agent_runtime::registry::RegistryRevision;
    use agent_runtime_core::cancel::CancelReason;
    use agent_runtime_core::clock::Timestamp;
    use agent_runtime_core::content::{InternalTurnSensitivity, InternalTurnSource};
    use agent_runtime_core::goal::{GoalTokenUsage, GoalUsageProvenance};
    use agent_runtime_core::ids::{EventId, GoalId, InteractionRequestId, SessionId};
    use agent_runtime_core::provider::ProviderAttemptPurpose;
    use agent_runtime_core::usage::{CounterKind, Provenance, UsageRecord, UsageSource};
    use smith_runtime::client::LimitKind;

    use super::{CacheProjection, TurnId, UsageDelta, observe_sequence};
    use super::{
        EventEnvelope, GoalProjection, GoalStatus, HeadlessFold, RuntimeEvent, TurnFinish,
    };

    fn fold() -> HeadlessFold {
        HeadlessFold::new(TurnId::new("root"), CacheProjection::default(), None, 0)
    }

    fn event(seq: u64, turn: &str, payload: RuntimeEvent) -> EventEnvelope {
        EventEnvelope::new(
            seq,
            EventId::new(format!("event-{seq}")),
            SessionId::new("session"),
            Some(TurnId::new(turn)),
            Timestamp::ZERO,
            payload,
        )
    }

    fn completed(seq: u64, turn: &str, finish: TurnFinish) -> EventEnvelope {
        event(
            seq,
            turn,
            RuntimeEvent::TurnCompleted {
                finish,
                visible_output: true,
            },
        )
    }

    fn no_goal() -> Result<Option<GoalProjection>, &'static str> {
        Ok(None)
    }

    fn internal_source(kind: &str, id: &str) -> InternalTurnSource {
        InternalTurnSource {
            kind: kind.to_owned(),
            id: id.to_owned(),
            revision: RegistryRevision::new("fold-test-v1"),
            sensitivity: InternalTurnSensitivity::Public,
            goal: None,
        }
    }

    fn active_goal() -> Result<Option<GoalProjection>, &'static str> {
        Ok(Some(GoalProjection {
            id: GoalId::new("goal"),
            generation: 1,
            objective: "Complete the goal".to_owned(),
            status: GoalStatus::Active,
            token_budget: None,
            usage: GoalTokenUsage {
                charged_tokens: None,
                provenance: GoalUsageProvenance::Unknown,
                active_elapsed_ms: 0,
            },
            created_at: Timestamp::ZERO,
            updated_at: Timestamp::ZERO,
            stopped_reason: None,
        }))
    }

    #[test]
    fn synthetic_usage_is_excluded_while_real_usage_merges() {
        let mut fold = fold();
        let records = [
            UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance::default(),
                delta: UsageDelta::new()
                    .with(CounterKind::InputUncached, 100)
                    .with(CounterKind::Output, 2),
            },
            UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance {
                    attempt_purpose: Some(ProviderAttemptPurpose::CacheKeepalive),
                    ..Provenance::default()
                },
                delta: UsageDelta::new()
                    .with(CounterKind::InputCached, 80)
                    .with(CounterKind::Output, 10),
            },
            UsageRecord {
                source: UsageSource::SemanticSummary,
                provenance: Provenance {
                    attempt_purpose: Some(ProviderAttemptPurpose::IdleCompaction),
                    ..Provenance::default()
                },
                delta: UsageDelta::new()
                    .with(CounterKind::InputUncached, 20)
                    .with(CounterKind::Output, 10),
            },
            UsageRecord {
                source: UsageSource::ProviderAttempt,
                provenance: Provenance::default(),
                delta: UsageDelta::new()
                    .with(CounterKind::InputUncached, 50)
                    .with(CounterKind::InputCached, 5)
                    .with(CounterKind::Output, 3),
            },
        ];
        for (seq, record) in (1_u64..).zip(records) {
            fold.apply(&event(seq, "root", RuntimeEvent::Usage { record }));
        }
        assert_eq!(
            fold.turn_usage,
            UsageDelta::new()
                .with(CounterKind::InputUncached, 150)
                .with(CounterKind::InputCached, 5)
                .with(CounterKind::Output, 5),
        );

        fold.apply(&event(
            5,
            "other",
            RuntimeEvent::Usage {
                record: UsageRecord {
                    source: UsageSource::ProviderAttempt,
                    provenance: Provenance::default(),
                    delta: UsageDelta::new().with(CounterKind::InputUncached, 500),
                },
            },
        ));
        assert_eq!(fold.turn_usage.get(CounterKind::InputUncached), 150);
    }

    #[test]
    fn a_goal_continuation_keeps_the_run_open_until_its_turn_completes() {
        let mut fold = fold();
        assert!(!fold.exit::<&'static str>(
            || panic!("goal lookup before root completion"),
            || panic!("child work before root completion")
        ));
        fold.apply(&completed(1, "root", TurnFinish::Completed));
        assert!(!fold.exit(active_goal, || panic!("child work while goal is active")));

        fold.apply(&event(
            2,
            "goal-turn",
            RuntimeEvent::InternalTurnStarted {
                source: internal_source("goal", "goal"),
            },
        ));
        assert_eq!(fold.goal_continuation_turns, 1);
        assert!(fold.active_goal_turns.contains("goal-turn"));
        assert!(!fold.exit(no_goal, || panic!("child work while a goal turn is active")));

        fold.apply(&completed(3, "goal-turn", TurnFinish::Completed));
        assert!(fold.active_goal_turns.is_empty());
        assert_eq!(fold.goal_continuation_turns, 1);
        assert!(fold.exit(no_goal, || false));
    }

    #[test]
    fn a_child_completion_after_root_finish_waits_for_delivery() {
        let mut fold = fold();
        fold.apply(&completed(1, "root", TurnFinish::Completed));
        assert!(!fold.exit(no_goal, || true));

        fold.apply(&event(
            2,
            "child-completion-turn",
            RuntimeEvent::InternalTurnStarted {
                source: internal_source("delegation.child-completion", "cursor-3"),
            },
        ));
        assert_eq!(fold.observed_delivery_revision, 3);
        assert!(
            fold.active_child_completion_turns
                .contains("child-completion-turn")
        );
        assert!(!fold.exit(no_goal, || panic!(
            "child work while delivery turn is active"
        )));

        fold.apply(&completed(
            3,
            "child-completion-turn",
            TurnFinish::Completed,
        ));
        assert!(fold.active_child_completion_turns.is_empty());
        assert!(fold.exit(no_goal, || false));
    }

    #[test]
    fn a_delivery_source_without_a_cursor_revision_counts_one_admission() {
        let mut fold = fold();
        fold.apply(&event(
            1,
            "child-completion-turn",
            RuntimeEvent::InternalTurnStarted {
                source: internal_source("delegation.child-completion", "unexpected"),
            },
        ));
        assert_eq!(fold.observed_delivery_revision, 1);
    }

    #[test]
    fn a_sequence_gap_uses_the_existing_sequence_error() {
        let mut fold = fold();
        let mut last = None;
        let mut expected = None;
        for seq in [4, 5, 8, 10] {
            fold.apply(&event(seq, "root", RuntimeEvent::TurnStarted));
            observe_sequence(&mut last, seq, &mut expected);
        }
        assert_eq!(fold.sequence_error, expected);
        assert_eq!(
            fold.sequence_error.as_deref(),
            Some("runtime event stream lost events between sequence 5 and 8")
        );
        assert_eq!(fold.last_sequence, Some(10));
    }

    #[test]
    fn a_finish_without_required_follow_up_does_not_consult_the_host() {
        for finish in [
            TurnFinish::Failed,
            TurnFinish::Cancelled {
                reason: CancelReason::UserRequested,
            },
            TurnFinish::LimitReached {
                limit: LimitKind::Output,
            },
            TurnFinish::NeedsInput {
                request: InteractionRequestId::new("question"),
            },
        ] {
            let mut fold = fold();
            fold.apply(&completed(1, "root", finish));
            let goal_called = Cell::new(false);
            let child_work_called = Cell::new(false);
            assert!(fold.exit(
                || {
                    goal_called.set(true);
                    no_goal()
                },
                || {
                    child_work_called.set(true);
                    true
                }
            ));
            assert!(!goal_called.get());
            assert!(!child_work_called.get());
        }
    }

    #[test]
    fn unavailable_goal_state_exits_and_preserves_the_first_sequence_error() {
        let mut fold = fold();
        fold.apply(&completed(1, "root", TurnFinish::Completed));
        assert!(fold.exit(
            || Err::<Option<GoalProjection>, _>("unavailable"),
            || { panic!("child work after goal lookup failed") }
        ));
        assert_eq!(
            fold.sequence_error.as_deref(),
            Some("persistent goal state became unavailable: unavailable")
        );

        let mut fold = self::fold();
        fold.apply(&event(1, "root", RuntimeEvent::TurnStarted));
        fold.apply(&completed(3, "root", TurnFinish::Completed));
        assert!(fold.exit(|| Err::<Option<GoalProjection>, _>("unavailable"), || false));
        assert_eq!(
            fold.sequence_error.as_deref(),
            Some("runtime event stream lost events between sequence 1 and 3")
        );
    }
}
