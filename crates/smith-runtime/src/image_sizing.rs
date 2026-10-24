//! Provider- and model-aware estimates for images in planned requests.
//!
//! Image dimensions are read from bounded data-URL headers and fitted with the
//! same pixel limits as request-time preparation. All other wire accounting
//! retains the shared runtime's default character-ratio estimator.

use agent_runtime::context::{
    CharRatioSizer, ContextFragment, EstimationConfidence, FragmentContent, RequestSizer,
};
use agent_runtime::registry::{RegistryId, RegistryRevision};
use agent_runtime_core::catalog::ComponentRef;
use agent_runtime_core::content::{ContentPart, Message, Role};
use agent_runtime_core::provider::ToolSchema;
use smith_tools::image_header::describe_image_url;
use smith_tools::image_preparation::fitted_dimensions;
use smith_tools::{ImageLimits, ImageProvider};

/// Revision of Smith's image accounting formulas and model-regime selection.
pub const SMITH_IMAGE_SIZER_REVISION: &str = "smith-image-sizer-1";
/// Conservative estimate when an image's header dimensions cannot be read.
pub const UNKNOWN_IMAGE_TOKENS: u32 = 2_500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageTokenRegime {
    AnthropicHighRes,
    AnthropicStandard,
    OpenAiPatch,
    OpenAiTile4o,
    OpenAiTile5_1,
    Gemini,
}

impl ImageTokenRegime {
    fn for_provider_and_model(provider: ImageProvider, model: &str) -> Self {
        // Catalog IDs may be provider-qualified and use dots or underscores in
        // version numbers. Only the final model segment chooses a billing tier.
        let model = model
            .rsplit('/')
            .next()
            .unwrap_or(model)
            .to_ascii_lowercase()
            .replace(['.', '_'], "-");
        match provider {
            ImageProvider::Anthropic => {
                let standard = [
                    "claude-2",
                    "claude-instant",
                    "claude-3",
                    "claude-4-0",
                    "claude-4-1",
                    "claude-4-5",
                    "claude-4-6",
                ]
                .iter()
                .any(|prefix| model.starts_with(prefix))
                    || ["sonnet", "opus", "haiku"].iter().any(|family| {
                        let base = format!("claude-{family}-4");
                        model == base
                            || ["0", "1", "5", "6", "2025"]
                                .iter()
                                .any(|version| model.starts_with(&format!("{base}-{version}")))
                    });
                if standard {
                    Self::AnthropicStandard
                } else {
                    Self::AnthropicHighRes
                }
            }
            ImageProvider::OpenAi => {
                if ["gpt-4o", "chatgpt-4o", "gpt-4-1", "o1", "o3", "o4"]
                    .iter()
                    .any(|prefix| model.starts_with(prefix))
                {
                    Self::OpenAiTile4o
                } else if model == "gpt-5"
                    || [
                        "gpt-5-mini",
                        "gpt-5-nano",
                        "gpt-5-chat",
                        "gpt-5-pro",
                        "gpt-5-codex",
                        "gpt-5-0",
                        "gpt-5-1",
                        "gpt-5-2025",
                    ]
                    .iter()
                    .any(|prefix| model.starts_with(prefix))
                {
                    Self::OpenAiTile5_1
                } else {
                    Self::OpenAiPatch
                }
            }
            ImageProvider::Gemini => Self::Gemini,
            ImageProvider::Xai | ImageProvider::Other => Self::OpenAiPatch,
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::AnthropicHighRes => "anthropic-high-res",
            Self::AnthropicStandard => "anthropic-standard",
            Self::OpenAiPatch => "openai-patch",
            Self::OpenAiTile4o => "openai-tile-4o",
            Self::OpenAiTile5_1 => "openai-tile-5-1",
            Self::Gemini => "gemini",
        }
    }
}

/// Offline request estimator with provider-fitted image patch/tile accounting.
///
/// Unknown models use their provider's higher image tier. Text, signatures,
/// schemas, and framing retain [`CharRatioSizer::default()`] accounting.
#[derive(Debug, Clone)]
pub struct SmithRequestSizer {
    text: CharRatioSizer,
    limits: ImageLimits,
    regime: ImageTokenRegime,
}

