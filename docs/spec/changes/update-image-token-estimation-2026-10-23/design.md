## Context

`agent-runtime` plans requests in two steps relevant to images:
1. `project_fragments` applies the model's input modalities and the configured
   `image_window` policy, replacing omitted images with `[image <id>: ...]` text
   placeholders.
2. `BudgetReport::compute` calls the configured `RequestSizer` on the projected
   fragments.

Only images that remain inline in the outgoing request are still
`ContentPart::Image` when the sizer runs. Later, when the request is sent,
`ImageFittingProvider` fits every `data:` image URL to
`ImageLimits::for_provider(provider)`.

## Decisions

- **D1 Header-only dimension inspection + deterministic fit.**
  The sizer uses `smith_tools::image_header::describe_image_url` (bounded to
  4 KiB for PNG/GIF/WebP and 64 KiB for JPEG) and `fitted_dimensions` from
  `smith_tools::image_preparation` against `ImageLimits::for_provider`. This
  reproduces the dimensions that `ImageFittingProvider` sends without decoding
  pixel payloads during planning.
- **D2 Model-aware provider regimes with conservative defaults.**
  - `ImageProvider::Anthropic`: `ClaudeHighRes` (2,576 px edge, 4,784 max
    tokens) for Claude 4.7+ and unknown models; `ClaudeStandard` (1,568 px edge,
    1,568 max tokens) for known earlier Claude models (`claude-2*`,
    `claude-instant*`, `claude-3*`, `claude-*-4-0*`, `claude-*-4-1*`,
    `claude-*-4-5*`, `claude-*-4-6*`, and unversioned `claude-{sonnet,opus,haiku}-4`).
  - `ImageProvider::OpenAi`, from OpenAI's image-input cost tables (rechecked
    2026-10-24, see Sources). Patch models count 32×32 patches after Smith's
    2,048 px / 2,500-patch fit and bill `ceil(patches × multiplier)`:
    - ×1.2 for `gpt-5.2+` (including the `gpt-5.4`/`gpt-5.6` mini/nano
      variants), `gpt-6*`, `gpt-5-mini`, and unknown models;
    - ×1.5 for `gpt-5-nano`; ×1.62 for `gpt-4.1-mini`; ×2.46 for
      `gpt-4.1-nano`; ×1.72 for `o4-mini`.
    Tile models scale the shortest side to ≤768 px, count 512×512 tiles, and
    bill `base + tile × tiles`:
    - `gpt-4o*`, `chatgpt-4o*`, `gpt-4.1`: 85 + 170;
    - `gpt-4o-mini*`: 2,833 + 5,667;
    - `o1*`, `o3*` (including `o1-pro`): 75 + 150;
    - `gpt-5`, `gpt-5-chat*`, `gpt-5-pro*`, `gpt-5-codex*`, `gpt-5.1*`: 70 + 140.
    More specific prefixes win (`gpt-4o-mini` before `gpt-4o`, `gpt-4.1-mini`
    before `gpt-4.1`). OpenAI publishes no patch budget for the older patch
    models (`gpt-4.1-mini`'s is 6,144); Smith applies only its own 2,500-patch
    fit, so a smaller provider budget can only make the estimate high.
  - `ImageProvider::Gemini`, by major model version:
    - Gemini 1.x/2.x: 258 tokens if `w <= 384 && h <= 384`, else
      `⌈w/768⌉ × ⌈h/768⌉ × 258`;
    - Gemini 3 and 4: a flat 1,120 tokens per image, the documented maximum at
      the default (`unspecified`) `media_resolution`, which Smith never sets;
    - any other Gemini model ID: the larger of the two.
  - `ImageProvider::Xai` and `ImageProvider::Other`: ×1.2 patches after the
    2,048 px `ImageLimits` fit.
- **D3 Fallback for unknown dimensions.**
  Remote (non-`data:`) image URLs or `data:` URLs whose header dimensions cannot
  be decoded are estimated at `2,500` tokens (`UNKNOWN_IMAGE_TOKENS`).
- **D4 Root and child parity, without fingerprinting the sizer.**
  Both `factory/construction.rs` (root `RuntimeBuilder`) and `delegation.rs`
  (`SmithChildFactory::child_builder`) install `SmithRequestSizer` for their
  resolved `(provider_kind, model)` pair. The child policy fingerprint does
  **not** include the sizer revision (owner decision, 2026-10-24): the
  fingerprint decides whether an interrupted child may resume, and the sizer
  revision moves whenever an estimate formula changes. Including it would make
  every child interrupted before an upgrade unresumable for a change that only
  affects planning estimates. The fingerprint already records the provider kind
  and model, which select the regime. An architecture test keeps the sizer out
  of `policy_fingerprint`.

## Sources

Accessed 2026-10-24.

- OpenAI, "Images and vision", image input costs (patch multipliers, tile
  base/tile tokens, detail limits):
  https://developers.openai.com/api/docs/guides/images-vision (redirect from
  https://platform.openai.com/docs/guides/images-vision)
- Google, "Media resolution" (Gemini 3 and 4 per-image maximums by
  `media_resolution`): https://ai.google.dev/gemini-api/docs/media-resolution
- Google, "Token counting" (258-token images and 768×768 tiles):
  https://ai.google.dev/gemini-api/docs/tokens

The 2026-10-23 review's claims all held: the gpt-5-mini/nano, gpt-4.1-mini/nano
and o4-mini patch billing, gpt-4o-mini's 2,833 + 5,667 tile rate, and o1/o3 at
75 + 150. Two details it did not state: `gpt-5-mini` uses the current ×1.2
multiplier (only the others are higher), and the `gpt-5` base model does keep
the 70 + 140 tile rate.
