---
created_at: 2026-10-16T02:27:02Z
updated_at: 2026-10-16T02:27:02Z
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

## What Changes

- **`read` returns images.** A path whose content is a PNG, JPEG, GIF or WebP
  image, within 3.5 MiB, comes back as an image part plus one line naming the
  path, media type, pixel size and byte size.
  - The type is detected from the file's signature, not its extension.
  - Other binary files are still refused.
  - A larger image is refused with its size and the bound.
- **Text-only models are unchanged in kind.** The runtime already replaces
  image parts with an `[image <id>: …]` placeholder for a text-only model, and
  `view_image` can inspect that id when a vision model is configured.
- **TUI `@FILE` attaches an image as an image**, under the same bound, like a
  pasted clipboard image.
- **`smith -p --image <PATH>`** (repeatable) attaches images to the prompt.
  - The caller names the file, so it is read without approval and is not
    confined to the project.
  - The same type and size bounds apply.
- `cli/*` profiles keep their own file tools. `--image` with a `cli/*` profile
  fails with an explicit message instead of dropping the image.

## Impact

- Affected specs: `tool-execution` (ADDED), `client-surfaces` (ADDED)
- Affected code:
  - `crates/smith-tools/src/{read.rs,support.rs,image_header.rs}`
  - `crates/smith-cli/src/{cli.rs,submission.rs,headless/}`
  - TUI attachment chips
  - docs: `docs/headless-protocol.md`, `DESIGN.md`, the configuration skill if
    it lists `read`'s limits
