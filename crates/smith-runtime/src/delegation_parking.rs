//! Pure Smith-side lifecycle policy for parked parent sessions.
//!
//! Agent Runtime owns the protected child-outcome ledger, cursor, and
//! conditional internal-turn admission.  This module deliberately keeps only
//! the Smith product projection around those primitives: whether the parent is
//! serving or parked, which direct children still have live work, and whether
//! one already-delivered terminal batch is eligible to wake an idle parent.
//! It does not create messages or runtime events.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use agent_runtime_core::error::{ErrorKind, RuntimeError};

/// The runtime hard bound for one model-facing child wait.
pub const HARD_MAX_WAIT_TIMEOUT_MS: u64 = 30_000;
/// The default bounded child wait.
pub const DEFAULT_WAIT_TIMEOUT_MS: u64 = 5_000;

/// Smith's host-narrowed child wait policy.
///
/// The actual wait is validated again by Agent Runtime's coordinator.  Keeping
/// this value at the Smith boundary lets configuration callers pass a
/// resolved policy without creating a second waiting implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelegationWaitPolicy {
    default_timeout_ms: u64,
    max_timeout_ms: u64,
}

impl DelegationWaitPolicy {
    /// Creates a bounded policy.  A zero default is an immediate status check;
    /// the maximum is always positive and never above Runtime's hard cap.
    pub fn new(default_timeout_ms: u64, max_timeout_ms: u64) -> Result<Self, RuntimeError> {
        if max_timeout_ms == 0 || max_timeout_ms > HARD_MAX_WAIT_TIMEOUT_MS {
            return Err(RuntimeError::new(
                ErrorKind::Config,
                "delegation wait maximum must be between 1 and 30,000 milliseconds",
            ));
        }
        if default_timeout_ms > HARD_MAX_WAIT_TIMEOUT_MS || default_timeout_ms > max_timeout_ms {
            return Err(RuntimeError::new(
                ErrorKind::Config,
                "delegation wait default must not exceed its maximum or 30,000 milliseconds",
            ));
        }
        Ok(Self {
            default_timeout_ms,
            max_timeout_ms,
        })
    }

    /// The standard Smith policy (5 seconds default, 30 seconds maximum).
    pub const fn default_policy() -> Self {
        Self {
            default_timeout_ms: DEFAULT_WAIT_TIMEOUT_MS,
            max_timeout_ms: HARD_MAX_WAIT_TIMEOUT_MS,
        }
    }

    /// Resolved default timeout in milliseconds.
    pub const fn default_timeout_ms(self) -> u64 {
        self.default_timeout_ms
    }

    /// Resolved maximum timeout in milliseconds.
    pub const fn max_timeout_ms(self) -> u64 {
        self.max_timeout_ms
    }

    /// Runtime-compatible default duration.
    pub const fn default_timeout(self) -> Duration {
        Duration::from_millis(self.default_timeout_ms)
    }

    /// Runtime-compatible maximum duration.
    pub const fn max_timeout(self) -> Duration {
        Duration::from_millis(self.max_timeout_ms)
    }

    /// Validates one model-facing `timeout_ms` before a coordinator wait.
    pub fn resolve_timeout(self, timeout_ms: Option<u64>) -> Result<Duration, RuntimeError> {
        let timeout_ms = timeout_ms.unwrap_or(self.default_timeout_ms);
        if timeout_ms > self.max_timeout_ms {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                format!(
                    "delegation wait timeout {timeout_ms} ms exceeds the configured maximum of {} ms",
                    self.max_timeout_ms
                ),
            ));
        }
        Ok(Duration::from_millis(timeout_ms))
    }
}

impl Default for DelegationWaitPolicy {
    fn default() -> Self {
        Self::default_policy()
    }
}

