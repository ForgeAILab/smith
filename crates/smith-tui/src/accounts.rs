//! Rendering a provider's credential pool: usage meters, cooldowns, and the
//! rotation offer.
//!
//! Everything here is pure formatting over a pool snapshot, so what the user
//! reads can be tested without a provider, a clock, or a terminal.
//!
//! Two presentation rules carry real weight:
//!
//! * **Unknown reads as unknown.** A member the provider has never reported on
//!   shows "usage unknown", never "0% used". The difference matters: one says
//!   nothing has measured this account, the other says it is untouched, and a
//!   user deciding which account to switch to would act differently on each.
//! * **Usage sits beside token counters, never inside them.** A rate-limit
//!   window is a server-reported percentage of an account's plan; token
//!   counters are Smith's disjoint measurement of one session. Mixing them
//!   would invite adding a percentage to a token count.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::picker::ResourceEntry;
use smith_host::rotation::{RotationMember, RotationRequest, RotationTrigger};

/// Wall-clock now, in Unix milliseconds.
///
/// Only ever used to render a *relative* delay ("in 42m") from an absolute
/// reset the provider reported. A clock that has jumped makes the label wrong,
/// not the policy: eligibility is decided in the runtime against its own
/// clock, never from this.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

/// One minute in milliseconds.
const MINUTE_MS: u64 = 60 * 1_000;
/// One hour in milliseconds.
const HOUR_MS: u64 = 60 * MINUTE_MS;
/// One day in milliseconds.
const DAY_MS: u64 = 24 * HOUR_MS;

/// Renders a member's server-reported consumption.
///
/// Returns `"usage unknown"` rather than a number when nothing has measured
/// it, because inventing `0%` would be a claim the provider never made.
pub fn usage_label(used_percent: Option<f64>) -> String {
    match used_percent {
        Some(percent) => format!("{}% used", percent.round() as i64),
        None => "usage unknown".to_owned(),
    }
}

/// Renders a duration as a coarse, readable delay.
///
/// Deliberately coarse: a reset an hour out is "in 1h", not "in 59m 47s". The
/// number is a provider's estimate and the extra precision would imply a
/// confidence nobody has.
pub fn until_label(now_ms: u64, at_ms: u64) -> String {
    let remaining = at_ms.saturating_sub(now_ms);
    if remaining == 0 {
        return "now".to_owned();
    }
    if remaining < MINUTE_MS {
        return "in under a minute".to_owned();
    }
    if remaining < HOUR_MS {
        return format!("in {}m", remaining / MINUTE_MS);
    }
    if remaining < DAY_MS {
        let hours = remaining / HOUR_MS;
        let minutes = (remaining % HOUR_MS) / MINUTE_MS;
        if minutes == 0 {
            return format!("in {hours}h");
        }
        return format!("in {hours}h {minutes}m");
    }
    format!("in {}d", remaining / DAY_MS)
}

/// Renders one pool member's one-line context.
///
/// A spent member whose consumption was never measured reads as "spent"
/// alone. "usage unknown · spent" is technically true — the rejection's
/// headers are dropped on the error path, so no number reached us — but it
/// reads as a contradiction, and the cooldown already says everything the
/// user needs. Absence is still never rendered as a number.
pub fn member_detail(member: &RotationMember, now_ms: u64) -> String {
    match (member.cooling_until_ms, member.used_percent) {
        (Some(until), Some(_)) => format!(
            "{} · spent, resets {}",
            usage_label(member.used_percent),
            until_label(now_ms, until)
        ),
        (Some(until), None) => format!("spent, resets {}", until_label(now_ms, until)),
        (None, _) => usage_label(member.used_percent),
    }
}

/// Builds the account picker's entries from a pool view.
///
/// A cooling member is listed rather than hidden, and carries the reason it
/// cannot be chosen: a user needs to see that the account exists and when it
/// comes back, not wonder where it went.
pub fn account_entries(
    members: &[RotationMember],
    active: usize,
    now_ms: u64,
) -> Vec<ResourceEntry> {
    members
        .iter()
        .map(|member| {
            let mut entry = ResourceEntry::new(
                member.position.to_string(),
                member.label.clone(),
                member_detail(member, now_ms),
            );
            entry.active = member.position == active;
            if let Some(until) = member.cooling_until_ms {
                entry.disabled_reason =
                    Some(format!("spent until this window resets {}", until_label(now_ms, until)));
            }
            entry
        })
        .collect()
}