impl SmithRequestSizer {
    /// Selects pixel limits and image billing rules for a resolved serving route.
    pub fn for_provider_and_model(kind: &str, model: &str) -> Self {
        let provider = crate::factory::image_provider_for_kind(kind);
        Self {
            text: CharRatioSizer::default(),
            limits: ImageLimits::for_provider(provider),
            regime: ImageTokenRegime::for_provider_and_model(provider, model),
        }
    }

    fn size_text(&self, text: &str) -> u32 {
        self.text
            .size_message(&Message::user(text))
            .saturating_sub(self.text.message_framing_tokens)
    }

    fn size_content_part(&self, part: &ContentPart) -> u32 {
        match part {
            ContentPart::Image { url, .. } => self.size_image(url),
            ContentPart::ToolResult(block) => block.content.iter().fold(
                self.text
                    .tool_call_framing_tokens
                    .saturating_add(self.size_text(&block.name)),
                |tokens, part| tokens.saturating_add(self.size_content_part(part)),
            ),
            // Delegate every non-image part, including reasoning signatures and
            // tool-call JSON, without charging a second message wrapper.
            _ => self
                .text
                .size_message(&Message {
                    role: Role::User,
                    content: vec![part.clone()],
                })
                .saturating_sub(self.text.message_framing_tokens),
        }
    }

    fn size_image(&self, url: &str) -> u32 {
        let Some((width, height)) = describe_image_url(url).dimensions else {
            return UNKNOWN_IMAGE_TOKENS;
        };
        let Ok((mut width, mut height)) = fitted_dimensions(width, height, &self.limits) else {
            return UNKNOWN_IMAGE_TOKENS;
        };
        match self.regime {
            ImageTokenRegime::AnthropicHighRes => {
                (width, height) = fit_long_edge(width, height, 2_576);
                (width.div_ceil(28) * height.div_ceil(28)).min(4_784)
            }
            ImageTokenRegime::AnthropicStandard => {
                (width, height) = fit_long_edge(width, height, 1_568);
                let area = u64::from(width) * u64::from(height);
                let budget_area = 1_568 * 28 * 28;
                if area > budget_area {
                    let scale = (budget_area as f64 / area as f64).sqrt();
                    (width, height) = scale_dimensions(width, height, scale);
                }
                (width.div_ceil(28) * height.div_ceil(28)).min(1_568)
            }
            ImageTokenRegime::OpenAiPatch => {
                if self.limits.tile_budget.is_none() {
                    // xAI/compatible uploads have no patch budget, but their
                    // conservative estimate uses OpenAI's exact shrink math.
                    let limits = ImageLimits {
                        tile_budget: Some(2_500),
                        ..self.limits.clone()
                    };
                    let Ok(fitted) = fitted_dimensions(width, height, &limits) else {
                        return UNKNOWN_IMAGE_TOKENS;
                    };
                    (width, height) = fitted;
                }
                let patches = u64::from(width.div_ceil(32)) * u64::from(height.div_ceil(32));
                (patches * 6).div_ceil(5) as u32
            }
            ImageTokenRegime::OpenAiTile4o | ImageTokenRegime::OpenAiTile5_1 => {
                if width.min(height) > 768 {
                    let scale = 768.0 / f64::from(width.min(height));
                    (width, height) = scale_dimensions(width, height, scale);
                }
                let tiles = width.div_ceil(512) * height.div_ceil(512);
                if self.regime == ImageTokenRegime::OpenAiTile4o {
                    85 + 170 * tiles
                } else {
                    70 + 140 * tiles
                }
            }
            ImageTokenRegime::Gemini => {
                if width <= 384 && height <= 384 {
                    258
                } else {
                    width.div_ceil(768) * height.div_ceil(768) * 258
                }
            }
        }
    }
}

fn scale_dimensions(width: u32, height: u32, scale: f64) -> (u32, u32) {
    (
        (f64::from(width) * scale).round().max(1.0) as u32,
        (f64::from(height) * scale).round().max(1.0) as u32,
    )
}

