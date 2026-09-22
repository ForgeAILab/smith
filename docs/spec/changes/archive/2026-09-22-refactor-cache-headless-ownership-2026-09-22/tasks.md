---
created_at: 2026-09-22T20:59:10Z
updated_at: 2026-09-22T22:08:22Z
completed_at: 2026-09-22T22:08:22Z
---

## 0. Baseline and Scope

- [x] 0.1 Approve the behavior-preserving decomposition and keep the change
  limited to the cache-controller and headless modules.
- [x] 0.2 Read the archived large-module refactor guidance; preserve existing
  uncommitted work owned by other changes.

## 1. Cache Controller Responsibilities

- [x] 1.1 Extract ordinary idle-compaction admission, execution, persistence,
  and outcome projection into a private child module.
- [x] 1.2 Extract cache-operation handoff persistence into a private child
  module, preserving the single Runtime operation and persistence rollback
  behavior.
- [x] 1.3 Extract Runtime-event reduction into the resume-capsule projection
  child, preserving event-gap handling, watermarks, metadata bounds, and
  synthetic-attempt filtering.
- [x] 1.4 Keep the controller's established module path and avoid widening
  implementation visibility beyond what sibling orchestration requires.
- [x] 1.5 Move the existing controller tests to `cache_controller/tests.rs`
  while retaining access through the parent module.

## 2. Headless Responsibilities

- [x] 2.1 Move result/usage/lifecycle projections and text/JSON formatting to a
  private output child module without changing serialized field names,
  omission rules, or diagnostic text.
- [x] 2.2 Move background-exit decisions, task waiting/stopping, and their
  report projection to a private background child module without changing
  policy behavior or task registry ordering.
- [x] 2.3 Move canonical event consumption, sequence checks, and terminal
  shutdown output to a private run-flow child module without changing event
  order or process exit codes.
- [x] 2.4 Keep the existing `headless::run` entry point and private test access
  through the parent module, without making implementation items public.
- [x] 2.5 Split headless tests into output, turn-flow, and background-exit
  modules under `headless/tests/`, keeping shared fixtures in `tests/mod.rs`
  and assertions unchanged.

## 3. Verification

- [x] 3.1 Format the touched Rust files and compile the `smith-runtime` and
  `smith-cli` packages.
- [x] 3.2 Run focused existing tests for cache lifecycle/idle compaction,
  resume-capsule projection, headless output, and background-exit policy.
  `cargo test -p smith-cli headless::tests` passed 40/40 and
  `cargo test -p smith-runtime cache_controller::tests` passed 11/11.
  `cargo check -p smith-config -p smith-runtime -p smith-cli -p smith-tui
  --all-targets` also passed after the split fixture paths were corrected.
- [x] 3.3 Review the diff for behavior changes, unintended visibility
  expansion, duplicate code, and edits outside the approved files.
