---
created_at: 2026-10-23T09:44:00Z
updated_at: 2026-10-24T12:38:26Z
completed_at: 2026-10-24T12:38:26Z
---

## 1. Implementation

- [x] 1.1 Expose deterministic post-limit dimension fitting (`fitted_dimensions`
  and `image_provider_for_kind`) in `smith_tools::image_preparation` /
  `smith_runtime::factory` so request sizing and request-time image fitting
  share one dimension calculation.
- [x] 1.2 Implement `SmithRequestSizer` in `crates/smith-runtime/src/image_sizing.rs`
  with model-aware Anthropic/OpenAI tiers, Gemini tile sizing, xAI/compatible
  patch fallback, and the 2,500-token unknown-header fallback.
- [x] 1.3 Wire `SmithRequestSizer` into root runtime construction
  (`crates/smith-runtime/src/factory/construction.rs`) and child runtime
  construction (`crates/smith-runtime/src/delegation.rs`).
- [x] 1.4 Size model-fallback routes with `SmithRequestSizer` too
  (`crates/smith-runtime/src/factory/fallback.rs`) once
  `add-profile-model-fallback-2026-10-23` lands; its branch leaves them on the
  default sizer so the two changes merge independently. Covered by
  `composition::fallback::a_fallback_route_sizes_images_at_its_own_provider_rate`.

- [x] 1.5 Correct the OpenAI tiers (per-model patch multipliers, `gpt-4o-mini`
  and `o1`/`o3` tile rates) and the Gemini 3/4 flat per-image maximum from the
  providers' published tables (design.md Sources).
- [x] 1.6 Leave the sizer revision out of the child policy fingerprint
  (`crates/smith-runtime/src/delegation.rs`) and guard it with an architecture
  test.

## 2. Verification

- [x] 2.1 Add unit tests in `crates/smith-runtime/src/image_sizing.rs` covering
  all worked examples from provider documentation (Anthropic standard and
  high-res, OpenAI patch and legacy tile regimes, Gemini small and tiled images,
  xAI/compatible fallback, remote/corrupt fallback, and nested tool-result
  images).
- [x] 2.2 Add runtime planning integration coverage verifying that inline images
  are counted at their provider-fitted patch/tile cost while windowed-out
  placeholder images are counted as short text.
- [x] 2.3 Run `cargo test`, `cargo clippy`, `cargo fmt --check`, and strict spec
  change validation.
