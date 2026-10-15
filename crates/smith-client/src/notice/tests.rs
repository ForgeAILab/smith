use super::*;

use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::ids::{EventId, SessionId, TurnId};
use agent_runtime_registry::Fingerprint;
use smith_runtime::client::EstimationConfidence;

fn plan(session: &str, images_text_only: Option<u32>) -> SmithEvent {
    SmithEvent::new(
        1,
        EventId::new("plan"),
        SessionId::new(session),
        Some(TurnId::new("turn")),
        Timestamp::ZERO,
        SmithEventKind::ContextPlanned {
            context: Fingerprint::of("context"),
            cache_plan: Fingerprint::of("cache"),
            segment_count: 1,
            totals: Default::default(),
            input_tokens: 100,
            input_budget_tokens: 1_000,
            reserved_tokens: 100,
            confidence: EstimationConfidence::Exact,
            images_text_only,
            images_windowed: Some(2),
        },
    )
}

#[test]
fn image_input_notice_reports_unknown_metadata_once() {
    let mut notices = ImageInputNotices::default();
    let event = plan("root", Some(1));
    let notice = notices
        .observe(&event, "root", "local", "custom", ImageInput::Unknown)
        .expect("first replaced image on an unknown model");
    assert_eq!(notice.kind, NoticeKind::Provider);
    assert_eq!(
        notice.text,
        "local/custom has no image-input metadata, so images are sent as placeholders. \
         Set [models.\"local/custom\"] input_modalities = [\"text\", \"image\"] \
         if it accepts images."
    );
    assert!(
        notices
            .observe(&event, "root", "local", "custom", ImageInput::Unknown)
            .is_none()
    );
    assert!(
        notices
            .observe(
                &plan("root", Some(5)),
                "root",
                "local",
                "custom",
                ImageInput::Unknown,
            )
            .is_none()
    );
}

#[test]
fn image_input_notice_does_not_report_known_metadata() {
    let mut notices = ImageInputNotices::default();
    let event = plan("root", Some(1));
    for input in [ImageInput::TextOnly, ImageInput::Accepts] {
        assert!(
            notices
                .observe(&event, "root", "local", "custom", input)
                .is_none()
        );
    }
    assert!(
        notices
            .observe(&event, "root", "local", "custom", ImageInput::Unknown)
            .is_some()
    );
}

#[test]
fn image_input_notice_waits_for_a_positive_text_only_count() {
    let mut notices = ImageInputNotices::default();
    // A windowed image alone is not evidence of missing metadata. Older
    // plans without either counter are equally silent.
    for count in [None, Some(0)] {
        assert!(
            notices
                .observe(
                    &plan("root", count),
                    "root",
                    "local",
                    "custom",
                    ImageInput::Unknown,
                )
                .is_none()
        );
    }
    assert!(
        notices
            .observe(
                &plan("root", Some(1)),
                "root",
                "local",
                "custom",
                ImageInput::Unknown,
            )
            .is_some()
    );
}

#[test]
fn image_input_notice_ignores_child_plans_and_unrelated_events() {
    let mut notices = ImageInputNotices::default();
    assert!(
        notices
            .observe(
                &plan("child", Some(1)),
                "root",
                "local",
                "custom",
                ImageInput::Unknown,
            )
            .is_none()
    );
    let mut unrelated = plan("root", Some(1));
    unrelated.payload = SmithEventKind::TurnStarted;
    assert!(
        notices
            .observe(&unrelated, "root", "local", "custom", ImageInput::Unknown)
            .is_none()
    );
    assert!(
        notices
            .observe(
                &plan("root", Some(1)),
                "root",
                "local",
                "custom",
                ImageInput::Unknown,
            )
            .is_some()
    );
}

#[test]
fn image_input_notice_is_scoped_to_session_provider_and_model() {
    let mut notices = ImageInputNotices::default();
    for (session, provider, model) in [
        ("first", "local", "custom"),
        ("second", "local", "custom"),
        ("second", "local", "other"),
        ("second", "remote", "other"),
    ] {
        let event = plan(session, Some(1));
        assert!(
            notices
                .observe(&event, session, provider, model, ImageInput::Unknown)
                .is_some()
        );
        assert!(
            notices
                .observe(&event, session, provider, model, ImageInput::Unknown)
                .is_none()
        );
    }
    assert!(
        notices
            .observe(
                &plan("second", Some(1)),
                "second",
                "local",
                "custom",
                ImageInput::Unknown,
            )
            .is_none()
    );
}

#[test]
fn image_input_notice_accepts_older_plans_without_image_counts() {
    let mut value = serde_json::to_value(plan("root", Some(1))).expect("plan JSON");
    let payload = value["payload"].as_object_mut().expect("payload");
    payload.remove("images_text_only");
    payload.remove("images_windowed");
    let event: SmithEvent = serde_json::from_value(value).expect("older context plan");
    let mut notices = ImageInputNotices::default();
    assert!(
        notices
            .observe(&event, "root", "local", "custom", ImageInput::Unknown)
            .is_none()
    );
}

#[test]
fn hosted_search_rejection_notice_is_classified_and_once_per_session() {
    use agent_runtime_core::ids::AttemptId;
    use agent_runtime_core::provider::{FinishReason, ProviderError, ProviderErrorKind};
    let mut notices = HostedSearchNotices::default();
    let mut event = plan("root", None);
    event.payload = SmithEventKind::ProviderAttemptFinished {
        attempt: AttemptId::new("attempt"),
        index: None,
        max_attempts: None,
        finish: FinishReason::Error,
        retryable: false,
        retry_delay_ms: None,
        error: Some(ProviderError::new(
            ProviderErrorKind::Unsupported,
            "hosted web search is not enabled for this account",
        )),
    };
    assert!(notices.observe(&event, "other-session").is_none());
    let notice = notices.observe(&event, "root").unwrap();
    assert_eq!(notice.kind, NoticeKind::Provider);
    assert_eq!(
        notice.text,
        "Hosted web search is unavailable for this organization."
    );
    assert!(notices.observe(&event, "root").is_none());
}
