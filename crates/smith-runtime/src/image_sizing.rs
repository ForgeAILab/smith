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
pub const SMITH_IMAGE_SIZER_REVISION: &str = "smith-image-sizer-2";
/// Conservative estimate when an image's header dimensions cannot be read.
pub const UNKNOWN_IMAGE_TOKENS: u32 = 2_500;
/// Gemini 3 and 4 allot at most this many tokens per image at the default
/// (`unspecified`) media resolution, which Smith never overrides.
const GEMINI_DEFAULT_MEDIA_RESOLUTION_TOKENS: u32 = 1_120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageTokenRegime {
    AnthropicHighRes,
    AnthropicStandard,
    /// 32x32-pixel patches times a per-model multiplier, in hundredths.
    OpenAiPatch {
        multiplier_percent: u32,
    },
    /// 512x512 tiles after the 768 px shortest-side fit.
    OpenAiTile {
        base: u32,
        tile: u32,
    },
    /// Gemini 2.x and earlier: 258 tokens per 768x768 tile.
    GeminiTiles,
    /// Gemini 3 and 4: a flat per-image maximum set by media resolution.
    GeminiMediaResolution,
    /// Unrecognized Gemini models take the larger of the two Gemini rules.
    GeminiUnknown,
}

impl ImageTokenRegime {
    const OPENAI_CURRENT_PATCH: Self = Self::OpenAiPatch {
        multiplier_percent: 120,
    };

    fn for_provider_and_model(provider: ImageProvider, model: &str) -> Self {
        // Catalog IDs may be provider-qualified and use dots or underscores in
        // version numbers. Only the final model segment chooses a billing tier.
        let model = model
            .rsplit('/')
            .next()
            .unwrap_or(model)
            .to_ascii_lowercase()
            .replace(['.', '_'], "-");
        let starts_with_any = |prefixes: &[&str]| prefixes.iter().any(|p| model.starts_with(p));
        match provider {
            ImageProvider::Anthropic => {
                let standard = starts_with_any(&[
                    "claude-2",
                    "claude-instant",
                    "claude-3",
                    "claude-4-0",
                    "claude-4-1",
                    "claude-4-5",
                    "claude-4-6",
                ]) || ["sonnet", "opus", "haiku"].iter().any(|family| {
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
            // Rates from OpenAI's image-input cost tables. More specific
            // prefixes come first: `gpt-4o-mini` and the gpt-4.1 mini/nano
            // models do not bill like `gpt-4o` and `gpt-4.1`.
            ImageProvider::OpenAi => {
                if model.starts_with("gpt-4o-mini") {
                    Self::OpenAiTile {
                        base: 2_833,
                        tile: 5_667,
                    }
                } else if model.starts_with("gpt-4-1-mini") {
                    Self::OpenAiPatch {
                        multiplier_percent: 162,
                    }
                } else if model.starts_with("gpt-4-1-nano") {
                    Self::OpenAiPatch {
                        multiplier_percent: 246,
                    }
                } else if model.starts_with("o4-mini") {
                    Self::OpenAiPatch {
                        multiplier_percent: 172,
                    }
                } else if model.starts_with("gpt-5-nano") {
                    Self::OpenAiPatch {
                        multiplier_percent: 150,
                    }
                } else if model.starts_with("gpt-5-mini") {
                    Self::OPENAI_CURRENT_PATCH
                } else if starts_with_any(&["gpt-4o", "chatgpt-4o", "gpt-4-1"]) {
                    Self::OpenAiTile {
                        base: 85,
                        tile: 170,
                    }
                } else if starts_with_any(&["o1", "o3"]) {
                    Self::OpenAiTile {
                        base: 75,
                        tile: 150,
                    }
                } else if model == "gpt-5"
                    || starts_with_any(&[
                        "gpt-5-chat",
                        "gpt-5-pro",
                        "gpt-5-codex",
                        "gpt-5-0",
                        "gpt-5-1",
                        "gpt-5-2025",
                    ])
                {
                    Self::OpenAiTile {
                        base: 70,
                        tile: 140,
                    }
                } else {
                    Self::OPENAI_CURRENT_PATCH
                }
            }
            ImageProvider::Gemini => match gemini_major_version(&model) {
                Some(0..=2) => Self::GeminiTiles,
                Some(3 | 4) => Self::GeminiMediaResolution,
                _ => Self::GeminiUnknown,
            },
            ImageProvider::Xai | ImageProvider::Other => Self::OPENAI_CURRENT_PATCH,
        }
    }

    fn slug(self) -> String {
        match self {
            Self::AnthropicHighRes => "anthropic-high-res".into(),
            Self::AnthropicStandard => "anthropic-standard".into(),
            Self::OpenAiPatch { multiplier_percent } => {
                format!("openai-patch-x{multiplier_percent}")
            }
            Self::OpenAiTile { base, tile } => format!("openai-tile-{base}-{tile}"),
            Self::GeminiTiles => "gemini-tiles".into(),
            Self::GeminiMediaResolution => "gemini-media-resolution".into(),
            Self::GeminiUnknown => "gemini-unknown".into(),
        }
    }
}

/// Major version of a normalized `gemini-<major>-...` model ID.
fn gemini_major_version(model: &str) -> Option<u32> {
    let digits = model
        .strip_prefix("gemini-")?
        .split('-')
        .next()
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))?;
    digits.parse().ok()
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
            ImageTokenRegime::OpenAiPatch { multiplier_percent } => {
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
                (patches * u64::from(multiplier_percent)).div_ceil(100) as u32
            }
            ImageTokenRegime::OpenAiTile { base, tile } => {
                if width.min(height) > 768 {
                    let scale = 768.0 / f64::from(width.min(height));
                    (width, height) = scale_dimensions(width, height, scale);
                }
                base + tile * width.div_ceil(512) * height.div_ceil(512)
            }
            ImageTokenRegime::GeminiTiles => gemini_tile_tokens(width, height),
            ImageTokenRegime::GeminiMediaResolution => GEMINI_DEFAULT_MEDIA_RESOLUTION_TOKENS,
            ImageTokenRegime::GeminiUnknown => {
                gemini_tile_tokens(width, height).max(GEMINI_DEFAULT_MEDIA_RESOLUTION_TOKENS)
            }
        }
    }
}

