---
created_at: 2026-08-02T16:31:46Z
updated_at: 2026-08-02T17:29:48Z
completed_at: 2026-08-02T17:29:48Z
---

## 0. Approval, Baseline, and Coordination

- [x] 0.1 Approve this behavior-preserving decomposition and its explicit
  non-goals before changing Rust source.
- [x] 0.2 Confirm the overlapping steering/input work is present and record
  the exact implementation baseline; leave the separate untracked memory
  proposal untouched.
- [x] 0.3 Capture focused green baselines for `smith-tui`, `smith-cli`,
  `smith-config`, and `smith-runtime`, plus the public import paths re-exported
  by `smith-tui::app`, `smith-config::resolve`, and
  `smith-runtime::factory`.
- [x] 0.4 Coordinate with any newly approved active change that edits the same
  file family; rebase on landed behavior rather than moving stale code.

## 1. TUI Application State

- [x] 1.1 Convert `app.rs` into a compatibility facade and add private
  `state`, `pending_input`, `reducer`, `prompts`, `input`, and `resources`
  child modules without changing public paths.
- [x] 1.2 Move prepared submissions, paste/image material, accepted steers,
  rejected follow-ups, explicit queues, drain/restoration, and preview logic
  into `pending_input` with the same bounds and FIFO behavior.
- [x] 1.3 Move `EventEnvelope` reduction, speculative attempt buffering,
  terminal reconciliation, and plan/work/child projections into `reducer`
  without changing live/replay equivalence or event ordering.
- [x] 1.4 Move approval/questionnaire ownership, FIFO presentation, expiry,
  cancellation, and exit restoration into `prompts` without adding an
  implicit default response.
- [x] 1.5 Move key/mouse routing, history/search, composer, escape/exit, and
  scrolling into `input`; move palette/resource/model/reasoning/profile and
  local-result behavior into `resources`.
- [x] 1.6 Split the in-file tests into reducer, pending-input, input, prompts,
  resources, and child-lifecycle suites with shared fixtures in `tests/mod.rs`.
- [x] 1.7 Run formatting, `smith-tui` unit/end-to-end tests, and warning-denied
  Clippy before beginning the CLI extraction.

## 2. CLI Composition Root

- [x] 2.1 Reduce `main.rs` to process entry, top-level command routing, and
  truly global constants while retaining the existing private modules.
- [x] 2.2 Move resolved host construction and interactive restart/reconfigure
  ownership into `runtime_host` while preserving the single
  `smith_runtime::host` composition path.
- [x] 2.3 Move the terminal event loop and TUI event/action routing into
  `tui_driver` without changing polling, redraw, prompt, interruption, or
  shutdown behavior.
- [x] 2.4 Move typed local command execution and status/context rendering into
  `local_command`; move prepared materialization, steer/turn dispatch, and
  agent/review actions into `submission`.
- [x] 2.5 Move runtime resources, workspace/session inventory, and resume
  picker behavior into `resources`; move readiness, resolve, explain, and
  session-list command helpers into `config_command`.
- [x] 2.6 Split binary tests by host routing, local commands, submission,
  resources, and rendering fixtures without making private production items
  public.
- [x] 2.7 Run formatting, `smith-cli` tests including PTY/end-to-end coverage,
  and warning-denied Clippy before beginning resolver extraction.

## 3. Configuration Resolver

- [x] 3.1 Convert `resolve.rs` into a compatibility facade and add private
  `types`, `provenance`, `load`, `agent`, and `provider` child modules.
- [x] 3.2 Move public request/result/error/resolved-value types into `types`
  and re-export them under their current `smith_config::resolve::*` paths with
  unchanged derives, fields, displays, and serde behavior.
- [x] 3.3 Move layer/source/sourced values, overrides, contributions,
  provenance, and explanation into `provenance` without changing winner order
  or diagnostic output.
- [x] 3.4 Move discovery, TOML loading/error positions, flattening, environment
  mapping, and text parsing into `load` without changing trust boundaries or
  source attribution.
- [x] 3.5 Move profile/agent/child resolution into `agent`; move
  provider/model/reasoning/context/limit/persistence/approval/background
  resolution and validation into `provider`.
- [x] 3.6 Split resolver tests by provenance, loading, agent, and provider
  behavior and add a compile test for the existing public import surface.
- [x] 3.7 Run formatting, all `smith-config` tests, downstream CLI/runtime
  compile checks, and warning-denied Clippy before beginning renderer
  extraction.

## 4. TUI Renderer

- [x] 4.1 Convert `render.rs` into the `draw`/`draw_synced` facade and add
  private `layout`, `transcript`, `composer`, `modal`, and narrowly shared
  `helpers` modules.
- [x] 4.2 Move surface geometry, anchored row budgets, minimum-size behavior,
  and scroll synchronization into `layout`.
- [x] 4.3 Move transcript, Markdown, tool/status cards, context view, and local
  result rendering into `transcript`.
- [x] 4.4 Move composer, paste placeholders, pending input, todos, hints, and
  identity footer rendering into `composer`.
- [x] 4.5 Move approval, questionnaire, palette, history search, recovery,
  review, agent, child, and exit overlays into `modal`.
- [x] 4.6 Split snapshot tests by visual region and prove the existing normal,
  narrow, tiny, scrolling, redaction, and non-color outputs remain unchanged.
- [x] 4.7 Run formatting, `smith-tui` unit/end-to-end tests, and warning-denied
  Clippy before refactoring the factory.

## 5. Runtime Factory

- [x] 5.1 Introduce private typed stage state for prompt/context inputs,
  capabilities, durability, policy assembly, builder assembly, and delegation
  output without changing `RuntimeRequest`, `RuntimePolicy`, `SmithRuntime`, or
  `FactoryError` public paths.
- [x] 5.2 Refactor `build` into short ordered stage calls that preserve
  fail-fast host validation, credential lifetime/redaction, provider wrapping,
  tool ordering, ability sealing, checkpoint behavior, contributor/hook
  ordering, and child-route construction.
- [x] 5.3 Keep stage helpers in `factory.rs` unless a private `factory/` child
  module can own a complete stage without `pub(crate)` expansion; document
  the decision in the implementation evidence.
- [x] 5.4 Retain focused tests for adapter selection, endpoint/credential
  safety, reasoning, context policy, approval, tools, goals, and child
  composition; add a parity test if stage extraction exposes an uncovered
  policy mapping.
- [x] 5.5 Run formatting, `smith-runtime` tests including persistence and host
  integration coverage, and warning-denied Clippy.

## 6. Workspace Verification and Review

- [x] 6.1 Verify the facade files contain only public compatibility exports,
  entry orchestration, and module declarations; verify each leaf module has
  one documented responsibility and no catch-all `util` ownership.
- [x] 6.2 Compare public paths/signatures, serialized fixtures, command help,
  config explanations, TUI snapshots, and runtime policy projections with the
  recorded baseline.
- [x] 6.3 Run `cargo fmt --all -- --check`, workspace tests, PTY/end-to-end and
  persistence tests, and `cargo clippy --workspace --all-targets -- -D
  warnings`.
- [x] 6.4 Run the coordinated Agent Runtime Smith consumer-conformance suite
  required by the existing harness changes and record any external release
  gate that cannot run locally.
- [x] 6.5 Review the final diff for semantic edits, unintended visibility
  expansion, new dependencies, stale duplicate code, unrelated working-tree
  changes, and update validation evidence.
