---
created_at: 2026-10-16T05:37:09Z
updated_at: 2026-10-17T20:03:09Z
---

## Why

v0.6.0 prepares an image once, when it enters the conversation, against the
provider active at that moment. After a `/model` switch to a stricter
provider, every later request still carries those images, and a provider
that rejects one fails the whole turn, for every turn after it.

Examples:
- a WebP read under ChatGPT, then sent to xAI;
- a 2576 px image read under Anthropic, then sent to an OpenAI-compatible
  endpoint;
- an MCP screenshot that was never prepared at all.

The owner asked on 2026-10-16 that images be fitted again so the chat keeps
working.

## What Changes

- **Every request is fitted to the provider that serves it.** Smith wraps
  each provider so that, just before a request is sent, every image part
  that does not fit that provider's limits is re-prepared:
  - downscaled;
  - converted to an accepted format;
  - re-encoded as needed.
  This uses the same table and preparation step as ingestion. Images that
  fit are sent unchanged.
- This covers every image source in the request: `read`, `@FILE`, `--image`,
  pasted images, MCP results and generated images.
- **Repeated images cost nothing extra.** Results are cached by content and
  limits, so an image re-sent on every request inside the image window is
  prepared once per provider. The transform is deterministic, so the
  provider's prompt cache stays stable.
- **An image that cannot be prepared** becomes a short text note in that
  request instead of failing the turn:
  `[image omitted: <reason>]`.
- Canonical history is unchanged. It keeps the image as ingested, so a later
  switch to a provider with higher limits still sends the ingested size.
- Children use their own provider's limits.

## Impact

- Affected specs: `tool-execution` (MODIFIED "Images are fitted to the active
  provider")
- Affected code: a provider wrapper in `crates/smith-runtime/src/factory/`
  (beside the response-policy and reasoning-dialect wrappers), a bounded
  cache, and `smith_tools::image_preparation` (a cheap "already fits" check
  from the header).
