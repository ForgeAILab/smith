## ADDED Requirements

### Requirement: Provider- and model-aware image token estimation

Smith SHALL install a `RequestSizer` on root and child runtimes that estimates
inline `ContentPart::Image` tokens from the serving provider's post-fit
dimensions and model vision billing regime rather than a flat low-detail
constant. For `data:` image URLs with readable headers, Smith SHALL decode pixel
dimensions from the bounded header, apply the serving provider's `ImageLimits`
dimension fit, and calculate tokens according to the active provider and model
tier:
- Anthropic models SHALL use 28×28-pixel patches (`⌈w/28⌉ × ⌈h/28⌉`) after
  downsizing to the model tier's edge and token budget (2,576 px / 4,784 tokens
  for Claude 4.7+ and unknown Anthropic models; 1,568 px / 1,568 tokens for
  earlier Claude models).
- OpenAI and ChatGPT models SHALL use 32×32-pixel patches with a 2,500-patch
  budget and `⌈patches × multiplier⌉` tokens for patch models, with a
  multiplier of 1.2 for `gpt-5.2+`, `gpt-6*`, `gpt-5-mini`, and unknown models,
  1.5 for `gpt-5-nano`, 1.62 for `gpt-4.1-mini`, 2.46 for `gpt-4.1-nano`, and
  1.72 for `o4-mini`. Tile models SHALL use 512×512 tiles after a 768 px
  shortest-side fit and `base + tile × tiles` tokens: 85 + 170 for `gpt-4o`
  and `gpt-4.1`, 2,833 + 5,667 for `gpt-4o-mini`, 75 + 150 for `o1` and `o3`,
  and 70 + 140 for `gpt-5` and `gpt-5.1`.
- Gemini 1.x and 2.x models SHALL use 258 tokens when both fitted dimensions
  are at most 384 px, and `⌈w/768⌉ × ⌈h/768⌉ × 258` tokens otherwise. Gemini 3
  and 4 models SHALL use 1,120 tokens per image, the maximum at the default
  media resolution. Other Gemini models SHALL use the larger of the two.
- xAI and compatible providers SHALL use the OpenAI 32×32 patch formula on the
  fitted 2,048 px dimensions.

When an image URL is not a `data:` URL or its header dimensions cannot be
decoded, the sizer SHALL estimate 2,500 tokens. Images replaced by text
placeholders before sizing (by the text-only projection or the bounded image
window) SHALL be sized as their placeholder text.

#### Scenario: High-resolution screenshot sized for Claude 4.7+, GPT-5.6, and Gemini

- **GIVEN** a 1920×1080 PNG data URL in a message or tool result
- **WHEN** the request sizer estimates its image token contribution
- **THEN** it estimates 2,691 tokens for Claude 4.7+, 2,448 tokens for
  OpenAI `gpt-5.6`, 1,548 tokens for Gemini 2.5, and 1,120 tokens for
  Gemini 3 and 4

#### Scenario: 4K image is sized using post-fit provider dimensions

- **GIVEN** a 3840×2160 PNG data URL in session history that has not yet been
  re-fitted for the active provider
- **WHEN** the request sizer estimates its token contribution
- **THEN** it applies the active provider's `ImageLimits` fit first and
  estimates 4,784 tokens for Claude 4.7+, 2,765 tokens for OpenAI `gpt-5.6`, and
  3,096 tokens for Gemini 2.5

#### Scenario: Unreadable or remote image uses conservative fallback

- **GIVEN** a remote `https://` image URL or a `data:` image URL with a corrupt
  header
- **WHEN** the request sizer estimates its token contribution
- **THEN** it estimates 2,500 tokens without failing planning

#### Scenario: Older OpenAI models use their own image rates

- **GIVEN** a 1920×1080 PNG data URL
- **WHEN** the request sizer estimates it for `gpt-4.1-mini`, `gpt-4o-mini`,
  and `o3`
- **THEN** it estimates 3,305 tokens (2,040 patches × 1.62), 36,835 tokens
  (2,833 + 6 tiles × 5,667), and 975 tokens (75 + 6 tiles × 150)

#### Scenario: Child policy fingerprints ignore the request sizer

- **GIVEN** a durable child interrupted under a Smith build whose image sizer
  revision differs from the current one
- **WHEN** the parent resumes and the child's policy fingerprint is compared
- **THEN** the sizer revision does not contribute to the fingerprint, so the
  child is not reported as having a changed policy for that reason alone
