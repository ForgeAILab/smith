---
created_at: 2026-08-02T21:10:27Z
updated_at: 2026-08-02T21:42:11Z
---

## Why

Smith should support Gemini through Google's native API rather than its
OpenAI-compatibility layer. Users should connect a key and select a model
without copying endpoints, token limits, modalities, tool support, or
reasoning controls into the main configuration file.

## What Changes

- Add a coordinated Agent Runtime adapter for Google's native Interactions API,
  including typed streamed steps, function calls/results, thinking, usage,
  caching observations, cancellation, structured output, and loss-aware
  provider continuation data.
- Add a trusted Smith provider descriptor named `google` with a fixed native
  endpoint and existing reviewed API-key enrollment. Users do not configure an
  endpoint or authorization header.
- Add the Models.dev `google` provider to Smith's embedded/last-good catalog.
  Model IDs, limits, modalities, capabilities, reasoning support, and effort
  names resolve from the same frozen snapshot used by `/model` and runtime
  preflight.
- Keep only the stable default model ID (`gemini-3.6-flash`) as Smith product
  policy. Its metadata comes from Models.dev; setup writes no `[models]` block.
- Add optional `models.toml` and `models.local.toml` files dedicated to custom
  model metadata and explicit overrides. New setup flows stop writing model
  records into `config.toml`; existing records remain readable for a bounded
  migration window.
- Make `/connect google` plus model selection the normal flow. The main
  `config.toml` retains profiles, provider selection, behavior, and credential
  references, while catalog details remain outside it.
- Verify native stateless continuation, thought/signature preservation,
  streamed tool calls, resume, usage, cancellation, authentication failures,
  and redaction with deterministic fixtures plus an opt-in spend-bounded live
  test.

## Impact

- Affected specs: `client-surfaces`, `configuration`, `provider-runtime`.
- Affected Smith code: `crates/smith-config` file models, discovery, setup
  descriptors, catalog bindings, and migration; `crates/smith-runtime` factory
  composition, catalog generation, and conformance tests; setup/connection
  surfaces in `crates/smith-cli` and `crates/smith-tui`; configuration docs.
- Upstream dependency: Agent Runtime needs a separately proposed, approved,
  conformance-tested native Gemini Interactions adapter plus exact replay of
  bounded provider continuation data. This proposal does not authorize edits
  in the sibling repository.
- Active-change coordination: extends the approved provider-catalog and
  `/connect` work and MUST reuse their inventory, credential, transaction, and
  safe-boundary runtime replacement paths.
- Security impact: native requests carry an API key and opaque thought
  signatures. Both remain secret/redaction-sensitive; stateless mode prevents
  Smith from silently depending on Google-hosted conversation storage.

## Approval Boundary

Approval covers a native, stateless Gemini Interactions provider, automatic
Models.dev metadata, and the `models.toml` split. It does not cover Vertex AI,
Google-hosted tools, server-side conversation storage, arbitrary endpoint
overrides, or changes to the sibling Agent Runtime until its coordinated
proposal is separately approved.
