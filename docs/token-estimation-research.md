# Context-size estimation research

Findings from 2026-10-23 feeding a future estimation improvement. Not
shipped behavior. Two questions: how the runtime estimates context usage
today, and what providers actually bill — especially for images.

## How estimation works today

Smith never installs a custom `RequestSizer`, so the agent-runtime builder
default applies (`agent-runtime/src/runtime/builder.rs:862`):
`CharRatioSizer::default()` (`agent-runtime-context/src/sizing.rs`).

- Text: `ceil(chars / 4)` plus framing (4/message, 8/tool schema, 6/tool
  call or result).
- Reasoning signatures: `ceil(chars / 6)`, measured on Gemini.
- **Images: a flat 85 tokens each** (`DEFAULT_IMAGE_TOKENS`), a constant
  "modeled after common low-detail vision pricing".

Every plan therefore carries `EstimationConfidence::Estimated`, and the
planner refuses estimated plans with headroom under `max_estimated_slack`.

## The image estimate is wrong in the unsafe direction

The 85-token constant mirrors OpenAI's legacy `detail: "low"` flat price.
Smith does not set a `detail` hint, and it prepares images for providers'
**high-resolution** limits (`ImageLimits::for_provider`,
`crates/smith-tools/src/image_preparation.rs:72`: OpenAI 2048 px / 2,500
tiles, Anthropic 2576 px, Gemini 3072 px). Providers bill the fitted image
by patches or tiles, not a flat rate.

The refit spec's risk note
(`docs/spec/changes/archive/2026-10-17-update-image-refit-per-request-2026-10-16/design.md:45`)
assumed image accounting errs high ("counted high … conservative"). For the
planner it errs **low**, by 18×–56× for ordinary screenshots, because the
sizer ignores the URL and dimensions entirely.

### Verified error magnitudes (85-token estimate vs provider billing)

| Source image | Sent as | Claude 4.7+ | OpenAI gpt-5.6 high | Gemini 2.5 | Flat estimate |
|---|---|---|---|---|---|
| 200×200 | unchanged | 64 | 59 | 258 | 85 |
| 1920×1080 | unchanged | 2,691 | 2,448 | 1,548 | 85 |
| 3840×2160 | fitted | 4,784 | 2,765 | 3,096 | 85 |

A session with five 1080p screenshots is undercounted by ~7k–13k tokens.
Because `max_estimated_slack` and compaction thresholds are computed from
the undercount, image-heavy turns can pass planning and then fail at the
provider instead of compacting first.

## Verified provider formulas (checked against live docs, 2026-10-23)

All formulas apply **after** the provider's own resize rules; estimates must
replicate Smith's `ImageLimits` fit first, since that is what is billed.

### Anthropic (docs.claude.com, vision)

`tokens = ⌈w/28⌉ × ⌈h/28⌉` (28×28-pixel patches), after downsizing to the
model tier: standard 1,568 px long edge / 1,568 visual tokens; high-res
(Claude 4.7+) 2,576 px / 4,784 tokens. All six worked examples in the docs
table reproduce exactly (200×200→64, 1000×1000→1,296, 1920×1080→2,691,
3840×2160→4,784 …).

### OpenAI (developers.openai.com/api/docs/guides/images-vision)

