---
created_at: 2026-08-04T18:00:00Z
updated_at: 2026-08-08T22:39:11Z
completed_at: 2026-08-08T22:39:11Z
---

## 1. Reasoning policy

- [x] 1.1 Grant `openai-effort` on the exact xAI catalog endpoint for
  catalog-advertised reasoning models, preferring Models.dev ladders and the
  universal `low`/`medium`/`high` fallback.
- [x] 1.2 Keep `off` only when `none` is advertised; leave unknown endpoints
  presence-only.
- [x] 1.3 Add focused unit tests for ladder grant, catalog refinement, and
  mandatory-on `off` refusal.

## 2. Docs and truth specs

- [x] 2.1 Document the xAI endpoint beside other normalized control endpoints in
  configuration and design docs.
- [x] 2.2 Land the provider-runtime delta for the xAI effort scenario.

## 3. Verification

- [x] 3.1 Run focused reasoning tests (`cargo test -p smith-runtime reasoning`:
  27 passed, including `xai_endpoint_grants_the_effort_ladder_without_an_off_switch`).