fn fit_long_edge(width: u32, height: u32, edge: u32) -> (u32, u32) {
    let scale = (f64::from(edge) / f64::from(width.max(height))).min(1.0);
    scale_dimensions(width, height, scale)
}

impl RequestSizer for SmithRequestSizer {
    fn size_fragment(&self, fragment: &ContextFragment) -> u32 {
        match &fragment.content {
            FragmentContent::Message(message) => self.size_message(message),
            FragmentContent::Tool(_) | FragmentContent::Text(_) => {
                self.text.size_fragment(fragment)
            }
        }
    }

    fn size_message(&self, message: &Message) -> u32 {
        message
            .content
            .iter()
            .fold(self.text.message_framing_tokens, |tokens, part| {
                tokens.saturating_add(self.size_content_part(part))
            })
    }

    fn size_tool_schema(&self, schema: &ToolSchema) -> u32 {
        self.text.size_tool_schema(schema)
    }

    fn revision(&self) -> ComponentRef {
        ComponentRef::new(
            RegistryId::tokenizer("smith-runtime/image-aware-char-ratio"),
            // Text accounting is delegated, so its revision must move ours.
            RegistryRevision::new(format!(
                "{SMITH_IMAGE_SIZER_REVISION}:{}:{}",
                self.regime.slug(),
                self.text.revision().revision.as_str()
            )),
        )
    }

