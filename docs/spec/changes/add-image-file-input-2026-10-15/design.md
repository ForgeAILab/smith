## Context

The runtime projects image parts per request. A text-only profile gets
placeholders, and an image-capable profile gets images inside a stepped
window. MCP results, generated images and clipboard attachments already use
that path. File input is the missing source:

- `read` and the `@FILE` attachment go through `support::read_bounded_*`,
  which refuses content that `looks_binary`.

## Goals / Non-Goals

- Goals:
  - An image-capable model can read an image file.
  - A user can attach one from the TUI or from `smith -p`.
  - Bounds and approval are the same as for any other read.
- Non-Goals:
  - Resizing or re-encoding. Only the PNG codec is compiled in, so files pass
    through unchanged.
  - PDF or other document types.
  - Image input on `cli/*` harness turns.
  - Changing `view_image` availability.

## Decisions

- **D1 Signature detection.** PNG, JPEG, GIF and WebP are recognized by their
  magic bytes. This reuses `image_header`'s parsing, which also reports pixel
  size. A file named `.png` that is not a PNG stays a binary refusal.
- **D2 Bound: 3.5 MiB per image** (owner may adjust).
  - Base64 encoding grows 3.5 MiB of file bytes to about 4.9 MB, under the
    5 MB per-image limit that Anthropic, the strictest provider Smith
    targets, applies.
  - Larger files are refused with the size and the bound.
  - Smith does not downscale.
- **D3 Always return the image part.** `read` does not check the model. The
  runtime's projection already decides per request what a text-only model
  receives, and `view_image` resolves the placeholder id. A model switch
  therefore never makes earlier history inconsistent.
- **D4 `read` keeps its authority.** The same workspace confinement and
  approval apply. Only the result shape changes.
- **D5 `--image` is caller authority.** Whoever runs `smith -p` names the file,
  the same as stdin with `-p -`. It is read directly, with no approval, and
  may be outside the project.
- **D6 `cli/*` refusal.** The harness bridge carries only text prompts today.
  Silently dropping an image would repeat this bug, so the run fails before
  starting.

## Risks / Trade-offs

- **Cost.** Images cost input tokens on every request inside the image window.
  The existing `ImageWindow` setting bounds that.
- **Provider size limits.** A provider may reject a file within 3.5 MiB on
  pixel dimensions. That surfaces as the provider's error on that request.

## Migration Plan

None. A model that previously got a "binary" refusal now gets the image.