fn gemini_tile_tokens(width: u32, height: u32) -> u32 {
    if width <= 384 && height <= 384 {
        258
    } else {
        width.div_ceil(768) * height.div_ceil(768) * 258
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

        // OpenAI / ChatGPT current patch models (multiplier 1.2): gpt-5.2+,
        // gpt-6*, gpt-5-mini, and unknown models.
        for (kind, model) in [
            (KIND_OPENAI_RESPONSES, "gpt-5.6"),
            (KIND_CHATGPT_RESPONSES, "gpt-6-sol"),
            (KIND_OPENAI_RESPONSES, "gpt-5.2"),
            (KIND_OPENAI_RESPONSES, "gpt-5.4-mini"),
            (KIND_OPENAI_RESPONSES, "gpt-5-mini"),
            (KIND_OPENAI_RESPONSES, "gpt-5-mini-2025-08-07"),
            (KIND_OPENAI_RESPONSES, "unknown-future-model"),
        ] {
            assert_eq!(image_tokens(kind, model, 200, 200), 59, "{model}");
            assert_eq!(image_tokens(kind, model, 1024, 1024), 1_229, "{model}");
            assert_eq!(image_tokens(kind, model, 1920, 1080), 2_448, "{model}");
            assert_eq!(image_tokens(kind, model, 2048, 2048), 3_000, "{model}");
            // Smith's 2048 px edge fit runs first: 2048x256 is 64x8 patches.
            assert_eq!(image_tokens(kind, model, 4096, 512), 615, "{model}");
            assert_eq!(image_tokens(kind, model, 3840, 2160), 2_765, "{model}");
        }

        // Older patch models with their own multipliers. A 200x200 image is
        // 7x7 = 49 patches and 1920x1080 is 60x34 = 2,040 patches.
        for (model, small, hd) in [
            ("gpt-5-nano", 74, 3_060),
            ("gpt-4.1-mini", 80, 3_305),
            ("gpt-4.1-mini-2025-04-14", 80, 3_305),
            ("gpt-4.1-nano", 121, 5_019),
            ("o4-mini", 85, 3_509),
        ] {
            assert_eq!(
                image_tokens(KIND_OPENAI_RESPONSES, model, 200, 200),
                small,
                "{model}"
            );
            assert_eq!(
                image_tokens(KIND_OPENAI_RESPONSES, model, 1920, 1080),
                hd,
                "{model}"
            );
        }

        // OpenAI tile models: base + tile * tiles after the 768 px shortest-side
        // fit. 200x200 is one tile; 1920x1080 becomes 1365x768, six tiles.
        for (model, base, tile) in [
            ("gpt-4o", 85, 170),
            ("chatgpt-4o-latest", 85, 170),
            ("gpt-4.1", 85, 170),
            ("gpt-4o-mini", 2_833, 5_667),
            ("gpt-4o-mini-2024-07-18", 2_833, 5_667),
            ("o1", 75, 150),
            ("o3", 75, 150),
            ("gpt-5", 70, 140),
            ("gpt-5.1", 70, 140),
            ("gpt-5-codex", 70, 140),
        ] {
            assert_eq!(
                image_tokens(KIND_OPENAI_RESPONSES, model, 200, 200),
                base + tile,
                "{model}"
            );
            assert_eq!(
                image_tokens(KIND_OPENAI_RESPONSES, model, 1920, 1080),
                base + 6 * tile,
                "{model}"
            );
        }

        // Gemini 2.x: 258 tokens when <=384px both sides, else
        // ceil(w/768) * ceil(h/768) * 258 after the 3072 px fit.
        for model in ["gemini-2.5-pro", "models/gemini-2.0-flash"] {
            assert_eq!(image_tokens(KIND_GEMINI_INTERACTIONS, model, 200, 200), 258);
            assert_eq!(image_tokens(KIND_GEMINI_INTERACTIONS, model, 384, 384), 258);
            assert_eq!(
                image_tokens(KIND_GEMINI_INTERACTIONS, model, 1920, 1080),
                1_548
            );
            assert_eq!(
                image_tokens(KIND_GEMINI_INTERACTIONS, model, 3840, 2160),
                3_096
            );
        }

        // Gemini 3 and 4 at the default media resolution: at most 1,120 per image.
        for model in [
            "gemini-3-pro-preview",
            "gemini-3.6-flash",
            "google/gemini-4-argon",
        ] {
            for (width, height) in [(200, 200), (1920, 1080), (3840, 2160)] {
                assert_eq!(
                    image_tokens(KIND_GEMINI_INTERACTIONS, model, width, height),
                    1_120,
                    "{model} {width}x{height}"
                );
            }
        }

        // Unrecognized Gemini models take the larger of the two rules.
        for model in ["gemini-flash-latest", "gemini-5-pro"] {
            assert_eq!(
                image_tokens(KIND_GEMINI_INTERACTIONS, model, 200, 200),
                1_120,
                "{model}"
            );
            assert_eq!(
                image_tokens(KIND_GEMINI_INTERACTIONS, model, 1920, 1080),
                1_548,
                "{model}"
            );
            assert_eq!(
                image_tokens(KIND_GEMINI_INTERACTIONS, model, 3840, 2160),
                3_096,
                "{model}"
            );
        }

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
            SmithRequestSizer::for_provider_and_model(KIND_OPENAI_RESPONSES, "gpt-4o-mini")
                .revision(),
            SmithRequestSizer::for_provider_and_model(KIND_OPENAI_RESPONSES, "o3").revision(),
            SmithRequestSizer::for_provider_and_model(KIND_OPENAI_RESPONSES, "gpt-4.1-mini")
                .revision(),
            SmithRequestSizer::for_provider_and_model(KIND_GEMINI_INTERACTIONS, "gemini-2.5-pro")
                .revision(),
            SmithRequestSizer::for_provider_and_model(KIND_GEMINI_INTERACTIONS, "gemini-4-argon")
                .revision(),
            SmithRequestSizer::for_provider_and_model(KIND_GEMINI_INTERACTIONS, "gemini-next")
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
