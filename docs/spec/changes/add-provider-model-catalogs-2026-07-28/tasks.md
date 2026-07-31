---
created_at: 2026-07-29T03:50:50Z
updated_at: 2026-07-29T04:42:27Z
completed_at: 2026-07-29T04:42:27Z
---

## 1. Catalog contract and cache

- [x] 1.1 Add typed Models.dev provider/model schemas with strict identifier,
  modality, capability, status, numeric-limit, response-size, and source-origin
  validation.
- [x] 1.2 Add exact normalized endpoint bindings for OpenRouter and Z.AI Coding
  Plan without importing remote endpoint, adapter, environment, header, or
  credential configuration.
- [x] 1.3 Add a reproducible generator and reviewed embedded seed containing
  only the supported provider catalogs, with source digest, schema revision,
  and retrieval timestamp.
- [x] 1.4 Implement an injectable catalog loader that selects a valid last-good
  cache or embedded seed, schedules bounded credential-free HTTPS refresh, and
  atomically publishes only fully validated responses.
- [x] 1.5 Build an immutable cached-remote `ModelCatalogSource` with normalized
  enforcement limits, capabilities, modalities, revision, and retrieval
  provenance.

## 2. Inventory and runtime integration

- [x] 2.1 Extend the pure local inventory builder to accept a read-only catalog
  snapshot and add provider-qualified candidates only for configured,
  adapter-valid, exactly bound providers.
- [x] 2.2 Merge explicit model fields above cached catalog fields, preserve the
  embedded trusted-model layer, and represent deprecated, incompatible, or
  invalid catalog entries with deterministic omission/disabled reasons,
  including a candidate whose effective reserves leave no input budget.
- [x] 2.3 Prepare one catalog snapshot in CLI host orchestration, use it for
  `SelectionInventory`, and inject the same frozen source through
  `RuntimeRequest.catalog_sources` on initial start and every picker-driven
  reconfiguration.
- [x] 2.4 Extend model/provider resource entries with catalog display name,
  selectable model counts, limits/capability summary, disabled reason, and
  source age/revision while preserving active/profile markers.
- [x] 2.5 Keep direct `/model <provider/model>` and unqualified active-provider
  resolution coherent for catalog-only models, with atomic provider/model
  application and unchanged explicit ambiguity handling.

## 3. Verification

- [x] 3.1 Add deterministic catalog fixtures covering multiple OpenRouter and
  Z.AI Coding Plan models, nested model IDs, optional input limits, tool/text
  capability filtering, deprecated entries, and explicit-field precedence.
- [x] 3.2 Add cache tests for missing, stale, corrupt, oversized, truncated,
  wrong-origin, redirect, timeout, concurrent refresh, atomic publication, and
  last-good/embedded offline fallback behavior.
- [x] 3.3 Add runtime composition tests proving a catalog-only selection
  resolves immutable limits from the same snapshot, explicit configuration
  wins, and no provider/credential request occurs while listing.
- [x] 3.4 Add reducer/render tests with hundreds of catalog entries, provider
  filtering, disabled reasons, active markers, direct selection, cancellation,
  narrow terminals, and deterministic ordering.
- [x] 3.5 Add process-level coverage that a GLM quick-start configuration and a
  configured OpenRouter endpoint expose additional models in `/model`, select
  one, rebuild safely, and remain usable with networking disabled.

## 4. Documentation and gates

- [x] 4.1 Document catalog source/provenance, supported endpoint bindings,
  refresh/offline behavior, explicit override precedence, entitlement caveats,
  and cache recovery in `README.md` and `DESIGN.md`.
- [x] 4.2 Run strict spec validation, the seed reproducibility check,
  `cargo fmt --check`, warning-denied Clippy, focused catalog/picker tests, and
  `cargo test --workspace`.
