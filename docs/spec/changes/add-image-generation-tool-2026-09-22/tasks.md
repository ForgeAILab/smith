---
created_at: 2026-09-22T16:40:00-04:00
updated_at: 2026-09-22T22:08:21Z
completed_at: 2026-09-22T22:08:21Z
---

# Tasks: Add an image generation tool

## 1. Agent Runtime

- [x] 1.1 In `agent-runtime-core/src/tool.rs`, size `ContentPart::Image` by
      its image cost instead of its URL length, so an image is never replaced
      by a truncation marker when it fits. Add a test with a 2 MB data URL.
- [x] 1.2 Push the compat branch and bump Smith's six `rev` pins.

## 2. Backend

- [x] 2.1 Add an images client for `{base}/images/generations` and
      `{base}/images/edits` with a typed request and response, bounded
      response size, and error mapping that shares the provider's retry
      classification.
- [x] 2.2 Authenticate through the active ChatGPT lease (bearer,
      `chatgpt-account-id`, `originator: smith`) or the OpenAI Platform API
      key.

## 3. Tool

- [x] 3.1 Add `generate_image` with its argument schema and validation:
      at most 5 references, the two reference arguments mutually exclusive,
      and reference paths inside the workspace or the generated-image
      directory.
- [x] 3.2 Save the PNG under `~/.smith/generated_images/<session>/<call>.png`
      (mode 0600, sanitized names) and return the image plus a path hint.
- [x] 3.3 Register the tool only for supported providers, declare its
      network effect, and exclude it from `plan` posture.

## 4. Config and display

- [x] 4.1 Add `[tools.image_generation]` (`enabled`, `model`, `quality`,
      `size`) with provenance and `config explain` support.
- [x] 4.2 Render in-progress, completed (path and dimensions), and failed
      rows in the TUI.

## 5. Docs and verification

- [x] 5.1 Document the tool, its provider support, and where files are saved
      in `docs/configuration.md`.
- [x] 5.2 Add tests with a fake HTTP backend for generation, editing, and
      errors, then run `cargo test` and clippy.