/// The rotation modal's body.
///
/// States the cache cost explicitly. Switching accounts abandons the
/// provider-side prompt cache, so the replayed turn resends its whole context
/// uncached — the user is agreeing to pay that, and should be told so before
/// they do rather than discover it on the bill.
pub fn rotation_prompt_body(request: &RotationRequest, now_ms: u64) -> String {
    let mut lines = Vec::new();

    match request.trigger {
        RotationTrigger::Exhausted => lines.push(format!(
            "{} is spent{}.",
            request.outgoing.label,
            request
                .outgoing_resets_at_ms
                .map(|at| format!(", and resets {}", until_label(now_ms, at)))
                .unwrap_or_default()
        )),
        RotationTrigger::Threshold { percent } => lines.push(format!(
            "{} is at {}, past the {percent}% you configured.",
            request.outgoing.label,
            usage_label(request.outgoing.used_percent)
        )),
    }

    lines.push(String::new());
    lines.push("Switch to:".to_owned());
    for member in &request.eligible {
        lines.push(format!(
            "  {}. {} — {}",
            member.position + 1,
            member.label,
            usage_label(member.used_percent)
        ));
    }
    lines.push(String::new());
    lines.push(
        "Switching resends this turn without the provider's prompt cache, so it \
         costs a full uncached request and spends the other account's budget."
            .to_owned(),
    );

    lines.join("\n")
}

/// The transcript line recorded when the active account changes.
pub fn switch_notice(outgoing: &str, incoming: &str, manual: bool) -> String {
    let how = if manual { "switched" } else { "rotated" };
    format!("{how} from {outgoing} to {incoming}")
}

