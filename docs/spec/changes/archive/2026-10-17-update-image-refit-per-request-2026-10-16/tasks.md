---
created_at: 2026-10-16T05:37:09Z
updated_at: 2026-10-17T20:03:09Z
completed_at: 2026-10-16T09:48:13Z
---

## 1. Implementation
- [x] 1.1 A header-only "fits" check in `smith_tools::image_preparation`, and
      a data-URL entry point that returns the prepared data URL.
- [x] 1.2 `ImageFittingProvider` with the bounded cache, wrapping the
      provider in `factory/provider.rs` for roots and children.
- [x] 1.3 An image that cannot be prepared becomes
      `[image omitted: <reason>]` in that request.

## 2. Tests
- [x] 2.1 A request carrying a WebP and a 3000 px PNG sent through an xAI
      wrapper arrives with PNGs within 2048 px. An image that fits arrives
      byte for byte.
- [x] 2.2 The second identical request does not decode again (cache hit
      observable in the test).
- [x] 2.3 A corrupt image data URL becomes the omitted note, and the request
      still goes out.
- [x] 2.4 Session-level: an image read under a ChatGPT profile, followed by
      `/model` to an xAI profile, sends a fitted image (fake provider
      capturing requests).

## 3. Verification
- [x] 3.1 Gate, and a live check: read an image with `visual` (ChatGPT),
      switch with `/model` to another provider, and ask about the image
      again.
      (2026-10-16: gate green, 2,926 tests. Live: a 1024 px WebP read under
      `chatgpt/gpt-6-astra`, then `/model openrouter/openai/gpt-4o-mini`
      (openai-compatible, so PNG/JPEG-only limits) described it correctly.
      xAI was not usable: its stored login fails credential refresh even
      for text-only prompts on v0.6.0.)
