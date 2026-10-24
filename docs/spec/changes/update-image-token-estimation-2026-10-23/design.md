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
  - `ImageProvider::OpenAi`: `OpenAiPatch` (32×32 patches, 2,500-patch budget,
    `ceil(patches × 1.2)`) for `gpt-5.2+`, `gpt-6*`, and unknown models;
    `OpenAiTile4o` (`85 + 170 × tiles`) for `gpt-4o*`, `gpt-4.1*`, `o1*`, `o3*`,
    `o4*`; `OpenAiTile5_1` (`70 + 140 × tiles`) for `gpt-5`, `gpt-5-mini`,
    `gpt-5-nano`, and `gpt-5.1*`.
  - `ImageProvider::Gemini`: 258 tokens if `w <= 384 && h <= 384`, else
    `⌈w/768⌉ × ⌈h/768⌉ × 258`.
  - `ImageProvider::Xai` and `ImageProvider::Other`: `OpenAiPatch` after the
    2,048 px `ImageLimits` fit.
- **D3 Fallback for unknown dimensions.**
  Remote (non-`data:`) image URLs or `data:` URLs whose header dimensions cannot
  be decoded are estimated at `2,500` tokens (`UNKNOWN_IMAGE_TOKENS`).
- **D4 Root and child parity.**
  Both `factory/construction.rs` (root `RuntimeBuilder`) and `delegation.rs`
  (`SmithChildFactory::child_builder`) install `SmithRequestSizer` for their
  resolved `(provider_kind, model)` pair, and child policy fingerprints include
  the sizer revision.
