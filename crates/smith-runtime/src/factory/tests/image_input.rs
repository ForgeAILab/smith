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
