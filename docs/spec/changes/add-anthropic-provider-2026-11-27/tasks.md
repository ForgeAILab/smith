---
created_at: 2026-11-27T07:22:39Z
updated_at: 2026-11-27T08:43:24Z
completed_at:
---

## 0. Approval

- [x] 0.1 Approve the key-based `anthropic` connection, the exact-endpoint
  catalog binding, catalog-derived `anthropic-effort` controls, and the
  gateway-setup relabel.
- [x] 0.2 Settle the default effort: `high`, mandatory-on, with `drop_block`
  block binding at the official endpoint.

## 1. Catalog

- [x] 1.1 Add `ANTHROPIC_CATALOG_PROVIDER = "anthropic"` in
  `smith-config/src/catalog.rs` and append it to `EXPECTED_PROVIDERS` in
  `smith-runtime/src/model_catalog.rs` and the generator script.
- [x] 1.2 Regenerate `crates/smith-runtime/data/models-dev-seed.json`
  reproducibly from Models.dev and confirm the `anthropic` entries normalize
  (limits, modalities, tool calling, prices, effort ladders) within the
  existing bounds.
- [x] 1.3 Map `(anthropic-messages, None)` and
  `(anthropic-messages, https://api.anthropic.com/v1)` to the `anthropic`
  catalog in `catalog_provider_for`, plus the matching modality mapping.
- [x] 1.4 Add binding tests: exact and normalized endpoint bind; a trailing
  path, gateway host, or other adapter kind does not; explicit `[models]`
  records still win; gateway setup ID fallback matches the exact Anthropic
  entry.

## 2. Reasoning and request safety

- [x] 2.1 Add the exact-endpoint branch in `resolve_reasoning_policy`: a
  catalog-advertised effort ladder yields `anthropic-effort` with those
  efforts, mandatory-on, and default effort `high` when advertised.
- [x] 2.2 Keep explicit model reasoning metadata authoritative, and give
  toggle-only catalog models presence-only reasoning.
- [x] 2.3 Set `ThinkingBlockBinding::DropBlock` on `AnthropicConfig` at the
  exact official endpoint, for main and child routes.
- [x] 2.4 Give `ReasoningDialectProvider` the lowest advertised effort and
  fill it into requests that arrive with no reasoning config.
- [x] 2.5 Add tests: `claude-opus-5-5` exposes `low` through `max` with default
  `high`; an unsupported effort fails in preflight; `claude-haiku-4-5` gets no
  controls; a gateway endpoint with a matching model name gets no controls and
  no binding; only the official endpoint gets `DropBlock`; a reasoning-less
  request is filled. The wire shape (adaptive thinking, effort,
  `block_binding`, one beta header) is covered by Agent Runtime's pinned
  adapter tests for a `DropBlock` config with an effort.

## 3. Setup and connection

- [x] 3.1 Add `ANTHROPIC_PROVIDER`, `ANTHROPIC_PROFILE = "claude"`, and
  `ANTHROPIC_DEFAULT_MODEL = "claude-opus-5-5"` plus a connectable `anthropic`
  descriptor using `QuickKeySetup::Anthropic` and `CREDENTIAL_METHODS`.
- [x] 3.2 Thread the new quick key through `smith-cli` (choices, plan, setup
  outcome) and `smith-tui` (`SetupQuickKey`, `SetupAction`,
  `SetupSubmission`, review). The plan writes kind, `base_url`, and credential
  reference, and leaves `max_output_tokens` and `context` unset.
- [x] 3.3 Relabel the `anthropic-messages` descriptor as the
  Anthropic-compatible gateway path that collects provider name and endpoint,
  keeping its setup id.
- [x] 3.4 Populate `/model` for `anthropic` from the frozen catalog with
  provenance and disabled reasons.
- [x] 3.5 Update the connectable-order, handler-coverage, and descriptor tests,
  plus the `setup_first_step` PTY snapshots.

## 4. Verification and docs

- [x] 4.1 Run the Smith gate (fmt, clippy on Rust 1.88, tests with
  `--no-fail-fast`, the CI `--no-default-features` steps).
- [ ] 4.2 Run the existing opt-in live conformance suite against a
  first-party key (`SMITH_LIVE_CONFORMANCE_MODELS=anthropic/claude-haiku-5-5`);
  it already covers streamed tool turns for any configured route. Not run:
  no first-party Anthropic key was available.
- [ ] 4.3 Run a manual tmux check: connect, select `claude-opus-5-5`, run one
  tool turn, change `/effort`, and resume the session. Offline part done
  (startup, `/effort` ladder, `/think off` refusal with a fake key); the
  turn and resume need a first-party key.
- [x] 4.4 Document `/connect anthropic` and the gateway path in the
  configuration docs.