    fn confidence(&self) -> EstimationConfidence {
        EstimationConfidence::Estimated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime::context::{FragmentKind, FragmentSource};
    use agent_runtime_core::content::{ToolCall, ToolResultBlock};
    use agent_runtime_core::ids::ToolCallId;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use smith_config::model::{
        KIND_ANTHROPIC_MESSAGES, KIND_CHATGPT_RESPONSES, KIND_GEMINI_INTERACTIONS,
        KIND_OPENAI_COMPATIBLE, KIND_OPENAI_RESPONSES, KIND_XAI_RESPONSES,
    };

    fn image_part(url: impl Into<String>) -> ContentPart {
        ContentPart::Image {
            url: url.into(),
            detail: None,
        }
    }

    fn png_header_data_url(width: u32, height: u32) -> String {
        let mut header = Vec::from(&b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"[..]);
        header.extend_from_slice(&width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        format!("data:image/png;base64,{}", STANDARD.encode(header))
    }

    fn image_tokens(kind: &str, model: &str, width: u32, height: u32) -> u32 {
        let sizer = SmithRequestSizer::for_provider_and_model(kind, model);
        let framing = sizer.size_message(&Message::user(""));
        let message = Message {
            role: Role::User,
            content: vec![image_part(png_header_data_url(width, height))],
        };
        sizer.size_message(&message) - framing
    }

    #[test]
    fn worked_examples_match_provider_documentation() {
        // Anthropic high-res (Claude 4.7+ and unknown models): 28x28 patches, 2576px / 4784 tokens.
        for model in [
            "claude-sonnet-4-7",
            "anthropic/claude-fable-5.1",
            "custom-claude",
        ] {
            assert_eq!(
                image_tokens(KIND_ANTHROPIC_MESSAGES, model, 200, 200),
                64,
                "{model}"
            );
            assert_eq!(
                image_tokens(KIND_ANTHROPIC_MESSAGES, model, 1000, 1000),
                1_296,
                "{model}"
            );
            assert_eq!(
                image_tokens(KIND_ANTHROPIC_MESSAGES, model, 1920, 1080),
                2_691,
                "{model}"
            );
            assert_eq!(
                image_tokens(KIND_ANTHROPIC_MESSAGES, model, 3840, 2160),
                4_784,
                "{model}"
            );
        }

        // Anthropic standard (known pre-4.7 Claude models): 28x28 patches, 1568px / 1568 tokens.
        for model in [
            "claude-3-5-sonnet-20241022",
            "claude-sonnet-4-0",
            "claude-sonnet-4-5-20250929",
            "anthropic/claude-opus-4.6",
        ] {
            assert_eq!(
                image_tokens(KIND_ANTHROPIC_MESSAGES, model, 200, 200),
                64,
                "{model}"
            );
            assert_eq!(
                image_tokens(KIND_ANTHROPIC_MESSAGES, model, 1000, 1000),
                1_296,
                "{model}"
            );
            let hd = image_tokens(KIND_ANTHROPIC_MESSAGES, model, 1920, 1080);
            assert!((1_400..=1_568).contains(&hd), "{model} 1080p: {hd}");
            let uhd = image_tokens(KIND_ANTHROPIC_MESSAGES, model, 3840, 2160);
            assert!((1_400..=1_568).contains(&uhd), "{model} 4k: {uhd}");
        }

        // OpenAI / ChatGPT patch models (gpt-5.2+, gpt-6*, and unknown models).
        for (kind, model) in [
            (KIND_OPENAI_RESPONSES, "gpt-5.6"),
            (KIND_CHATGPT_RESPONSES, "gpt-6-sol"),
            (KIND_OPENAI_RESPONSES, "gpt-5.2"),
            (KIND_OPENAI_RESPONSES, "unknown-future-model"),
        ] {
            assert_eq!(image_tokens(kind, model, 200, 200), 59, "{model}");
            assert_eq!(image_tokens(kind, model, 1024, 1024), 1_229, "{model}");
            assert_eq!(image_tokens(kind, model, 1920, 1080), 2_448, "{model}");
            assert_eq!(image_tokens(kind, model, 2048, 2048), 3_000, "{model}");
            assert_eq!(image_tokens(kind, model, 3840, 2160), 2_765, "{model}");
        }

        // OpenAI legacy tile models: gpt-4o / gpt-4.1 (85 + 170 * tiles) and gpt-5.1 (70 + 140 * tiles).
        for model in ["gpt-4o", "gpt-4.1", "o3"] {
            assert_eq!(
                image_tokens(KIND_OPENAI_RESPONSES, model, 200, 200),
                255,
                "{model}"
            );
            assert_eq!(
                image_tokens(KIND_OPENAI_RESPONSES, model, 1920, 1080),
                1_105,
                "{model}"
            );
        }
        for model in ["gpt-5.1", "gpt-5", "gpt-5-mini"] {
            assert_eq!(
                image_tokens(KIND_OPENAI_RESPONSES, model, 200, 200),
                210,
                "{model}"
            );
            assert_eq!(
                image_tokens(KIND_OPENAI_RESPONSES, model, 1920, 1080),
                910,
                "{model}"
            );
        }

        // Gemini: 258 tokens when <=384px both sides, else ceil(w/768) * ceil(h/768) * 258.
        assert_eq!(
            image_tokens(KIND_GEMINI_INTERACTIONS, "gemini-2.5-pro", 200, 200),
            258
        );
        assert_eq!(
            image_tokens(KIND_GEMINI_INTERACTIONS, "gemini-2.5-pro", 384, 384),
            258
        );
        assert_eq!(
            image_tokens(KIND_GEMINI_INTERACTIONS, "gemini-2.5-pro", 1920, 1080),
            1_548
        );
        assert_eq!(
            image_tokens(KIND_GEMINI_INTERACTIONS, "gemini-2.5-pro", 3840, 2160),
            3_096
        );

        // xAI and OpenAI-compatible fallback: OpenAI patch formula on 2048px fit.
        for (kind, model) in [
            (KIND_XAI_RESPONSES, "grok-4"),
            (KIND_OPENAI_COMPATIBLE, "glm-5.3"),
        ] {
            assert_eq!(image_tokens(kind, model, 1920, 1080), 2_448, "{kind}");
            assert_eq!(image_tokens(kind, model, 3840, 2160), 2_765, "{kind}");
        }
    }

    #[test]
    fn remote_and_unreadable_images_use_conservative_fallback() {
        let sizer =
            SmithRequestSizer::for_provider_and_model(KIND_ANTHROPIC_MESSAGES, "claude-sonnet-4-7");
        let framing = sizer.size_message(&Message::user(""));
        for url in [
            "https://example.com/screenshot.png",
            "data:image/png;base64,not-an-image",
            "data:text/plain;base64,aGVsbG8=",
        ] {
            let message = Message {
                role: Role::User,
                content: vec![image_part(url)],
            };
            assert_eq!(
                sizer.size_message(&message) - framing,
                UNKNOWN_IMAGE_TOKENS,
                "{url}"
            );
        }
    }

    #[test]
    fn tool_result_images_and_non_image_parts_size_accurately() {
        let sizer =
            SmithRequestSizer::for_provider_and_model(KIND_ANTHROPIC_MESSAGES, "claude-sonnet-4-7");
        let baseline = CharRatioSizer::default();

        let text_and_reasoning = Message {
            role: Role::Assistant,
            content: vec![
                ContentPart::Reasoning {
                    text: "checking screenshot".into(),
                    redacted: false,
                    signature: Some("opaque-signature-payload".into()),
                    producer: None,
                },
                ContentPart::ToolCall(ToolCall {
                    id: ToolCallId::new("call-1"),
                    name: "read".into(),
                    arguments: serde_json::json!({"path": "screen.png"}),
                }),
            ],
        };
        assert_eq!(
            sizer.size_message(&text_and_reasoning),
            baseline.size_message(&text_and_reasoning)
        );

        let schema = ToolSchema {
            name: "read".into(),
            description: "Read a file or image".into(),
            input_schema: serde_json::json!({"type": "object"}),
        };
        assert_eq!(
            sizer.size_tool_schema(&schema),
            baseline.size_tool_schema(&schema)
        );

        let text_only_result = Message {
            role: Role::Tool,
            content: vec![ContentPart::ToolResult(ToolResultBlock {
                call_id: ToolCallId::new("call-1"),
                name: "read".into(),
                content: vec![ContentPart::text("screen.png: 1920x1080 PNG")],
                is_error: false,
            })],
        };
        let with_image_result = Message {
            role: Role::Tool,
            content: vec![ContentPart::ToolResult(ToolResultBlock {
                call_id: ToolCallId::new("call-1"),
                name: "read".into(),
                content: vec![
                    ContentPart::text("screen.png: 1920x1080 PNG"),
                    image_part(png_header_data_url(1920, 1080)),
                ],
                is_error: false,
            })],
        };
        assert_eq!(
            sizer.size_message(&with_image_result) - sizer.size_message(&text_only_result),
            2_691
        );

        let fragment = ContextFragment::new(
            "history:1",
            FragmentKind::ToolResult,
            FragmentSource::History,
            RegistryRevision::new("test-1"),
            FragmentContent::Message(with_image_result.clone()),
        );
        assert_eq!(
            sizer.size_fragment(&fragment),
            sizer.size_message(&with_image_result)
        );
    }

    #[test]
    fn revision_tracks_provider_and_model_regime() {
        let revisions = [
            SmithRequestSizer::for_provider_and_model(KIND_ANTHROPIC_MESSAGES, "claude-sonnet-4-7")
                .revision(),
            SmithRequestSizer::for_provider_and_model(KIND_ANTHROPIC_MESSAGES, "claude-3-5-sonnet")
                .revision(),
            SmithRequestSizer::for_provider_and_model(KIND_OPENAI_RESPONSES, "gpt-5.6").revision(),
            SmithRequestSizer::for_provider_and_model(KIND_OPENAI_RESPONSES, "gpt-4o").revision(),
            SmithRequestSizer::for_provider_and_model(KIND_OPENAI_RESPONSES, "gpt-5.1").revision(),
            SmithRequestSizer::for_provider_and_model(KIND_GEMINI_INTERACTIONS, "gemini-2.5-pro")
                .revision(),
        ];
        for (index, left) in revisions.iter().enumerate() {
            for right in &revisions[index + 1..] {
                assert_ne!(left, right);
            }
        }
        let text = CharRatioSizer::default().revision().revision;
        assert!(
            revisions
                .iter()
                .all(|revision| revision.revision.as_str().ends_with(text.as_str()))
        );
    }
}
