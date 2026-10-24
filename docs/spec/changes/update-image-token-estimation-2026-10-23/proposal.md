---
created_at: 2026-10-23T09:44:00Z
updated_at: 2026-10-23T10:16:55Z
---

## Why

Smith does not install a custom `RequestSizer`, so `RuntimeBuilder` falls back
to `CharRatioSizer::default()`, which estimates every image at a flat 85 tokens
(`DEFAULT_IMAGE_TOKENS`, modeled after OpenAI's legacy `detail: "low"` flat
price). However, Smith never sets a low-detail hint and prepares images for each
provider's high-resolution limits (`ImageLimits::for_provider`). Providers bill
fitted images by patches or tiles, so an ordinary 1080p or 4K screenshot costs
1,548–4,784 tokens—an 18×–56× undercount.

Because `max_estimated_slack` and compaction watermarks rely on the sizer's
estimate, image-heavy turns can appear well within the input budget during
planning and then fail at the provider instead of compacting first.

## What Changes

- Implement a provider- and model-aware `RequestSizer` in `smith-runtime` and
  install it via `RuntimeBuilder::request_sizer(...)` on both root and child
  runtimes.
- Delegate text, reasoning-signature, message-framing, tool-schema, and
  tool-call/result sizing to `CharRatioSizer::default()`.
- Size `ContentPart::Image` parts (in messages and nested tool results) by:
  1. Decoding pixel dimensions from the bounded data-URL header
     (`smith_tools::image_header::describe_image_url`) without decoding full
     image payloads.
  2. Applying the serving provider's `ImageLimits::for_provider` dimension fit
     so the planner estimates the post-fit dimensions that `ImageFittingProvider`
     sends on the wire.
  3. Applying the provider and model tier's documented patch/tile formula:
     - **Anthropic**: `⌈w/28⌉ × ⌈h/28⌉` patches after downsizing to the model
       tier (Claude 4.7+ and unknown models: 2,576 px long edge / 4,784 token
       budget; earlier Claude models: 1,568 px long edge / 1,568 token budget).
     - **OpenAI / ChatGPT**:
       - Patch models (`gpt-5.2+`, `gpt-6*`, and unknown models): 32×32 patches
         with a 2,500-patch budget shrink and `ceil(patches × 1.2)` tokens.
       - Legacy tile models: shortest side scaled to ≤768 px, `⌈w/512⌉ × ⌈h/512⌉`
         tiles, `85 + 170 × tiles` for `gpt-4o` / `gpt-4.1` / `o1` / `o3` / `o4`
         and `70 + 140 × tiles` for `gpt-5` / `gpt-5.1`.
     - **Gemini**: 258 tokens when both dimensions are ≤384 px; otherwise
       `⌈w/768⌉ × ⌈h/768⌉ × 258`.
     - **xAI / Other**: OpenAI patch formula on the fitted 2,048 px image.
  4. Falling back to a conservative 2,500 tokens when an image URL is remote
     (non-`data:`) or its header dimensions cannot be decoded.
- Include the sizer's provider/model regime in its `ComponentRef` revision so
  plan fingerprints reflect the active image-sizing rules.

## Approval

The user approved Track A implementation with model-aware tiers (defaulting
unknown models to the conservative higher tier) and a 2,500-token fallback for
remote or unreadable image headers.

## Impact

- Affected specs: `context-working-set`.
- Affected code: `crates/smith-tools/src/image_preparation.rs`,
  `crates/smith-runtime/src/image_sizing.rs`,
  `crates/smith-runtime/src/factory.rs`,
  `crates/smith-runtime/src/factory/construction.rs`,
  `crates/smith-runtime/src/delegation.rs`.
