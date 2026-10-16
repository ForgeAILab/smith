## Context

The runtime projects image parts per request. A text-only profile gets
placeholders, and an image-capable profile gets images inside a stepped
window. MCP results, generated images and clipboard attachments already use
that path. File input is the missing source: `read` and `@FILE` go through
`support::read_bounded_*`, which refuses content that `looks_binary`.

Codex prepares every local image once, when it enters history
(`core/src/image_preparation.rs`):

- Detection by magic bytes.
- A fit to a longest side of 2048 px and a budget of 2,500 tiles of 32 px.
- The original bytes are kept when nothing changes. GIF becomes PNG. JPEG is
  re-encoded at quality 85.
- Errors become a text placeholder.
- A 32-entry LRU cache keyed by content hash.

## Goals / Non-Goals

- Goals:
  - An image-capable model can read an image file.
  - A user can attach one from the TUI or from `smith -p`.
  - Images are fitted to the active provider rather than refused.
- Non-Goals:
  - HEIC, BMP, TIFF, AVIF and PDF. Codex does not decode them either; they
    stay binary refusals.
  - Re-preparing images already in history when the provider changes.
  - Routing MCP image results through the new step. The runtime already
    bounds those.
  - Image input on `cli/*` harness turns.
  - Changing `view_image` availability.

## Decisions

- **D1 Signature detection** with `image::guess_format`. A file named `.png`
  that is not a PNG stays a binary refusal.
- **D2 Provider limits, one table.** They are chosen from the main model's
  provider when the runtime is built, and recomposition (`/model`) rebuilds
  them. Values come from each provider's current documentation, checked
  2026-10-15:

  | Provider | Formats sent as-is | Longest side and budget | Encoded size |
  |---|---|---|---|
  | OpenAI, ChatGPT | PNG, JPEG, WebP, still GIF | 2048 px, 2,500 tiles of 32 px (Codex's high-detail budget) | no per-image cap; requests ≤ 512 MB |
  | Anthropic | PNG, JPEG, GIF, WebP | 2576 px (the high-resolution models' limit; the API downscales beyond it) | 10 MB base64 per image (5 MB on Bedrock and Vertex) |
  | Gemini | PNG, JPEG, WebP | 3072 px | 20 MB inline per request |
  | xAI | PNG, JPEG only | 2048 px | 20 MiB per image |
  | OpenAI-compatible and unknown | PNG, JPEG | 2048 px | 5 MB base64 |

  The table is data, so a limit changes in one place. Gemini's 3072 px is
  my choice: the pages checked give request-size limits, not a pixel edge.
- **D3 Prepare once, at ingestion.** History holds the prepared image. A
  later switch to a stricter provider is not re-prepared (non-goal); the
  provider's own error surfaces if it rejects.
- **D4 Fit, then convert, then shrink.**
  1. Decode, refusing first a file over 64 MiB or an image over 100
     megapixels, so a malicious file cannot exhaust memory.
  2. Scale to the longest side, then to the pixel budget, using a triangle
     filter as Codex does.
  3. If the format is not accepted, encode PNG. An animated GIF becomes its
     first frame.
  4. If the encoded size is still over the limit, a PNG without transparency
     is re-encoded as JPEG at quality 85. Otherwise the image is scaled down
     in 25% steps until it fits.
  5. Unchanged images keep their original bytes.
- **D5 `read` keeps its authority.** The same workspace confinement and
  approval apply; only the result shape changes. `read` always returns the
  image part, and the runtime's projection decides what a text-only model
  receives.
- **D6 `--image` is caller authority.** Whoever runs `smith -p` names the
  file, the same as stdin with `-p -`.
- **D7 `cli/*` refusal.** The harness bridge carries only text prompts today.
  Dropping an image silently would repeat this bug.
- **D8 No cache at first.** Preparation runs once per ingestion, and history
  then holds the result. Codex's LRU mainly serves re-preparation on resume,
  which Smith does not do.

## Risks / Trade-offs

- **Binary size.** The JPEG, GIF and WebP decoders grow Smith's binary. Codex
  ships the same set.
- **Lossy JPEG.** Re-encoding at quality 85 can blur fine text in a photo of
  a screen. PNG is kept whenever it fits.
- **Stale limits.** Provider limits drift. The table records when it was
  checked.

## Migration Plan

None. A model that previously got a "binary" refusal now gets the image.
