// Answering a credential-rotation offer. Included into the `tests` module.

use smith_host::rotation::{
    InteractiveRotation, RotationDecision, RotationMember, RotationPolicy, RotationRequest,
    RotationTrigger,
};

const ROTATION_NOW: u64 = 1_785_862_800_000;

fn member(position: usize, label: &str) -> RotationMember {
    RotationMember {
        position,
        label: label.to_owned(),
        used_percent: None,
        cooling_until_ms: None,
    }
}

fn offer(eligible: Vec<RotationMember>) -> RotationRequest {
    let mut outgoing = member(0, "keychain:smith/personal");
    outgoing.used_percent = Some(100.0);
    RotationRequest {
        provider: "acme".to_owned(),
        trigger: RotationTrigger::Exhausted,
        outgoing,
        eligible,
        outgoing_resets_at_ms: Some(ROTATION_NOW + 3_600_000),
    }
}

/// Presents `request` on `app` and returns the policy's answer.
async fn answer(
    app: &mut App,
    request: RotationRequest,
    press: KeyCode,
) -> RotationDecision {
    let (policy, mut requests) = InteractiveRotation::new(1);
    let decision = tokio::spawn(async move { policy.decide(&request).await });
    let prompt = requests.recv().await.expect("an offer reached the surface");
    app.present_rotation(prompt);
    app.on_key(key(press));
    decision.await.expect("the policy answered")
}

#[tokio::test]
async fn the_modal_states_the_cache_cost_before_the_user_agrees() {
    let mut app = app();
    let (policy, mut requests) = InteractiveRotation::new(1);
    let request = offer(vec![member(1, "keychain:smith/work")]);
    let pending = tokio::spawn(async move { policy.decide(&request).await });
    let prompt = requests.recv().await.expect("an offer");
    app.present_rotation(prompt);

    let Some(Overlay::RotationConfirm { content, .. }) = &app.overlay else {
        panic!("the offer must own the surface it is asking about");
    };
    assert!(content.contains("keychain:smith/personal is spent"));
    assert!(content.contains("without the provider's prompt cache"));
    // No credential value ever reaches the surface.
    assert!(!content.contains("sk-"));

    app.on_key(key(KeyCode::Char('n')));
    pending.await.expect("the policy answered");
}

#[tokio::test]
async fn confirming_switches_to_the_first_offered_account() {
    let mut app = app();
    let decision = answer(
        &mut app,
        offer(vec![member(1, "keychain:smith/work")]),
        KeyCode::Char('y'),
    )
    .await;

    assert_eq!(
        decision,
        RotationDecision::Switch { position: 1 }
    );
    assert!(app.overlay.is_none(), "the answered offer is dismissed");
    let transcript = format!("{:?}", app.transcript);
    assert!(transcript.contains("rotated from keychain:smith/personal to keychain:smith/work"));
}

#[tokio::test]
async fn declining_keeps_the_account_and_records_the_reset() {
    let mut app = app();
    let decision = answer(
        &mut app,
        offer(vec![member(1, "keychain:smith/work")]),
        KeyCode::Char('n'),
    )
    .await;

    assert_eq!(decision, RotationDecision::Decline);
    let transcript = format!("{:?}", app.transcript);
    assert!(transcript.contains("stayed on keychain:smith/personal"));
    // The user is told when work can resume, not only that it cannot.
    assert!(transcript.contains("resets"));
}

#[tokio::test]
async fn escape_declines_rather_than_leaving_the_turn_blocked() {
    let mut app = app();
    let decision = answer(
        &mut app,
        offer(vec![member(1, "keychain:smith/work")]),
        KeyCode::Esc,
    )
    .await;

    assert_eq!(decision, RotationDecision::Decline);
    assert!(app.overlay.is_none());
}

#[tokio::test]
async fn a_number_selects_the_account_the_modal_printed() {
    let mut app = app();
    // Two eligible members, listed as 2 and 3 (positions 1 and 2).
    let decision = answer(
        &mut app,
        offer(vec![
            member(1, "keychain:smith/work"),
            member(2, "keychain:smith/spare"),
        ]),
        KeyCode::Char('3'),
    )
    .await;

    assert_eq!(
        decision,
        RotationDecision::Switch { position: 2 }
    );
    let transcript = format!("{:?}", app.transcript);
    assert!(transcript.contains("to keychain:smith/spare"));
}

#[tokio::test]
async fn a_number_naming_no_offered_account_is_ignored() {
    let mut app = app();
    let (policy, mut requests) = InteractiveRotation::new(1);
    let request = offer(vec![member(1, "keychain:smith/work")]);
    let pending = tokio::spawn(async move { policy.decide(&request).await });
    let prompt = requests.recv().await.expect("an offer");
    app.present_rotation(prompt);

    // Position 8 was never offered: a mistyped digit must not spend the turn.
    app.on_key(key(KeyCode::Char('9')));
    assert!(
        matches!(app.overlay, Some(Overlay::RotationConfirm { .. })),
        "an unoffered number leaves the question open"
    );

    app.on_key(key(KeyCode::Char('y')));
    assert_eq!(
        pending.await.expect("the policy answered"),
        RotationDecision::Switch { position: 1 }
    );
}
