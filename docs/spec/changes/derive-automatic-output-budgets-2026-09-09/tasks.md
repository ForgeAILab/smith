---
created_at: 2026-09-09T21:59:52Z
updated_at: 2026-09-09T22:37:20Z
completed_at: 2026-09-09T22:37:20Z
---

## 1. Shared output-budget policy

- [x] 1.1 Add a typed Smith-owned helper that derives the effective request
  output budget and default output reserve from explicit values, frozen model
  limits, and the reasoning reserve.
- [x] 1.2 Prove the automatic 32,768-token and quarter-context bounds, small
  model ceilings, reasoning-reserve exhaustion, checked arithmetic, and
  explicit-value precedence with focused `smith-config` tests.

## 2. Catalog inventory

- [x] 2.1 Replace the catalog inventory's direct model-ceiling fallback with
  the shared policy and retain fail-closed diagnostics for invalid catalog
  metadata and explicit reserve conflicts.
- [x] 2.2 Carry the effective request budget and its automatic/configured source
  into resource metadata, provider selectable counts, and direct model
  selection.
- [x] 2.3 Add regression coverage for a valid 500,000-context/500,000-output
  catalog model becoming selectable with a 32,768-token automatic request
  budget and no local model override.

## 3. Runtime composition

- [x] 3.1 Use the shared result for ordinary `LoopConfig.max_output_tokens` and
  `ContextPolicy.output_reserve` in root and child runtimes.
- [x] 3.2 Include the effective automatic values in immutable runtime/harness
  identity and preserve identical values across retries, tool continuations,
  and a frozen catalog snapshot.
- [x] 3.3 Add factory/composition tests proving picker/runtime coherence,
  explicit override behavior, and pre-provider failure when explicit reserves
  consume the context window.

## 4. User-facing behavior and documentation

- [x] 4.1 Update model-picker detail to distinguish the advertised output
  ceiling from the automatic or configured request budget, with bounded labels
  in narrow terminals.
- [x] 4.2 Update xAI setup commentary and configuration documentation so users
  are not instructed to provide model-limit guesses for catalog-backed models.
- [x] 4.3 Add CLI/TUI regression coverage for Grok-shaped catalog metadata and
  disabled explicit-reserve diagnostics.

## 5. Verification

- [x] 5.1 Run `cargo fmt --check`, focused `smith-config`, `smith-runtime`,
  `smith-cli`, and `smith-tui` tests, and Clippy with warnings denied for the
  affected crates.
- [x] 5.2 Run the workspace test suite and record any unrelated pre-existing
  failures without weakening the focused acceptance coverage.