/// Smith's parent lifecycle projection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ParentParkingState {
    /// No parent turn is active and no parked child hold is currently needed.
    #[default]
    Idle,
    /// A real or attributed internal parent turn is active.
    Serving,
    /// The last parent turn ended while at least one direct child remained
    /// live.  No provider stream or tool call is retained by this state.
    ParkedAwaitingChild,
    /// Shutdown has frozen both maintenance and automatic admission.
    ShuttingDown,
}

impl ParentParkingState {
    /// Stable client-facing lifecycle label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Serving => "serving",
            Self::ParkedAwaitingChild => "parked-awaiting-child",
            Self::ShuttingDown => "shutting-down",
        }
    }
}

/// A bounded, identity-only terminal child outcome key.
///
/// The protected outcome payload remains in Agent Runtime.  Smith stores only
/// stable identities for deterministic projection and tests.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TerminalOutcomeKey {
    /// Stable child identity.
    pub child_id: String,
    /// Runtime-owned outcome identity (turn or interaction request).
    pub outcome_id: String,
}

impl TerminalOutcomeKey {
    /// Creates an identity-only key.
    pub fn new(child_id: impl Into<String>, outcome_id: impl Into<String>) -> Self {
        Self {
            child_id: child_id.into(),
            outcome_id: outcome_id.into(),
        }
    }
}

/// One deterministic ready terminal batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalBatch {
    /// Smith's monotonic projection watermark.  Runtime's protected cursor is
    /// authoritative for actual consumption.
    pub watermark: u64,
    /// Stable keys in canonical lexical order.
    pub outcomes: Vec<TerminalOutcomeKey>,
}

/// A compact, inspectable snapshot of the local parking projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkingSnapshot {
    /// Current parent lifecycle state.
    pub state: ParentParkingState,
    /// Direct children with live, nonterminal work.
    pub pending_children: Vec<String>,
    /// Identity-only terminal outcomes observed by Smith.
    pub ready_outcomes: Vec<TerminalOutcomeKey>,
    /// Latest Smith projection watermark.
    pub outcome_watermark: u64,
    /// Latest Runtime cursor revision observed after an admission.
    pub committed_cursor_revision: u64,
    /// Monotonic parked-interval identity.
    pub parked_interval: u64,
    /// Whether a real user turn is ready to win local admission.
    pub user_input_ready: bool,
    /// Whether the local worker owns an admission attempt.
    pub admission_in_flight: bool,
    /// Whether shutdown has frozen automatic work.
    pub shutdown_frozen: bool,
}

/// Pure state machine for parent parking and admission scheduling.
#[derive(Debug, Default)]
pub struct DelegationParking {
    state: ParentParkingState,
    pending_children: BTreeSet<String>,
    ready_outcomes: BTreeMap<TerminalOutcomeKey, u64>,
    next_watermark: u64,
    committed_cursor_revision: u64,
    parked_interval: u64,
    user_input_ready: bool,
    admission_in_flight: bool,
    admission_batch: BTreeSet<TerminalOutcomeKey>,
    parent_completion_generation: u64,
    admission_parent_generation: Option<u64>,
    wakeup_eligible: bool,
    shutdown_frozen: bool,
}

impl DelegationParking {
    /// A fresh parent starts idle and never starts a provider request merely
    /// because a child exists.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current local lifecycle state.
    pub fn state(&self) -> ParentParkingState {
        self.state
    }

    /// Returns a deterministic projection for tests and host status surfaces.
    pub fn snapshot(&self) -> ParkingSnapshot {
        ParkingSnapshot {
            state: self.state,
            pending_children: self.pending_children.iter().cloned().collect(),
            ready_outcomes: self.ready_outcomes.keys().cloned().collect(),
            outcome_watermark: self.next_watermark,
            committed_cursor_revision: self.committed_cursor_revision,
            parked_interval: self.parked_interval,
            user_input_ready: self.user_input_ready,
            admission_in_flight: self.admission_in_flight,
            shutdown_frozen: self.shutdown_frozen,
        }
    }