Two regimes. **Patch models**: fit to the detail level's pixel limit (high:
2048 px), `patches = ⌈w/32⌉ × ⌈h/32⌉`, shrink to the patch budget (high:
2,500) via `shrink = sqrt(32² · budget / (w·h))`, then
`tokens = ceil(patches × multiplier)`. Worked examples (gpt-6-astra, high)
verify: 1024×1024→1,229, 2048×2048→3,000, 4096×512→2,458 (no 2048 px edge for
gpt-6-astra; Smith's own fit makes that image 2048×256, 615 tokens).

| Patch model | Multiplier |
|---|---|
| gpt-6-astra, gpt-5.6-*, gpt-5.5, gpt-5.4 (+mini/nano), gpt-5.2, gpt-5-mini | 1.2 |
| gpt-5-nano | 1.5 |
| gpt-4.1-mini | 1.62 |
| gpt-4.1-nano | 2.46 |
| o4-mini | 1.72 |

**Tile models**: scale into 2048², shortest side to ≤768,
`tiles = ⌈w/512⌉ × ⌈h/512⌉`, `base + tiles × tile_cost`. `detail: "low"`
costs only the base tokens.

| Tile model | Base | Per tile |
|---|---|---|
| gpt-5, gpt-5.1 | 70 | 140 |
| gpt-4o, gpt-4.1 | 85 | 170 |
| gpt-4o-mini | 2,833 | 5,667 |
| o1, o1-pro, o3 | 75 | 150 |

Rechecked 2026-10-24. The first pass (2026-10-23) put gpt-5-mini/nano,
gpt-4.1-mini/nano and o4-mini on tiles, priced gpt-4o-mini like gpt-4o, and o1/o3
at 85 + 170; all three were wrong. The page gives no patch budget for the
older patch models except gpt-4.1-mini (6,144), and says unlisted models "can
use different limits".

### Gemini (ai.google.dev, token counting and media resolution)

Gemini 1.x/2.x: 258 tokens if both dimensions ≤ 384 px; otherwise 768×768
tiles at 258 tokens each (`⌈w/768⌉ × ⌈h/768⌉ × 258`, documented as a rough
formula).

Gemini 3 and 4 allot a per-image maximum set by `media_resolution`: low 280,
medium 560, high 1,120, ultra_high 2,240, and 1,120 for the default
`unspecified` (ai.google.dev/gemini-api/docs/media-resolution). Smith never
sets it, so 1,120 is an upper bound per image whatever its size.

### xAI and Z.AI/GLM

No published patch/tile formula found. Use the OpenAI patch formula with a
2,048 px edge as a conservative fallback (`ImageProvider::Xai` and `Other`
limits already match).

## Implementation tracks

**A. Smith-side image-aware sizer (biggest win, no upstream change).**
Implement `RequestSizer` in `smith-runtime` and install it via the runtime
builder's `.request_sizer(...)`. Every input already exists in this repo:
provider kind at `crates/smith-runtime/src/factory.rs:1046`,
`ImageLimits` per provider, bounded header decoding for data URLs
(`crates/smith-tools/src/image_header.rs:21`, PNG/JPEG/GIF/WebP without a
full decode). Size the post-fit dimensions, mirror the planner's image
window (projection already runs before `BudgetReport::compute`), and fall
back to a conservative ~2,000–3,000 tokens for unreadable or remote images
instead of 85. A changed sizer automatically changes plan fingerprints via
its `ComponentRef` revision.

**B. Upstream `CharRatioSizer` upgrade.** Make the upstream sizer image
aware (dimension-based formulas above) and split a `json_chars_per_token`
(~3) for tool schemas/calls/results; bump `CHAR_RATIO_SIZER_REVISION` so
replayed plans invalidate cleanly. Benefits every agent-runtime consumer;
requires a ForgeAILab/agent-runtime change and a Smith dependency bump.

**C. Self-calibration.** Providers report actual `input_tokens` each turn
and `smith-client/src/status/accounting.rs` already separates reported from
estimated counters. Comparing the plan's estimated input against the
reported input per turn yields a rolling correction factor (or dynamic
slack) that absorbs text-ratio drift too — CJK and minified JSON tokenize
near 1–2 and ~3 chars per token respectively, versus the current flat 4.

## Open questions

- Claude tier per model (standard vs high-res) needs a model-version map;
  Smith's catalog knows model IDs but not tier.
- OpenAI regime selection is per model id (patch vs tile models), and
  Smith never sets `detail`, so `auto`/`original` behavior is assumed.
- Remote (non-`data:`) image URLs cannot be header-read without a fetch;
  they need the conservative fallback.
- The planner sizes the pre-fit image while the provider re-fits it; the
  sizer must reproduce the fit deterministically (same limits), or sizing
  must move to the fitted URL.
