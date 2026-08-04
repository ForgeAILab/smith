//! The credential-rotation gate.
//!
//! When a provider reports that the active account's usage window is spent,
//! Smith can replay the attempt on another account in the same pool. It asks
//! first. Rotating abandons the provider-side prompt cache, so the replayed
//! turn resubmits its whole context uncached — a cost the user pays in tokens
//! and latency — and it spends a second account's budget. Neither should
//! happen because a background policy decided it silently.
//!
//! The shape deliberately mirrors [`crate::approval`], because the
//! requirements are the same ones:
//!
//! - **The channel is the seam.** [`InteractiveRotation`] knows nothing about
//!   rendering, so the same policy serves the TUI, a test, or a future host.
//! - **Fail closed on a dead surface.** A missing or crashed surface declines
//!   rotation rather than switching accounts unattended.
//! - **Headless never rotates.** [`HeadlessRotation`] records the exhaustion
//!   and declines, so a script's credential cannot change under it mid-run.
//! - **Nothing carries a secret.** A prompt names members by pool position and
//!   display label; a credential reference's *value* never reaches a surface.

use std::sync::Mutex;

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

/// One pool member as a surface may show it.
///
/// Everything here is safe to render: a position, a label derived from the
/// reference's non-secret parts, and server-reported usage. The credential
/// reference itself never appears.
#[derive(Debug, Clone, PartialEq)]
pub struct RotationMember {
    /// Zero-based position in the declared pool.
    pub position: usize,
    /// A redaction-safe display label, e.g. `keychain:smith/work`.
    pub label: String,
    /// Server-reported consumption, when a snapshot has been observed.
    ///
    /// `None` means unknown — never zero, and never full.
    pub used_percent: Option<f64>,
    /// When this member's window reopens, in Unix milliseconds, if it is
    /// cooling down.
    pub cooling_until_ms: Option<u64>,
}

impl RotationMember {
    /// Whether the member can serve an attempt now.
    pub fn is_eligible(&self) -> bool {
        self.cooling_until_ms.is_none()
    }
}

/// Why the runtime is offering to change accounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationTrigger {
    /// The active member's usage window is spent.
    Exhausted,
    /// The active member crossed the configured proactive threshold.
    Threshold {
        /// The configured percentage.
        percent: u8,
    },
}

/// A request to change the active pool member.
#[derive(Debug, Clone, PartialEq)]
pub struct RotationRequest {
    /// The provider whose pool this is.
    pub provider: String,
    /// Why rotation is being offered.
    pub trigger: RotationTrigger,
    /// The member in use now.
    pub outgoing: RotationMember,
    /// The members that could serve the attempt, in pool order. Never empty:
    /// with nothing to rotate to, the runtime fails instead of asking.
    pub eligible: Vec<RotationMember>,
    /// When the outgoing member's window reopens, in Unix milliseconds.
    pub outgoing_resets_at_ms: Option<u64>,
}

/// What the surface decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RotationDecision {
    /// Replay the attempt on the member at this pool position.
    Switch {
        /// The chosen member's position.
        position: usize,
    },
    /// Keep the active member and let the turn fail.
    Decline,
    /// No surface could answer, which is declining by another name but worth
    /// distinguishing in machine output.
    Unavailable {
        /// A redaction-safe explanation.
        reason: String,
    },
}

impl RotationDecision {
    /// Whether the decision changes the active member.
    pub fn switches(&self) -> bool {
        matches!(self, Self::Switch { .. })
    }
}

/// Decides whether an exhausted attempt moves to another account.
#[async_trait]
pub trait RotationPolicy: Send + Sync + std::fmt::Debug {
    /// Answers a rotation offer.
    async fn decide(&self, request: &RotationRequest) -> RotationDecision;
}

/// Redaction-safe evidence that an unattended run hit a spent window.
#[derive(Debug, Clone, PartialEq)]
pub struct RotationRequired {
    /// The provider whose pool was exhausted.
    pub provider: String,
    /// The label of the member that was in use.
    pub outgoing: String,
    /// Its pool position.
    pub position: usize,
    /// When its window reopens, in Unix milliseconds, if reported.
    pub resets_at_ms: Option<u64>,
    /// How many other members could have served the attempt.
    pub eligible: usize,
}

