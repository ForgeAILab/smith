use super::*;

#[test]
fn image_input_limits_follow_provider_adapter_kind() {
    use smith_tools::{ImageLimits, ImageProvider};
    for (kind, provider) in [
        (KIND_XAI_RESPONSES, ImageProvider::Xai),
        (KIND_CHATGPT_RESPONSES, ImageProvider::OpenAi),
        (KIND_OPENAI_RESPONSES, ImageProvider::OpenAi),
        (KIND_ANTHROPIC_MESSAGES, ImageProvider::Anthropic),
        (KIND_GEMINI_INTERACTIONS, ImageProvider::Gemini),
        (KIND_OPENAI_COMPATIBLE, ImageProvider::Other),
        (KIND_COMMAND_JSONL, ImageProvider::Other),
        (KIND_FAKE, ImageProvider::Other),
        ("cli/codex", ImageProvider::Other),
        ("unknown", ImageProvider::Other),
    ] {
        assert_eq!(
            image_limits_for_provider_kind(kind),
            ImageLimits::for_provider(provider),
            "{kind}"
        );
    }
}

#[test]
fn planner_sizes_inline_images_by_provider_and_windowed_images_as_placeholders() {
    use agent_runtime::context::{
        ContextFragment, ContextPlanner, FragmentContent, FragmentKind, FragmentSource,
        ImageWindow, RequestSizer,
    };
    use agent_runtime::registry::RegistryRevision;
    use agent_runtime_core::content::ContentPart;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;

    let mut header = Vec::from(&b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"[..]);
    header.extend_from_slice(&1920u32.to_be_bytes());
    header.extend_from_slice(&1080u32.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    let data_url = format!("data:image/png;base64,{}", STANDARD.encode(header));

    let mut config = resolved_config();
    config.context.inline_images = sourced(1);
    let mut model_profile = profile(ModelLimits::new(128_000, 124_000, 4_096));
    model_profile.input_modalities = vec![Modality::Text, Modality::Image];
    let output_budget = effective_output_budget(&config, &model_profile).expect("output budget");
    let policy = context_policy(&config, &model_profile.limits, &output_budget);
    assert_eq!(policy.image_window, Some(ImageWindow { inline: 1 }));

    let sizer = crate::image_sizing::SmithRequestSizer::for_provider_and_model(
        KIND_ANTHROPIC_MESSAGES,
        "claude-sonnet-4-7",
    );
    let planner = ContextPlanner::new(&model_profile, &sizer, policy);

    let make_image_fragment = |id: &str, sequence: u64| {
        ContextFragment::new(
            id,
            FragmentKind::History,
            FragmentSource::History,
            RegistryRevision::new("test-1"),
            FragmentContent::Message(Message {
                role: agent_runtime_core::content::Role::Assistant,
                content: vec![ContentPart::Image {
                    url: data_url.clone(),
                    detail: None,
                }],
            }),
        )
        .with_sequence(sequence)
    };

    // One inline 1920x1080 screenshot costs 2,691 image tokens + 4 framing tokens.
    let one_image_plan = planner
        .plan(vec![make_image_fragment("history:0", 0)])
        .expect("one image plan");
    assert_eq!(one_image_plan.input_tokens(), 2_695);
    assert_eq!(one_image_plan.sizer_revision(), &sizer.revision());

    // With inline_images = 1, the older screenshot is projected to a text
    // placeholder (~17 tokens) before sizing while the newer screenshot stays
    // inline at 2,691 tokens.
    let two_image_plan = planner
        .plan(vec![
            make_image_fragment("history:0", 0),
            make_image_fragment("history:1", 1),
        ])
        .expect("two image plan");
    let delta = two_image_plan.input_tokens() - one_image_plan.input_tokens();
    assert!(
        (15..40).contains(&delta),
        "windowed-out image should be sized as a short text placeholder, got delta {delta}"
    );
}
