---
created_at: 2026-10-16T02:27:02Z
updated_at: 2026-10-16T03:05:00Z
---

## Why

A model that accepts images cannot see an image file:

- `read` refuses PNG, JPEG, GIF and WebP as binary.
- The `vision` module, and so `view_image`, mounts only for text-only models.
- `smith -p` has no way to attach an image.
- A TUI `@FILE` attachment of an image fails the same way `read` does.

The owner hit this on 2026-10-15 labeling pictures with the `visual` profile
(`chatgpt/gpt-6-astra`) through `smith -p`. Astra answered without seeing
the picture. Only a pasted clipboard image or an MCP screenshot reaches the
model today.

The owner asked that the limits be the provider's, and that images be
compressed or converted to fit rather than refused (2026-10-15). Codex's
image pipeline (`codex-rs/utils/image`, `core/src/image_preparation.rs`) is
the reference.

## What Changes

- **One image preparation step** for every image file a user or model brings
  in. It has the stages below; when an image already fits, it is sent byte for
  byte unchanged.
  - **Detect:** PNG, JPEG, GIF or WebP, recognized from the file's bytes.
  - **Fit:** downscale to the active provider's limits on longest side, pixel
    budget and encoded size.
  - **Convert:** a format the provider does not accept becomes PNG, and an
    animated GIF becomes its first frame.
  - **Re-encode:** only when needed. JPEG uses quality 85; if PNG is still too
    large, the image becomes JPEG.
- **Per-provider limits** live in one table: OpenAI and ChatGPT, Anthropic,
  Gemini, xAI, plus a conservative default for OpenAI-compatible and unknown
  providers.
- **`read` returns images.** An image file comes back as the prepared image
  plus one line naming the path, the original size, and the size sent, for
  example `photo.jpg · 4032×3024 → 2048×1536 JPEG · 412 KiB`. Other binary
  files are still refused.
- **TUI `@FILE` attaches an image as an image**, prepared the same way, shown
  like a pasted clipboard image.
- **`smith -p --image <PATH>`** (repeatable) attaches prepared images to the
  prompt. The caller names the file, so it is read without approval and is
  not confined to the project.
- **Text-only models are unchanged in kind.** They still receive the runtime's
  `[image <id>: …]` placeholder, which `view_image` can inspect.
- `--image` with a `cli/*` profile fails with an explicit message instead of
  dropping the image.
- **Dependency:** the `image` crate gains its `jpeg`, `gif` and `webp`
  decoders, the same set Codex enables.

## Impact

- Affected specs: `tool-execution` (ADDED), `client-surfaces` (ADDED)
- Affected code:
  - `Cargo.toml` (`image` features)
  - a new preparation module in `crates/smith-tools`
  - `crates/smith-tools/src/{read.rs,support.rs}`
  - the factory, which passes the provider's limits to `read`
  - `crates/smith-cli/src/{cli.rs,submission.rs,headless/}`
  - TUI attachment chips
- Docs: `docs/headless-protocol.md`, `DESIGN.md`, CLI help
