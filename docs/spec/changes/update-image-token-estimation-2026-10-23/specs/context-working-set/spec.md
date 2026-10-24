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
  budget and `⌈patches × 1.2⌉` tokens for `gpt-5.2+`, `gpt-6*`, and unknown
  models, and 512×512 tiles after a 768 px shortest-side fit for legacy tile
  models (`85 + 170 × tiles` for `gpt-4o`, `gpt-4.1`, and `o1`/`o3`/`o4`;
  `70 + 140 × tiles` for `gpt-5` and `gpt-5.1`).
- Gemini models SHALL use 258 tokens when both fitted dimensions are at most
  384 px, and `⌈w/768⌉ × ⌈h/768⌉ × 258` tokens otherwise.
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
  OpenAI `gpt-5.6`, and 1,548 tokens for Gemini

#### Scenario: 4K image is sized using post-fit provider dimensions

- **GIVEN** a 3840×2160 PNG data URL in session history that has not yet been
  re-fitted for the active provider
- **WHEN** the request sizer estimates its token contribution
- **THEN** it applies the active provider's `ImageLimits` fit first and
  estimates 4,784 tokens for Claude 4.7+, 2,765 tokens for OpenAI `gpt-5.6`, and
  3,096 tokens for Gemini

#### Scenario: Unreadable or remote image uses conservative fallback

- **GIVEN** a remote `https://` image URL or a `data:` image URL with a corrupt
  header
- **WHEN** the request sizer estimates its token contribution
- **THEN** it estimates 2,500 tokens without failing planning