/// The transcript line recorded when a rotation offer is refused.
pub fn declined_notice(outgoing: &str, resets_at_ms: Option<u64>, now_ms: u64) -> String {
    match resets_at_ms {
        Some(at) => format!(
            "stayed on {outgoing}; its window resets {}",
            until_label(now_ms, at)
        ),
        None => format!("stayed on {outgoing}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_785_862_800_000;

    fn member(position: usize, label: &str) -> RotationMember {
        RotationMember {
            position,
            label: label.to_owned(),
            used_percent: None,
            cooling_until_ms: None,
        }
    }

    #[test]
    fn unmeasured_usage_reads_as_unknown_not_as_zero() {
        assert_eq!(usage_label(None), "usage unknown");
        // The distinction the whole feature rests on.
        assert_ne!(usage_label(None), usage_label(Some(0.0)));
        assert_eq!(usage_label(Some(0.0)), "0% used");
    }

    #[test]
    fn a_percentage_is_rendered_whole() {
        assert_eq!(usage_label(Some(82.4)), "82% used");
        assert_eq!(usage_label(Some(82.6)), "83% used");
        assert_eq!(usage_label(Some(100.0)), "100% used");
    }

    #[test]
    fn a_reset_is_rendered_coarsely() {
        assert_eq!(until_label(NOW, NOW), "now");
        assert_eq!(until_label(NOW, NOW + 30_000), "in under a minute");
        assert_eq!(until_label(NOW, NOW + 42 * MINUTE_MS), "in 42m");
        assert_eq!(until_label(NOW, NOW + HOUR_MS), "in 1h");
        assert_eq!(until_label(NOW, NOW + HOUR_MS + 30 * MINUTE_MS), "in 1h 30m");
        assert_eq!(until_label(NOW, NOW + 3 * DAY_MS), "in 3d");
    }

    #[test]
    fn a_reset_already_past_reads_as_now_rather_than_underflowing() {
        assert_eq!(until_label(NOW, NOW - HOUR_MS), "now");
    }

    #[test]
    fn a_cooling_member_shows_its_usage_and_when_it_returns() {
        let mut cooling = member(1, "keychain:smith/work");
        cooling.used_percent = Some(100.0);
        cooling.cooling_until_ms = Some(NOW + HOUR_MS);

        assert_eq!(
            member_detail(&cooling, NOW),
            "100% used · spent, resets in 1h"
        );
    }

    #[test]
    fn an_unmeasured_spent_member_does_not_read_as_a_contradiction() {
        let mut cooling = member(1, "keychain:smith/work");
        cooling.cooling_until_ms = Some(NOW + HOUR_MS);

        // Not "usage unknown · spent": the cooldown already says the state,
        // and no number is invented to fill the gap.
        assert_eq!(member_detail(&cooling, NOW), "spent, resets in 1h");
    }

    #[test]
    fn the_picker_lists_every_member_marking_the_active_and_the_spent() {
        let mut spent = member(1, "keychain:smith/work");
        spent.cooling_until_ms = Some(NOW + HOUR_MS);
        let mut active = member(0, "keychain:smith/personal");
        active.used_percent = Some(40.0);

        let entries = account_entries(&[active, spent], 0, NOW);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "0");
        assert_eq!(entries[0].label, "keychain:smith/personal");
        assert_eq!(entries[0].detail, "40% used");
        assert!(entries[0].active);
        assert!(entries[0].disabled_reason.is_none());

        // Listed, not hidden: the user needs to know it exists and when it
        // comes back.
        assert_eq!(entries[1].id, "1");
        assert!(!entries[1].active);
        assert_eq!(
            entries[1].disabled_reason.as_deref(),
            Some("spent until this window resets in 1h")
        );
    }

    #[test]
    fn the_rotation_modal_names_the_cost_of_switching() {
        let mut outgoing = member(0, "keychain:smith/personal");
        outgoing.used_percent = Some(100.0);
        let request = RotationRequest {
            provider: "acme".to_owned(),
            trigger: RotationTrigger::Exhausted,
            outgoing,
            eligible: vec![member(1, "keychain:smith/work")],
            outgoing_resets_at_ms: Some(NOW + HOUR_MS),
        };

        let body = rotation_prompt_body(&request, NOW);

        assert!(body.contains("keychain:smith/personal is spent, and resets in 1h"));
        assert!(body.contains("2. keychain:smith/work — usage unknown"));
        // The cache cost is stated, not implied.
        assert!(body.contains("without the provider's prompt cache"));
        assert!(body.contains("spends the other account's budget"));
    }

    #[test]
    fn a_threshold_offer_explains_which_threshold_was_crossed() {
        let mut outgoing = member(0, "keychain:smith/personal");
        outgoing.used_percent = Some(93.0);
        let request = RotationRequest {
            provider: "acme".to_owned(),
            trigger: RotationTrigger::Threshold { percent: 90 },
            outgoing,
            eligible: vec![member(1, "keychain:smith/work")],
            outgoing_resets_at_ms: None,
        };

        let body = rotation_prompt_body(&request, NOW);
        assert!(body.contains("is at 93% used, past the 90% you configured"));
        assert!(body.contains("without the provider's prompt cache"));
    }

    #[test]
    fn a_modal_with_no_reported_reset_does_not_invent_one() {
        let request = RotationRequest {
            provider: "acme".to_owned(),
            trigger: RotationTrigger::Exhausted,
            outgoing: member(0, "keychain:smith/personal"),
            eligible: vec![member(1, "keychain:smith/work")],
            outgoing_resets_at_ms: None,
        };

        let body = rotation_prompt_body(&request, NOW);
        assert!(body.contains("keychain:smith/personal is spent."));
        assert!(!body.contains("resets"));
    }

    #[test]
    fn transcript_notices_distinguish_a_manual_switch_from_a_rotation() {
        assert_eq!(
            switch_notice("a", "b", true),
            "switched from a to b"
        );
        assert_eq!(
            switch_notice("a", "b", false),
            "rotated from a to b"
        );
    }

    #[test]
    fn a_refusal_records_where_the_session_stayed() {
        assert_eq!(
            declined_notice("keychain:smith/personal", Some(NOW + HOUR_MS), NOW),
            "stayed on keychain:smith/personal; its window resets in 1h"
        );
        assert_eq!(
            declined_notice("keychain:smith/personal", None, NOW),
            "stayed on keychain:smith/personal"
        );
    }
}