    /// Marks a parent turn as active.
    pub fn parent_turn_started(&mut self) {
        if !self.shutdown_frozen {
            self.state = ParentParkingState::Serving;
            self.user_input_ready = false;
        }
    }

    /// Records a direct child with live, nonterminal work.
    pub fn child_spawned(&mut self, child_id: impl Into<String>) {
        if !self.shutdown_frozen {
            self.pending_children.insert(child_id.into());
        }
    }

    /// Replaces the pending-child projection at a safe parent turn boundary.
    /// A nonempty set enters parked state; an empty set never creates a stale
    /// parked state merely because a terminal outcome is still inspectable.
    pub fn parent_turn_completed<I, S>(&mut self, pending_children: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if self.shutdown_frozen {
            return;
        }
        let was_serving = self.state == ParentParkingState::Serving;
        self.parent_completion_generation = self.parent_completion_generation.saturating_add(1);
        self.pending_children = pending_children.into_iter().map(Into::into).collect();
        // Runtime can accept and finish a very short child-continuation turn
        // before the admission worker observes its return value. Preserve the
        // reserved batch until that worker acknowledges the canonical cursor;
        // the completion generation below prevents it from resurrecting a
        // stale local `Serving` state.
        if !self.admission_in_flight {
            self.admission_batch.clear();
            self.admission_parent_generation = None;
        }
        if self.pending_children.is_empty() {
            self.state = ParentParkingState::Idle;
            self.wakeup_eligible = was_serving && !self.ready_outcomes.is_empty();
        } else {
            self.state = ParentParkingState::ParkedAwaitingChild;
            self.parked_interval = self.parked_interval.saturating_add(1);
            self.wakeup_eligible = !self.ready_outcomes.is_empty();
        }
    }

    /// Removes a child from the live-work projection.  A terminal outcome can
    /// wake an already parked parent even when another child remains active;
    /// the next parent turn will re-evaluate the remaining hold.
    pub fn child_terminal(&mut self, child_id: &str) {
        self.pending_children.remove(child_id);
        if self.state == ParentParkingState::ParkedAwaitingChild {
            self.wakeup_eligible = true;
            if self.pending_children.is_empty() {
                self.state = ParentParkingState::Idle;
            }
        }
    }

    /// Records one terminal outcome identity. Returns `true` only for a new
    /// key, making repeated snapshots/replays harmless.
    pub fn record_terminal_outcome(&mut self, key: TerminalOutcomeKey) -> bool {
        if self.shutdown_frozen || self.ready_outcomes.contains_key(&key) {
            return false;
        }
        self.next_watermark = self.next_watermark.saturating_add(1);
        self.ready_outcomes.insert(key, self.next_watermark);
        if matches!(
            self.state,
            ParentParkingState::Idle | ParentParkingState::ParkedAwaitingChild
        ) {
            self.wakeup_eligible = true;
        }
        true
    }

    /// Reconciles the local identity-only projection with Runtime's current
    /// protected ready set. Existing keys retain their local watermarks; new
    /// keys receive later watermarks, and only keys Runtime no longer reports
    /// are removed.
    pub fn reconcile_ready_outcomes<I>(&mut self, keys: I) -> bool
    where
        I: IntoIterator<Item = TerminalOutcomeKey>,
    {
        let ready = keys.into_iter().collect::<BTreeSet<_>>();
        let before = self.ready_outcomes.keys().cloned().collect::<BTreeSet<_>>();
        self.ready_outcomes.retain(|key, _| ready.contains(key));
        for key in ready {
            if !self.ready_outcomes.contains_key(&key) {
                self.next_watermark = self.next_watermark.saturating_add(1);
                self.ready_outcomes.insert(key, self.next_watermark);
            }
        }
        if self.ready_outcomes.is_empty() {
            self.wakeup_eligible = false;
        } else if matches!(
            self.state,
            ParentParkingState::Idle | ParentParkingState::ParkedAwaitingChild
        ) {
            self.wakeup_eligible = true;
        }
        before != self.ready_outcomes.keys().cloned().collect()
    }

