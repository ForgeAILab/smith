---
created_at: 2026-08-04T18:00:00Z
updated_at: 2026-08-04T18:00:00Z
---

## Why

xAI's Grok models are catalog-backed at `https://api.x.ai/v1` and advertise
reasoning effort ladders (for example `low`/`medium`/`high` on Grok 4.5), but
Smith only auto-grants adjustable reasoning controls on the OpenAI, OpenRouter,
Z.AI Coding Plan, and native Gemini endpoints. On xAI, catalog `reasoning =
true` stays presence-only, so `/effort` and profile `reasoning.effort` refuse
before the provider is called even though the Responses adapter can already
carry a typed OpenAI-style effort selection.

## What Changes

- Treat the exact normalized xAI endpoint as an endpoint that normalizes
  OpenAI-effort controls for every catalog-advertised reasoning model it serves.
- Prefer Models.dev advertised effort ladders when present; otherwise fall back
  to the universal `low`/`medium`/`high` ladder already used for OpenAI.
- Keep `off` representable only when the ladder advertises `none`, matching the
  existing OpenAI-effort dialect rules.
- Leave unknown endpoints fixed unless explicit trusted per-model metadata names
  a dialect. Do not invent a new wire dialect.
- Document the xAI endpoint beside the other normalized control endpoints.

## Impact

- Affected specs: `provider-runtime`, `configuration`
- Affected code: `crates/smith-runtime` reasoning policy resolution and tests;
  configuration/design docs that list normalized endpoints
- Compatibility: additive capability for existing xAI bindings; sessions without
  an effort override keep provider default behavior
- Network behavior: no new probe; control shape still comes from the frozen
  catalog snapshot plus the exact endpoint trust boundary

## Approval Boundary

Approval authorizes auto-granting the existing `openai-effort` dialect on the
exact xAI catalog endpoint for catalog-advertised reasoning models, using the
same ladder/`none`/mandatory-on rules as OpenAI. It does not authorize guessing
controls from a provider name alone, inventing xAI-specific vendor extensions,
or changing non-xAI endpoint behavior.
