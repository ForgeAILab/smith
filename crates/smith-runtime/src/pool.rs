//! Credential pool state: which account is active, which are spent, and what
//! the provider last reported about each.
//!
//! This module is pure. It performs no I/O, opens no keychain, and asks no
//! questions: it decides *what* should be offered and to whom, and the caller
//! owns the asking. That separation is what makes exhaustion behavior testable
//! without a provider, a clock, or a terminal.
//!
//! Three rules run through it:
//!
//! * **Unknown is not zero.** A member with no observed snapshot reports its
//!   consumption as `None`. Rendering that as 0% would tell the user an
//!   account is untouched when nothing has ever measured it.
//! * **Cooldowns expire on their own.** A spent member is not disabled; it is
//!   unavailable until a stated instant, after which it is eligible again with
//!   no bookkeeping to undo.
//! * **A pool of one is not a pool.** Everything that offers rotation checks
//!   for somewhere to rotate *to* first, so a single-account provider behaves
//!   exactly as it did before pools existed.

use std::collections::BTreeMap;

use agent_runtime_core::provider::RateLimitSnapshot;
use smith_host::rotation::{RotationMember, RotationRequest, RotationTrigger};

/// How long a member waits when the provider reported no reset time.
///
/// Providers usually state one; this is the fallback for the ones that do not.
/// Fifteen minutes is short enough that a member is not stranded for an hour
/// on a guess, and long enough that the pool does not immediately retry an
/// account it just watched fail. A manual switch overrides it either way, so
/// the cost of being wrong is bounded in both directions.
pub const DEFAULT_COOLDOWN_MS: u64 = 15 * 60 * 1_000;

/// One declared account in a provider's pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolMember {
    /// Zero-based position in the declared order.
    pub position: usize,
    /// The credential reference, e.g. `keychain:smith/work`.
    ///
    /// This is a *location*, not a secret, and it is what the user wrote in
    /// their own configuration — so it doubles as the display label. A value
    /// that failed to parse as a reference never reaches this type: it is
    /// rejected during configuration resolution, precisely because an
    /// unparseable "reference" is usually a pasted key.
    pub reference: String,
}

impl PoolMember {
    /// The label a surface shows for this member.
    pub fn label(&self) -> &str {
        &self.reference
    }
}

/// The live state of one provider's credential pool.
#[derive(Debug, Clone)]
pub struct CredentialPool {
    provider: String,
    members: Vec<PoolMember>,
    active: usize,
    /// Position → when its window reopens, in Unix milliseconds.
    cooldowns: BTreeMap<usize, u64>,
    /// Position → the latest snapshot the provider reported for it.
    snapshots: BTreeMap<usize, RateLimitSnapshot>,
    rotate_at_percent: Option<u8>,
    /// Whether a proactive offer was already declined for the current turn.
    threshold_offered: bool,
}

impl CredentialPool {
    /// Builds a pool over `references` in declared order.
    ///
    /// An empty list yields an empty pool, which every rotation path treats as
    /// "nothing to offer" rather than as an error: a provider may legitimately
    /// authenticate with an inline key or no credential at all.
    pub fn new(
        provider: impl Into<String>,
        references: impl IntoIterator<Item = String>,
        rotate_at_percent: Option<u8>,
    ) -> Self {
        let members = references
            .into_iter()
            .enumerate()
            .map(|(position, reference)| PoolMember {
                position,
                reference,
            })
            .collect();
        Self {
            provider: provider.into(),
            members,
            active: 0,
            cooldowns: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            rotate_at_percent,
            threshold_offered: false,
        }
    }

    /// The provider this pool belongs to.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Every declared member, in pool order.
    pub fn members(&self) -> &[PoolMember] {
        &self.members
    }

    /// The member serving attempts now.
    pub fn active(&self) -> Option<&PoolMember> {
        self.members.get(self.active)
    }

    /// The active member's position.
    pub fn active_position(&self) -> usize {
        self.active
    }

    /// Whether there is more than one account to choose between.
    pub fn has_pool(&self) -> bool {
        self.members.len() > 1
    }

    /// Makes `position` active, if it exists.
    ///
    /// Returns whether the active member changed. Selecting a cooling member
    /// is allowed on purpose: a manual switch is how a user re-tests a window
    /// they believe has reopened, and the cooldown is Smith's estimate rather
    /// than the provider's ruling.
    pub fn set_active(&mut self, position: usize) -> bool {
        if position >= self.members.len() || position == self.active {
            return false;
        }
        self.active = position;
        self.threshold_offered = false;
        true
    }