/// A non-interactive policy that never rotates and records why.
///
/// A headless run picks its member once at session start and keeps it. Two
/// reasons: a script whose credential changes mid-run produces results its
/// author cannot attribute to an account, and there is no surface to answer a
/// prompt, so "ask first" would resolve to "switch silently".
#[derive(Debug, Default)]
pub struct HeadlessRotation {
    required: Mutex<Option<RotationRequired>>,
}

impl HeadlessRotation {
    /// Creates a fail-closed headless policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// The first exhaustion this run reached, if any.
    pub fn required(&self) -> Option<RotationRequired> {
        self.required
            .lock()
            .expect("headless rotation state poisoned")
            .clone()
    }
}

#[async_trait]
impl RotationPolicy for HeadlessRotation {
    async fn decide(&self, request: &RotationRequest) -> RotationDecision {
        let mut required = self
            .required
            .lock()
            .expect("headless rotation state poisoned");
        if required.is_none() {
            *required = Some(RotationRequired {
                provider: request.provider.clone(),
                outgoing: request.outgoing.label.clone(),
                position: request.outgoing.position,
                resets_at_ms: request.outgoing_resets_at_ms,
                eligible: request.eligible.len(),
            });
        }
        RotationDecision::Unavailable {
            reason: "headless runs keep the account they started on; rerun once the window resets \
                     or select another account interactively"
                .to_owned(),
        }
    }
}

/// One rotation offer handed to the user-facing surface, with the channel to
/// answer it on.
///
/// Dropping a prompt without answering declines. That keeps the closed path
/// the default: an abandoned prompt cannot quietly spend a second account.
#[derive(Debug)]
pub struct RotationPrompt {
    request: RotationRequest,
    responder: Option<oneshot::Sender<RotationDecision>>,
}

impl RotationPrompt {
    /// Switches to the member at `position` and replays the attempt.
    pub fn switch_to(mut self, position: usize) {
        if let Some(responder) = self.responder.take() {
            let _ = responder.send(RotationDecision::Switch { position });
        }
    }

    /// Keeps the active member and lets the turn fail.
    pub fn decline(mut self) {
        if let Some(responder) = self.responder.take() {
            let _ = responder.send(RotationDecision::Decline);
        }
    }

    /// What is being offered.
    pub fn request(&self) -> &RotationRequest {
        &self.request
    }
}

impl Drop for RotationPrompt {
    fn drop(&mut self) {
        if let Some(responder) = self.responder.take() {
            let _ = responder.send(RotationDecision::Decline);
        }
    }
}

/// The receiving half: the surface that shows rotation offers to the user.
#[derive(Debug)]
pub struct RotationRequests {
    rx: mpsc::Receiver<RotationPrompt>,
}

impl RotationRequests {
    /// Waits for the next offer, or `None` once the runtime is gone.
    pub async fn recv(&mut self) -> Option<RotationPrompt> {
        self.rx.recv().await
    }

    /// Takes a pending offer without waiting.
    pub fn try_recv(&mut self) -> Option<RotationPrompt> {
        self.rx.try_recv().ok()
    }
}

/// A rotation policy that asks the user.
#[derive(Debug)]
pub struct InteractiveRotation {
    tx: mpsc::Sender<RotationPrompt>,
}

impl InteractiveRotation {
    /// Creates the policy and the receiver its offers arrive on.
    pub fn new(capacity: usize) -> (Self, RotationRequests) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        (Self { tx }, RotationRequests { rx })
    }
}

