---
created_at: 2026-10-16T02:27:02Z
updated_at: 2026-10-16T04:53:48Z
completed_at: 2026-10-16T03:45:12Z
---

## 1. Preparation
- [x] 1.1 Enable the `image` crate's `jpeg`, `gif` and `webp` features and
      check `cargo deny`.
- [x] 1.2 Add the image preparation module in `crates/smith-tools`. It has
      the provider limits table (D2) and the stages of D4: guards, signature
      detection, fit, convert, size shrink, and passthrough of unchanged
      bytes. Its result describes the original and the prepared image.
- [x] 1.3 The factory selects the limits from the main model's provider and
      passes them to `read`; child runtimes use their own provider.

## 2. Inputs
- [x] 2.1 `read` returns the prepared image and its one-line description;
      other binary files keep the existing refusal.
- [x] 2.2 TUI `@FILE` of an image attaches the prepared image, shown like a
      clipboard attachment.
- [x] 2.3 `smith -p --image <PATH>` (repeatable) adds prepared images to the
      prompt; `--image` with a `cli/*` profile fails before the run starts.

## 3. Tests and docs
- [x] 3.1 Preparation tests, using generated images:
      - passthrough of a small PNG, byte for byte;
      - downscale of a large JPEG to each provider's longest side;
      - the tile budget;
      - WebP converted to PNG for xAI;
      - an animated GIF reduced to its first frame;
      - a PNG over the size limit becoming JPEG;
      - a transparent PNG staying PNG and shrinking;
      - the 64 MiB and 100-megapixel guards;
      - a corrupt image;
      - a fake `.png`.
- [x] 3.2 Input tests:
      - `read` on an image and on a text file;
      - `@FILE` of an image;
      - `--image`, including a missing file and a `cli/*` refusal;
      - a text-only model sees the placeholder.
- [x] 3.3 Docs: `docs/headless-protocol.md` (`--image`), `DESIGN.md`, CLI help.
- [x] 3.4 Gate. Live: the `visual` profile labels a real photo through
      `smith -p --image` and through `read`, and a large photo is downscaled.
      (2026-10-16: full locked gate, 2,918 tests passed. On the owner's config,
      `chatgpt/gpt-6-astra` named the colors of a 6016×6016 JPEG (sent as
      1600×1600, 194 KiB, the tile budget) and an 800×800 PNG (sent
      unchanged), through `--image` and through `read` with neutral file
      names. `--image` with `cli/claude-code/opus` exits 1 before a session.)