    /// Records what the provider reported for `position`.
    pub fn record_snapshot(&mut self, position: usize, snapshot: RateLimitSnapshot) {
        if position >= self.members.len() || snapshot.is_empty() {
            return;
        }
        self.snapshots.insert(position, snapshot);
    }

    /// The latest snapshot observed for `position`.
    pub fn snapshot(&self, position: usize) -> Option<&RateLimitSnapshot> {
        self.snapshots.get(&position)
    }

    /// Server-reported consumption for `position`, or `None` when unmeasured.
    pub fn used_percent(&self, position: usize) -> Option<f64> {
        self.snapshots
            .get(&position)?
            .most_consumed()?
            .used_percent_or_derived()
    }

    /// Places `position` in cooldown until its window reopens.
    ///
    /// `resets_at_ms` is what the provider reported; when it reported nothing,
    /// the bounded [`DEFAULT_COOLDOWN_MS`] applies rather than a guess at the
    /// provider's schedule.
    pub fn exhaust(&mut self, position: usize, resets_at_ms: Option<u64>, now_ms: u64) {
        if position >= self.members.len() {
            return;
        }
        let until = resets_at_ms.unwrap_or_else(|| now_ms.saturating_add(DEFAULT_COOLDOWN_MS));
        // A later reset wins: two reports for one member should not shorten a
        // cooldown the provider already justified.
        let until = self
            .cooldowns
            .get(&position)
            .map_or(until, |existing| (*existing).max(until));
        self.cooldowns.insert(position, until);
    }

    /// When `position` becomes eligible again, if it is cooling down now.
    pub fn cooling_until(&self, position: usize, now_ms: u64) -> Option<u64> {
        self.cooldowns
            .get(&position)
            .copied()
            .filter(|until| *until > now_ms)
    }

    /// Whether `position` can serve an attempt at `now_ms`.
    pub fn is_eligible(&self, position: usize, now_ms: u64) -> bool {
        position < self.members.len() && self.cooling_until(position, now_ms).is_none()
    }

    /// The soonest moment any member becomes eligible again.
    ///
    /// This is what an all-exhausted failure reports, so the user learns when
    /// work can resume rather than only that it cannot.
    pub fn earliest_reset(&self, now_ms: u64) -> Option<u64> {
        self.members
            .iter()
            .filter_map(|member| self.cooling_until(member.position, now_ms))
            .min()
    }

    /// Every member as a surface should show it.
    pub fn view(&self, now_ms: u64) -> Vec<RotationMember> {
        self.members
            .iter()
            .map(|member| RotationMember {
                position: member.position,
                label: member.label().to_owned(),
                used_percent: self.used_percent(member.position),
                cooling_until_ms: self.cooling_until(member.position, now_ms),
            })
            .collect()
    }

    /// The members that could take over from the active one, in pool order.
    pub fn eligible_others(&self, now_ms: u64) -> Vec<RotationMember> {
        self.view(now_ms)
            .into_iter()
            .filter(|member| member.position != self.active && member.is_eligible())
            .collect()
    }

    /// Builds the offer to make when the active member is spent.
    ///
    /// Returns `None` when no other member could serve the attempt, because
    /// then there is nothing to ask: the caller fails the turn and reports
    /// [`CredentialPool::earliest_reset`] instead.
    pub fn exhaustion_offer(&self, now_ms: u64) -> Option<RotationRequest> {
        self.offer(RotationTrigger::Exhausted, now_ms)
    }

    /// Builds the offer to make when the active member crosses the configured
    /// threshold, if one is configured and it has been crossed.
    ///
    /// Asks at most once per turn: a user who declined at 93% should not be
    /// asked again at 94% in the same breath.
    pub fn threshold_offer(&self, now_ms: u64) -> Option<RotationRequest> {
        if self.threshold_offered {
            return None;
        }
        let percent = self.rotate_at_percent?;
        let used = self.used_percent(self.active)?;
        if used < f64::from(percent) {
            return None;
        }
        self.offer(RotationTrigger::Threshold { percent }, now_ms)
    }

    /// Records that a proactive offer was made for the current turn.
    pub fn mark_threshold_offered(&mut self) {
        self.threshold_offered = true;
    }

    /// Clears per-turn offer bookkeeping.
    pub fn begin_turn(&mut self) {
        self.threshold_offered = false;
    }