#[async_trait]
impl RotationPolicy for InteractiveRotation {
    async fn decide(&self, request: &RotationRequest) -> RotationDecision {
        let (responder, answer) = oneshot::channel();
        let prompt = RotationPrompt {
            request: request.clone(),
            responder: Some(responder),
        };
        if self.tx.send(prompt).await.is_err() {
            return RotationDecision::Unavailable {
                reason: "no surface is available to confirm a change of account".to_owned(),
            };
        }
        answer.await.unwrap_or(RotationDecision::Decline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(position: usize, label: &str) -> RotationMember {
        RotationMember {
            position,
            label: label.to_owned(),
            used_percent: None,
            cooling_until_ms: None,
        }
    }

    fn request() -> RotationRequest {
        RotationRequest {
            provider: "acme".to_owned(),
            trigger: RotationTrigger::Exhausted,
            outgoing: member(0, "keychain:smith/personal"),
            eligible: vec![member(1, "keychain:smith/work")],
            outgoing_resets_at_ms: Some(1_785_862_800_000),
        }
    }

    #[tokio::test]
    async fn a_headless_run_never_rotates_and_records_the_exhaustion() {
        let policy = HeadlessRotation::new();
        let decision = policy.decide(&request()).await;

        assert!(!decision.switches());
        assert!(matches!(decision, RotationDecision::Unavailable { .. }));

        let required = policy.required().expect("a recorded exhaustion");
        assert_eq!(required.provider, "acme");
        assert_eq!(required.outgoing, "keychain:smith/personal");
        assert_eq!(required.resets_at_ms, Some(1_785_862_800_000));
        assert_eq!(required.eligible, 1);
    }

    #[tokio::test]
    async fn only_the_first_headless_exhaustion_is_recorded() {
        let policy = HeadlessRotation::new();
        policy.decide(&request()).await;
        let mut second = request();
        second.outgoing = member(1, "keychain:smith/work");
        policy.decide(&second).await;

        // The run's stable outcome is where it stopped first, not wherever it
        // happened to end up.
        assert_eq!(
            policy.required().expect("a recorded exhaustion").outgoing,
            "keychain:smith/personal"
        );
    }

    #[tokio::test]
    async fn confirming_the_prompt_switches_to_the_chosen_member() {
        let (policy, mut requests) = InteractiveRotation::new(1);
        let surface = tokio::spawn(async move {
            let prompt = requests.recv().await.expect("an offer");
            assert_eq!(prompt.request().eligible.len(), 1);
            assert_eq!(prompt.request().trigger, RotationTrigger::Exhausted);
            prompt.switch_to(1);
        });

        let decision = policy.decide(&request()).await;
        surface.await.expect("the surface finished");
        assert_eq!(decision, RotationDecision::Switch { position: 1 });
    }

    #[tokio::test]
    async fn declining_the_prompt_keeps_the_active_member() {
        let (policy, mut requests) = InteractiveRotation::new(1);
        let surface = tokio::spawn(async move {
            requests.recv().await.expect("an offer").decline();
        });

        let decision = policy.decide(&request()).await;
        surface.await.expect("the surface finished");
        assert_eq!(decision, RotationDecision::Decline);
    }

    #[tokio::test]
    async fn an_abandoned_prompt_declines_rather_than_switching() {
        let (policy, mut requests) = InteractiveRotation::new(1);
        let surface = tokio::spawn(async move {
            // Received and dropped without an answer: the user closed the
            // surface, or shutdown raced the offer.
            drop(requests.recv().await.expect("an offer"));
        });

        let decision = policy.decide(&request()).await;
        surface.await.expect("the surface finished");
        assert_eq!(decision, RotationDecision::Decline);
    }

    #[tokio::test]
    async fn a_dead_surface_is_unavailable_rather_than_a_silent_switch() {
        let (policy, requests) = InteractiveRotation::new(1);
        drop(requests);

        let decision = policy.decide(&request()).await;
        assert!(!decision.switches());
        assert!(matches!(decision, RotationDecision::Unavailable { .. }));
    }

    #[test]
    fn a_cooling_member_is_not_eligible() {
        let mut cooling = member(1, "keychain:smith/work");
        cooling.cooling_until_ms = Some(1_785_862_800_000);
        assert!(!cooling.is_eligible());
        assert!(member(1, "keychain:smith/work").is_eligible());
    }
}
