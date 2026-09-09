---
created_at: 2026-09-09T21:59:52Z
updated_at: 2026-09-09T22:37:20Z
---

## Why

Smith currently treats a model's advertised maximum output as the default
per-request output reserve. Models such as `xai/grok-4.6` legitimately
advertise a 500,000-token output ceiling inside a 500,000-token context window,
so Smith reserves the entire window and disables the model even though a normal
bounded request would be usable. Users should not have to invent a local model
override to select a catalog-backed model.

## What Changes

- Separate the provider-advertised maximum output ceiling from Smith's
  effective per-request output budget.
- When no explicit request limit exists, derive a deterministic automatic
  budget from the frozen model limits: no more than 32,768 tokens, one quarter
  of the context window, the model's output ceiling, or the space remaining
  after the reasoning reserve while preserving input space.
- Use the same derived value as the provider request limit and as the default
  context output reserve. Explicit request limits and explicit context reserves
  keep their existing precedence and are never silently clamped.
- Show both the model ceiling and the effective automatic/configured request
  budget in model-selection diagnostics.
- Keep models disabled when their catalog metadata is incomplete, their coding
  capabilities are incompatible, or explicit reserves still consume the
  usable context window.

## Impact

- Affected specs: `configuration`, `provider-runtime`, `client-surfaces`.
- Affected code: model inventory and resolution in `smith-config`; request and
  context policy composition in `smith-runtime`; picker metadata in
  `smith-cli`; focused configuration, runtime, and TUI tests; configuration
  documentation.
- Compatibility: existing explicit `max_output_tokens` and
  `context.output_reserve` values retain precedence. Runs that omit both will
  begin sending a bounded `max_output_tokens` value instead of leaving the
  provider request unspecified.
- Security/cost: the automatic budget is a maximum, not a target. It prevents
  an advertised ceiling from becoming an unexpectedly huge default request and
  remains bounded by the immutable model profile.

## Non-Goals

- Do not alter or rewrite provider-advertised context, input, or output
  ceilings.
- Do not infer limits for models absent from explicit or trusted catalog
  metadata.
- Do not claim that a catalog-advertised model is entitled for the connected
  account, plan, or region.
- Do not make image-only, non-tool-calling, deprecated, or otherwise
  incompatible models selectable.
