---
created_at: 2026-09-22T20:59:10Z
updated_at: 2026-09-22T22:08:22Z
---

## Why

`cache_controller.rs` and `headless.rs` have grown into large mixed-responsibility
modules. Cache scheduling, idle compaction, cache handoff persistence, and
resume-capsule reduction currently share one source file. Headless result
projection, background-exit policy, and canonical event-stream handling share
another. The overlap makes changes harder to review and creates avoidable
conflicts between behavior that already has distinct owners.

## What Changes

- Keep the existing `smith_runtime::cache_controller` and private CLI
  `headless` module paths while moving complete responsibilities into private
  child modules.
- Move the ordinary idle-compaction lane, cache handoff persistence, and
  resume-capsule event projection out of the cache-controller orchestration
  file. Keep scheduling decisions and the long-lived controller loop in the
  parent unless a child owns a complete stage.
- Move cache-controller tests into `cache_controller/tests.rs`; split
  headless tests into output, turn-flow, and background-exit modules under
  `headless/tests/`, with shared host fixtures in `tests/mod.rs`.
- Move headless output projection/formatting, background-exit policy, and
  event-stream/terminal-output flow into private child modules. Keep the
  public-to-the-binary `run` entry point and test surface under `headless`.
- Preserve scheduling boundaries, cancellation and persistence ordering,
  event sequencing, serialized output, exit codes, and existing internal
  module paths.

## Impact

- Affected specs: `code-organization`
- Affected code: `crates/smith-runtime/src/cache_controller.rs`,
  `crates/smith-runtime/src/cache_controller/`,
  `crates/smith-cli/src/headless.rs`, and
  `crates/smith-cli/src/headless/`
- Compatibility: no public API, command-line option, configuration, serialized
  contract, or runtime behavior is intentionally changed.
- Dependencies: no new dependency is required.

## Coordination

This is a focused continuation of the archived
`refactor-large-rust-modules` change. It applies that change's rule to keep
helpers in their parent unless a private child can own a complete
responsibility. It is limited to the two named modules; concurrent factory,
configuration, and TUI work remains outside this change.

## Approval Boundary

Approval authorizes behavior-preserving source decomposition and private
visibility adjustments needed for sibling coordination in the two named
modules. It does not authorize feature work, changes to factory/config/TUI
state, public API expansion, new dependencies, or documentation/configuration
changes outside this spec change.
