---
created_at: 2026-11-27T07:22:39Z
updated_at: 2026-11-27T08:43:24Z
---

## Why

Smith already speaks the native Claude API: the `anthropic-messages` adapter
runs through Agent Runtime's `AnthropicProvider`, and gateways use it today.
Connecting Anthropic directly is still a manual chore. The setup entry is not
offered by `/connect`, has no catalog, and asks the user to type a model ID,
context window, and output limit. Users should connect an Anthropic API key
and pick a Claude model the way they connect Google or xAI.

## What Changes

- Add a trusted, connectable `anthropic` provider descriptor: `/connect
  anthropic` and guided setup take an API key through the existing reviewed
  credential methods, bind the fixed `https://api.anthropic.com/v1` endpoint,
  and create a `claude` profile whose default model is `claude-opus-5-5`.
- Add Models.dev's `anthropic` provider to Smith's bounded catalog allow-list
  and regenerate the embedded seed. Model limits, modalities, tool support,
  prices, and effort ladders come from that frozen snapshot; setup writes no
  `[models]` record.
- Bind `anthropic-messages` at the exact Anthropic endpoint (written, or the
  adapter's default when `base_url` is omitted) to the `anthropic` catalog.
  Gateways at any other endpoint keep today's behavior and inherit nothing.
- Derive `anthropic-effort` reasoning controls from the catalog's advertised
  effort ladder for that exact binding: mandatory-on, default effort `high`.
  Explicit `[models."provider/model".reasoning]` metadata still wins. Models
  that advertise only a toggle get no effort controls.
- At the official endpoint, set the adapter's thinking block binding to
  `drop_block`. Current Claude models reject replayed thinking blocks once the
  earlier history changes, and Smith's compaction and summary calls change it.
  Requests that reach the provider without a reasoning selection (the
  semantic-summary call) run at the lowest advertised effort, so they carry
  the control too.
- Repurpose the existing `anthropic-messages` setup entry as the
  Anthropic-compatible gateway path. It collects the endpoint instead of fixing
  it to the official one, so the official endpoint has a single setup route.

## Impact

- Affected specs: `client-surfaces`, `configuration`, `provider-runtime`.
- Affected Smith code: `crates/smith-config/src/setup.rs` (descriptors,
  constants, quick-key plan), `crates/smith-config/src/catalog.rs` (catalog
  identity and binding), `crates/smith-runtime/src/model_catalog.rs` and
  `data/models-dev-seed.json` (allow-list, seed),
  `crates/smith-runtime/src/reasoning.rs` and `reasoning/provider.rs`
  (catalog-driven dialect, one-shot effort fill),
  `crates/smith-runtime/src/factory/provider/adapter.rs` (block binding),
  `crates/smith-cli/src/setup*` and `crates/smith-tui/src/setup*` (quick-key
  plumbing, review text, picker order), setup snapshots and docs.
- Upstream: none required. Smith pins Agent Runtime `730f1ef` (v0.12.1), whose
  Anthropic adapter already handles streaming, tools, images, prompt caching,
  hosted web search, adaptive effort, and an opt-in thinking block binding.
- Coordination: the still-active `add-google-gemini-provider-2026-08-02`
  change adds requirements to the same three capabilities. This change adds
  separate requirements and modifies only "Explicit native Anthropic reasoning
  metadata", which that change does not touch.
- Known upstream follow-up (not blocking): the adapter sends
  `thinking: {"type": "adaptive"}` without `display`, so current Claude models
  return empty thinking text. A separate Agent Runtime change can opt into
  summarized display; this change does not edit the sibling repository.
- Security: the request carries `x-api-key`. The key stays in the reviewed
  credential store or environment reference and follows the existing redaction
  contract. Catalog fields such as Models.dev's `env` and `npm` are not
  imported.

## Approval Boundary

Approval covers a key-based Anthropic connection, the Models.dev `anthropic`
catalog binding at the exact official endpoint, catalog-derived
`anthropic-effort` controls, and the gateway-setup relabel. It does not cover
Claude subscription (Pro/Max) sign-in, Amazon Bedrock, Google Vertex AI,
Microsoft Foundry, fast mode, server-side refusal fallbacks, a newer
`anthropic-version`, or Agent Runtime changes.