    /// Makes a protected outcome recovered at process start eligible to wake
    /// an idle parent. Ordinary outcomes observed after the parent boundary
    /// are already eligible when recorded; this method documents and repairs
    /// the cold-start snapshot path explicitly.
    pub fn enable_idle_wakeup_for_recovered_outcomes(&mut self) {
        if !self.shutdown_frozen
            && self.state == ParentParkingState::Idle
            && !self.ready_outcomes.is_empty()
        {
            self.wakeup_eligible = true;
        }
    }

    /// Returns all locally observed terminal identities in deterministic order.
    pub fn ready_batch(&self) -> Option<TerminalBatch> {
        (!self.ready_outcomes.is_empty()).then(|| TerminalBatch {
            watermark: self.next_watermark,
            outcomes: self.ready_outcomes.keys().cloned().collect(),
        })
    }

    /// Gives real user input explicit local priority in deterministic tests.
    /// Runtime still repeats this arbitration at its canonical admission lock.
    pub fn mark_user_input_ready(&mut self) {
        self.user_input_ready = true;
    }

    /// Clears the local user-input marker after its safe boundary.
    pub fn clear_user_input_ready(&mut self) {
        self.user_input_ready = false;
    }

    /// Reserves exactly one child-completion admission attempt.
    pub fn begin_child_completion_admission(&mut self) -> bool {
        if self.shutdown_frozen
            || self.user_input_ready
            || self.admission_in_flight
            || !self.wakeup_eligible
            || self.ready_outcomes.is_empty()
            || !matches!(
                self.state,
                ParentParkingState::Idle | ParentParkingState::ParkedAwaitingChild
            )
        {
            return false;
        }
        self.admission_in_flight = true;
        self.admission_batch = self.ready_outcomes.keys().cloned().collect();
        self.admission_parent_generation = Some(self.parent_completion_generation);
        true
    }

    /// Runtime accepted the batch and committed its canonical cursor.
    pub fn admission_accepted(&mut self, cursor_revision: u64) {
        let completion_crossed_admission = self
            .admission_parent_generation
            .take()
            .is_some_and(|generation| self.parent_completion_generation > generation);
        self.admission_in_flight = false;
        self.committed_cursor_revision = self.committed_cursor_revision.max(cursor_revision);
        for key in std::mem::take(&mut self.admission_batch) {
            self.ready_outcomes.remove(&key);
        }
        self.wakeup_eligible = !self.ready_outcomes.is_empty()
            && matches!(
                self.state,
                ParentParkingState::Idle | ParentParkingState::ParkedAwaitingChild
            );
        if !completion_crossed_admission {
            self.state = ParentParkingState::Serving;
        }
    }

    /// Runtime lost the idle race.  Protected outcomes remain ready.
    pub fn admission_busy(&mut self) {
        self.admission_in_flight = false;
        self.admission_batch.clear();
        self.admission_parent_generation = None;
    }

    /// Runtime observed an already-consumed cursor.  The local projection may
    /// safely discard its duplicate identity batch; Runtime remains the source
    /// of truth for exact payload delivery.
    pub fn admission_stale(&mut self, cursor_revision: u64) {
        self.admission_in_flight = false;
        self.admission_parent_generation = None;
        self.committed_cursor_revision = self.committed_cursor_revision.max(cursor_revision);
        for key in std::mem::take(&mut self.admission_batch) {
            self.ready_outcomes.remove(&key);
        }
        self.wakeup_eligible = !self.ready_outcomes.is_empty()
            && matches!(
                self.state,
                ParentParkingState::Idle | ParentParkingState::ParkedAwaitingChild
            );
    }

    /// Clears a local admission reservation after a structural conflict.
    pub fn admission_conflict(&mut self) {
        self.admission_in_flight = false;
        self.admission_batch.clear();
        self.admission_parent_generation = None;
        self.wakeup_eligible = false;
    }

