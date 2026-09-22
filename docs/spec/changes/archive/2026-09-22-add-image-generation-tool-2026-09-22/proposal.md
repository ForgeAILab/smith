---
created_at: 2026-09-22T16:40:00-04:00
updated_at: 2026-09-22T22:08:21Z
---

# Proposal: Add an image generation tool

## Why

Codex lets the model generate and edit images on a ChatGPT subscription.
Smith already holds the same ChatGPT credentials and can return image content
from tools, but it has no image tool.

## How Codex does it (`../codex/codex-rs/ext/image-generation`)

- Codex exposes one function tool, `image_gen.imagegen`, with the arguments
  `prompt`, optional `referenced_image_paths` (at most 5 absolute paths), and
  optional `num_last_images_to_include` (1–5 recent conversation images).
  The two reference arguments are mutually exclusive.
- The tool calls the Images API on the active provider's base URL, not the
  Responses stream:
  - `POST {base}/images/generations` with
    `{prompt, model: "gpt-image-2", size: "auto", quality: "auto", background: "auto"}`
  - `POST {base}/images/edits` when reference images are given, with
    `images: [{image_url: <data URL>}]`
- For ChatGPT, `{base}` is `https://chatgpt.com/backend-api/codex`, called with
  the same bearer token, `chatgpt-account-id`, and `originator` headers as the
  model stream. The response is `data[0].b64_json` (PNG).
- The tool is offered only for OpenAI-authenticated providers, and not on the
  ChatGPT Free plan.
- Codex decodes the PNG and saves it to
  `<codex_home>/generated_images/<thread>/<call>.png`. The tool result
  returns the image as `input_image` plus a text hint with the saved path, so
  the model can see and reuse it.

## What Changes

- A built-in `generate_image` tool offers the same contract: `prompt`,
  optional `reference_paths` (at most 5 files inside the workspace or Smith's
  generated-image directory), and optional `recent_images` (1–5). The two
  reference arguments are mutually exclusive.
- The tool is registered only when the active provider is ChatGPT
  (`chatgpt-responses`) or the OpenAI Platform endpoint. It reuses that
  provider's credential lease, so an account pool rotates for images too.
- Output is saved to `~/.smith/generated_images/<session>/<call>.png`. The
  tool result carries the image plus the saved path. The TUI shows a
  completed-image row with the path and dimensions. Inline terminal image
  rendering is a non-goal.
- The tool declares a network effect, so it goes through the existing
  approval and posture policy. It is unavailable in `plan` posture.
- `[tools.image_generation]` adds `enabled` (default true when the provider
  supports it), `model` (default `gpt-image-2`), and `quality` / `size`
  defaults (`auto`).
- **Agent Runtime dependency:** tool-output bounding currently measures an
  image by the length of its data-URL string, so a generated image (about
  1–2 MB of base64) is always replaced with a truncation marker. The runtime
  must size image parts by their image cost instead, as context sizing
  already does. This lands in the agent-runtime compat branch, and then the
  pins are bumped.

## Non-Goals

- The hosted Responses `image_generation` tool type (Codex does not use it
  either).
- Rendering images inline in the terminal (kitty or iTerm protocols).
- Other image providers (Gemini, xAI). The backend is one trait, so they can
  be added later.

## Impact

- Affected specs: tool-execution, configuration, tool-call-display
- Affected code: new `smith-tools/src/image.rs` (tool, schema, and the
  image-loading helpers), `smith-runtime` (ChatGPT/OpenAI images backend next
  to `chatgpt.rs`, tool registration in `factory.rs`), `smith-config`
  (`[tools.image_generation]`), and `smith-tui` (row rendering).
- agent-runtime-compat: `agent-runtime-core/src/tool.rs` (`rendered_size`
  and `truncate_part` for `ContentPart::Image`).

## Open Questions

- Checkpoint size: each generated image adds its base64 data to the session
  checkpoint. Should the checkpoint keep only the saved path and rehydrate
  the image on resume? The proposal keeps the inline data for v1.