    fn offer(&self, trigger: RotationTrigger, now_ms: u64) -> Option<RotationRequest> {
        let active = self.active()?;
        let eligible = self.eligible_others(now_ms);
        if eligible.is_empty() {
            return None;
        }
        Some(RotationRequest {
            provider: self.provider.clone(),
            trigger,
            outgoing: RotationMember {
                position: active.position,
                label: active.label().to_owned(),
                used_percent: self.used_percent(active.position),
                cooling_until_ms: self.cooling_until(active.position, now_ms),
            },
            eligible,
            outgoing_resets_at_ms: self.cooldowns.get(&active.position).copied(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::provider::RateLimitWindow;

    const NOW: u64 = 1_785_862_800_000;

    fn pool() -> CredentialPool {
        CredentialPool::new(
            "acme",
            [
                "keychain:smith/personal".to_owned(),
                "keychain:smith/work".to_owned(),
            ],
            None,
        )
    }

    fn snapshot(used_percent: f64) -> RateLimitSnapshot {
        let mut snapshot = RateLimitSnapshot::new();
        snapshot.push(RateLimitWindow {
            used_percent: Some(used_percent),
            ..RateLimitWindow::new("primary")
        });
        snapshot
    }

    #[test]
    fn a_new_pool_starts_on_its_first_member() {
        let pool = pool();
        assert_eq!(pool.active_position(), 0);
        assert_eq!(pool.active().expect("a member").label(), "keychain:smith/personal");
        assert!(pool.has_pool());
    }

    #[test]
    fn a_pool_of_one_is_not_a_pool_and_offers_nothing() {
        let pool = CredentialPool::new("acme", ["env:ONLY".to_owned()], None);
        assert!(!pool.has_pool());
        // Nothing to rotate to, so nothing is ever asked.
        assert_eq!(pool.exhaustion_offer(NOW), None);
    }

    #[test]
    fn an_empty_pool_offers_nothing_rather_than_failing() {
        let pool = CredentialPool::new("acme", [], None);
        assert_eq!(pool.active(), None);
        assert_eq!(pool.exhaustion_offer(NOW), None);
        assert_eq!(pool.earliest_reset(NOW), None);
    }

    #[test]
    fn unmeasured_consumption_stays_unknown() {
        let mut pool = pool();
        assert_eq!(pool.used_percent(0), None);

        // An empty snapshot is not a measurement, so it does not become one.
        pool.record_snapshot(0, RateLimitSnapshot::new());
        assert_eq!(pool.used_percent(0), None);

        pool.record_snapshot(0, snapshot(82.0));
        assert_eq!(pool.used_percent(0), Some(82.0));
    }

    #[test]
    fn exhaustion_cools_a_member_until_its_reported_reset() {
        let mut pool = pool();
        pool.exhaust(0, Some(NOW + 3_600_000), NOW);

        assert!(!pool.is_eligible(0, NOW));
        assert_eq!(pool.cooling_until(0, NOW), Some(NOW + 3_600_000));
        assert!(pool.is_eligible(1, NOW));
        assert_eq!(pool.earliest_reset(NOW), Some(NOW + 3_600_000));
    }

    #[test]
    fn a_cooldown_expires_on_its_own() {
        let mut pool = pool();
        pool.exhaust(0, Some(NOW + 1_000), NOW);
        assert!(!pool.is_eligible(0, NOW));

        // One second later the window has reopened, with nothing to undo.
        assert!(pool.is_eligible(0, NOW + 1_001));
        assert_eq!(pool.cooling_until(0, NOW + 1_001), None);
        assert_eq!(pool.earliest_reset(NOW + 1_001), None);
    }

    #[test]
    fn an_unreported_reset_falls_back_to_a_bounded_default() {
        let mut pool = pool();
        pool.exhaust(0, None, NOW);

        // Not "resets now", and not forever.
        assert_eq!(
            pool.cooling_until(0, NOW),
            Some(NOW + DEFAULT_COOLDOWN_MS)
        );
    }

    #[test]
    fn a_later_reset_never_shortens_an_existing_cooldown() {
        let mut pool = pool();
        pool.exhaust(0, Some(NOW + 3_600_000), NOW);
        pool.exhaust(0, Some(NOW + 60_000), NOW);

        assert_eq!(pool.cooling_until(0, NOW), Some(NOW + 3_600_000));
    }

    #[test]
    fn exhaustion_offers_the_remaining_eligible_member() {
        let mut pool = pool();
        pool.record_snapshot(0, snapshot(100.0));
        pool.exhaust(0, Some(NOW + 3_600_000), NOW);

        let offer = pool.exhaustion_offer(NOW).expect("an offer");
        assert_eq!(offer.provider, "acme");
        assert_eq!(offer.trigger, RotationTrigger::Exhausted);
        assert_eq!(offer.outgoing.position, 0);
        assert_eq!(offer.outgoing.used_percent, Some(100.0));
        assert_eq!(offer.outgoing_resets_at_ms, Some(NOW + 3_600_000));
        assert_eq!(offer.eligible.len(), 1);
        assert_eq!(offer.eligible[0].position, 1);
        // The incoming member has never been measured, and says so.
        assert_eq!(offer.eligible[0].used_percent, None);
    }

    #[test]
    fn nothing_is_offered_once_every_member_is_spent() {
        let mut pool = pool();
        pool.exhaust(0, Some(NOW + 3_600_000), NOW);
        pool.exhaust(1, Some(NOW + 600_000), NOW);

        // No prompt: there is no member that could serve the replay, so the
        // caller fails the turn with the soonest reset instead of asking a
        // question with no good answer.
        assert_eq!(pool.exhaustion_offer(NOW), None);
        assert_eq!(pool.earliest_reset(NOW), Some(NOW + 600_000));
    }

    #[test]
    fn switching_changes_the_active_member() {
        let mut pool = pool();
        assert!(pool.set_active(1));
        assert_eq!(pool.active_position(), 1);
        assert_eq!(pool.active().expect("a member").label(), "keychain:smith/work");

        // Selecting the current member, or one that does not exist, is a no-op.
        assert!(!pool.set_active(1));
        assert!(!pool.set_active(9));
        assert_eq!(pool.active_position(), 1);
    }

    #[test]
    fn a_manual_switch_may_target_a_cooling_member() {
        let mut pool = pool();
        pool.exhaust(1, Some(NOW + 3_600_000), NOW);

        // The cooldown is Smith's estimate, not the provider's ruling: a user
        // who believes the window reopened is allowed to find out.
        assert!(pool.set_active(1));
        assert_eq!(pool.active_position(), 1);
    }

    #[test]
    fn the_threshold_offers_rotation_before_exhaustion() {
        let mut pool = CredentialPool::new(
            "acme",
            [
                "keychain:smith/personal".to_owned(),
                "keychain:smith/work".to_owned(),
            ],
            Some(90),
        );

        pool.record_snapshot(0, snapshot(89.0));
        assert_eq!(pool.threshold_offer(NOW), None, "below the threshold");

        pool.record_snapshot(0, snapshot(93.0));
        let offer = pool.threshold_offer(NOW).expect("an offer");
        assert_eq!(offer.trigger, RotationTrigger::Threshold { percent: 90 });
        assert_eq!(offer.outgoing.used_percent, Some(93.0));
    }

    #[test]
    fn a_declined_threshold_is_not_re_asked_within_the_turn() {
        let mut pool = CredentialPool::new(
            "acme",
            [
                "keychain:smith/personal".to_owned(),
                "keychain:smith/work".to_owned(),
            ],
            Some(90),
        );
        pool.record_snapshot(0, snapshot(93.0));
        assert!(pool.threshold_offer(NOW).is_some());

        pool.mark_threshold_offered();
        pool.record_snapshot(0, snapshot(94.0));
        assert_eq!(pool.threshold_offer(NOW), None);

        // A new turn asks again, because the account is still filling up.
        pool.begin_turn();
        assert!(pool.threshold_offer(NOW).is_some());
    }

    #[test]
    fn an_unmeasured_member_never_crosses_the_threshold() {
        let pool = CredentialPool::new(
            "acme",
            [
                "keychain:smith/personal".to_owned(),
                "keychain:smith/work".to_owned(),
            ],
            Some(90),
        );
        // Unknown is not "over the line" any more than it is 0%.
        assert_eq!(pool.threshold_offer(NOW), None);
    }

    #[test]
    fn a_view_reports_unknown_cooldown_and_active_state_for_every_member() {
        let mut pool = pool();
        pool.record_snapshot(0, snapshot(40.0));
        pool.exhaust(1, Some(NOW + 600_000), NOW);

        let view = pool.view(NOW);
        assert_eq!(view.len(), 2);
        assert_eq!(view[0].used_percent, Some(40.0));
        assert_eq!(view[0].cooling_until_ms, None);
        assert!(view[0].is_eligible());
        assert_eq!(view[1].used_percent, None);
        assert_eq!(view[1].cooling_until_ms, Some(NOW + 600_000));
        assert!(!view[1].is_eligible());
    }

    #[test]
    fn a_label_is_the_reference_the_user_wrote_and_carries_no_secret() {
        let pool = pool();
        let view = pool.view(NOW);
        assert_eq!(view[0].label, "keychain:smith/personal");
        assert_eq!(view[1].label, "keychain:smith/work");
    }
}