    /// Reconciles a canonical no-ready snapshot after another admission has
    /// consumed the protected batch.
    pub fn reconcile_no_ready_outcomes(&mut self) {
        if self.ready_outcomes.is_empty() {
            self.wakeup_eligible = false;
        }
    }

    /// Freezes shutdown. No maintenance/admission decision can be reserved
    /// afterward, and the local state cannot become parked again.
    pub fn shutdown(&mut self) {
        self.shutdown_frozen = true;
        self.admission_in_flight = false;
        self.admission_batch.clear();
        self.admission_parent_generation = None;
        self.wakeup_eligible = false;
        self.state = ParentParkingState::ShuttingDown;
    }

    /// Whether shutdown has frozen automatic work.
    pub fn is_shutdown_frozen(&self) -> bool {
        self.shutdown_frozen
    }

    /// Process-exit reconciliation helper. It returns identity-only
    /// interrupted outcomes and never schedules a restart or provider work.
    pub fn reconcile_process_exit(&mut self) -> Vec<TerminalOutcomeKey> {
        let interrupted = self
            .pending_children
            .iter()
            .map(|child| TerminalOutcomeKey::new(child.clone(), "interrupted_by_process_exit"))
            .collect::<Vec<_>>();
        self.pending_children.clear();
        self.admission_batch.clear();
        self.admission_in_flight = false;
        self.admission_parent_generation = None;
        self.state = if self.shutdown_frozen {
            ParentParkingState::ShuttingDown
        } else {
            ParentParkingState::Idle
        };
        self.wakeup_eligible = false;
        interrupted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_policy_accepts_immediate_default_and_rejects_over_max() {
        let policy = DelegationWaitPolicy::new(0, 30_000).expect("valid policy");
        assert_eq!(policy.resolve_timeout(None).unwrap(), Duration::ZERO);
        assert_eq!(
            policy.resolve_timeout(Some(30_000)).unwrap(),
            Duration::from_secs(30)
        );
        assert!(policy.resolve_timeout(Some(30_001)).is_err());
        assert!(DelegationWaitPolicy::new(4, 3).is_err());
        assert!(DelegationWaitPolicy::new(0, 0).is_err());
    }

    #[test]
    fn parent_parks_only_for_live_children_and_wakes_once() {
        let mut parking = DelegationParking::new();
        parking.parent_turn_started();
        parking.child_spawned("child-2");
        parking.parent_turn_completed(["child-2"]);
        assert_eq!(parking.state(), ParentParkingState::ParkedAwaitingChild);

        parking.child_terminal("child-2");
        parking.record_terminal_outcome(TerminalOutcomeKey::new("child-2", "turn-4"));
        assert_eq!(parking.state(), ParentParkingState::Idle);
        assert!(parking.begin_child_completion_admission());
        assert!(!parking.begin_child_completion_admission());
        parking.admission_accepted(1);
        assert_eq!(parking.state(), ParentParkingState::Serving);
        assert!(parking.ready_batch().is_none());
    }

    #[test]
    fn terminal_batch_is_sorted_and_replay_deduplicated() {
        let mut parking = DelegationParking::new();
        parking.parent_turn_completed(["child-1"]);
        assert!(parking.record_terminal_outcome(TerminalOutcomeKey::new("child-2", "turn-9")));
        assert!(parking.record_terminal_outcome(TerminalOutcomeKey::new("child-1", "turn-3")));
        assert!(!parking.record_terminal_outcome(TerminalOutcomeKey::new("child-1", "turn-3")));
        let batch = parking.ready_batch().expect("ready batch");
        assert_eq!(batch.watermark, 2);
        assert_eq!(
            batch.outcomes,
            vec![
                TerminalOutcomeKey::new("child-1", "turn-3"),
                TerminalOutcomeKey::new("child-2", "turn-9"),
            ]
        );
    }

    #[test]
    fn user_priority_and_shutdown_freeze_admission() {
        let mut parking = DelegationParking::new();
        parking.parent_turn_started();
        parking.parent_turn_completed(["child-1"]);
        parking.record_terminal_outcome(TerminalOutcomeKey::new("child-1", "turn-1"));
        parking.mark_user_input_ready();
        assert!(!parking.begin_child_completion_admission());
        parking.clear_user_input_ready();
        assert!(parking.begin_child_completion_admission());
        parking.shutdown();
        parking.admission_busy();
        assert!(!parking.begin_child_completion_admission());
        assert_eq!(parking.state(), ParentParkingState::ShuttingDown);
    }

    #[test]
    fn process_exit_reconciles_without_restart() {
        let mut parking = DelegationParking::new();
        parking.parent_turn_completed(["child-1", "child-2"]);
        let interrupted = parking.reconcile_process_exit();
        assert_eq!(
            interrupted,
            vec![
                TerminalOutcomeKey::new("child-1", "interrupted_by_process_exit"),
                TerminalOutcomeKey::new("child-2", "interrupted_by_process_exit"),
            ]
        );
        assert_eq!(parking.state(), ParentParkingState::Idle);
        assert!(parking.ready_batch().is_none());
    }

    #[test]
    fn fresh_idle_outcome_wakes_once_and_persistent_conflict_does_not_spin() {
        let mut parking = DelegationParking::new();
        parking.record_terminal_outcome(TerminalOutcomeKey::new("child-1", "turn-1"));
        assert!(parking.begin_child_completion_admission());
        parking.admission_conflict();
        assert!(!parking.begin_child_completion_admission());
        assert!(parking.ready_batch().is_some());
    }

    #[test]
    fn outcome_recorded_after_parent_completion_still_wakes_once() {
        let mut parking = DelegationParking::new();
        parking.parent_turn_started();
        parking.child_spawned("child-1");
        // The event projection reaches the safe boundary before the protected
        // outcome snapshot worker observes completion.
        parking.parent_turn_completed(Vec::<String>::new());
        parking.child_terminal("child-1");
        assert!(parking.record_terminal_outcome(TerminalOutcomeKey::new("child-1", "turn-1")));
        assert!(parking.begin_child_completion_admission());
        assert!(!parking.begin_child_completion_admission());
    }

    #[test]
    fn accepted_admission_acknowledges_only_its_reserved_batch() {
        let mut parking = DelegationParking::new();
        parking.record_terminal_outcome(TerminalOutcomeKey::new("child-1", "turn-1"));
        assert!(parking.begin_child_completion_admission());

        // A second child can finish while Runtime is admitting the first
        // cursor. Accepting that cursor must not erase the later outcome.
        parking.record_terminal_outcome(TerminalOutcomeKey::new("child-2", "turn-2"));
        parking.admission_accepted(1);

        assert_eq!(
            parking
                .ready_batch()
                .expect("later outcome remains ready")
                .outcomes,
            vec![TerminalOutcomeKey::new("child-2", "turn-2")]
        );
    }

    #[test]
    fn fast_accepted_turn_completion_cannot_resurrect_serving_state() {
        let mut parking = DelegationParking::new();
        parking.record_terminal_outcome(TerminalOutcomeKey::new("child-1", "turn-1"));
        assert!(parking.begin_child_completion_admission());

        // Runtime acceptance emits the internal turn events independently of
        // returning the admission result. Model the event worker observing a
        // complete fast turn before the admission worker handles `Accepted`.
        parking.parent_turn_started();
        parking.parent_turn_completed(Vec::<String>::new());
        parking.admission_accepted(1);

        assert_eq!(parking.state(), ParentParkingState::Idle);
        assert!(parking.ready_batch().is_none());
        parking.record_terminal_outcome(TerminalOutcomeKey::new("child-2", "turn-2"));
        assert!(parking.begin_child_completion_admission());
    }
}
