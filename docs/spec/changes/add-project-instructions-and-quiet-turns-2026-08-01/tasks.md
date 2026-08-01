---
created_at: 2026-08-01T17:26:57Z
updated_at: 2026-08-01T19:45:18Z
completed_at: 2026-08-01T19:45:18Z
---

## 0. Approval and Baselines

- [x] 0.1 Approve this proposal, design, and all three capability deltas before
  implementation.
- [x] 0.2 Capture current prompt fragments/cache fingerprints, standard
  interactive/headless host composition, child prompt inheritance, and TUI
  successful/non-success terminal fixtures.
- [x] 0.3 Record the pre-change fact that the standard factory leaves project
  context empty and that successful completion creates local transcript
  decoration not present in the canonical runtime event.

## 1. Bounded Project-Instruction Discovery

- [x] 1.1 Add a host-owned immutable project-instruction snapshot with source
  label, content-derived revision/digest, bounded UTF-8 body, and redaction-safe
  debug behavior.
- [x] 1.2 Load only `<canonical-project-root>/AGENTS.md` before provider/session/
  terminal startup; accept absence and fail clearly for symlink, non-regular,
  unreadable, non-UTF-8, outside-root, or over-32-KiB content.
- [x] 1.3 Wire the identical loader into standard TUI and headless composition,
  keep direct runtime embedders explicit, and preserve complete system-prompt
  override semantics.
- [x] 1.4 Add deterministic filesystem/preflight tests for missing, valid,
  changed, symlinked, oversized, invalid-UTF-8, and canonical-root cases.

## 2. Prompt, Cache, and Child Composition

- [x] 2.1 Add a required developer-instruction fragment dedicated to the
  project snapshot, separate from optional retrieval-style `project_context`.
- [x] 2.2 Derive its revision from exact source/content and prove unchanged
  Smith product fragments retain their existing independent revisions.
- [x] 2.3 Include the project revision in runtime composition diagnostics and
  exact policy/cache fingerprints without copying the raw body into canonical
  user history.
- [x] 2.4 Clone the parent's exact snapshot into direct child factories and
  prove spawn/follow-up/resume do not reread mutable workspace instructions.
- [x] 2.5 Prove a mid-runtime file edit does not change the active plan, while a
  newly constructed runtime observes the new revision and cannot claim the old
  exact cache identity.

## 3. Quiet Successful Turn Terminals

- [x] 3.1 Remove TUI transcript notices for every `TurnFinish::Completed`
  branch, including completions without visible assistant text.
- [x] 3.2 Preserve terminal cleanup, speculative-output discard, idle/activity
  transitions, todo reconciliation, usage, canonical events, journals, and
  timeline evidence.
- [x] 3.3 Retain concise visible terminal notices for cancellation, limits,
  needs-input, and failure, with honest elapsed time where available.
- [x] 3.4 Replace duration-notice assertions with live/replay-equivalent tests
  proving successful terminals are silent and non-success terminals remain
  visible at narrow, normal, and wide terminal sizes.

## 4. Validation and Documentation

- [x] 4.1 Update `DESIGN.md`, prompt/context documentation, and the security
  threat model with root-only activation, no-authority semantics, read-once
  behavior, cache identity, and quiet success presentation.
- [x] 4.2 Run `cargo fmt --all --check`, warning-denied workspace/all-feature
  Clippy, workspace/all-feature tests, targeted TUI/prompt/host tests, strict
  spec validation, and diff hygiene without overwriting unrelated user work.
- [x] 4.3 Run deterministic interactive/headless parity and child-inheritance
  scenarios, then record any unavailable hosted or live-provider gates
  explicitly.

## Validation Evidence

- `cargo fmt --all --check`, warning-denied workspace/all-target/all-feature
  Clippy, and `cargo test --workspace --all-features` passed.
- Targeted loader, prompt, cache-identity, terminal/headless parity, direct-child
  inheritance, quiet-terminal render, and live/replay-equivalence scenarios
  passed, including narrow, normal, and wide terminal sizes.
- Strict validation passed for this change and all active changes. The opt-in
  live-provider integration remained skipped because it spends provider quota;
  no network/provider gate is required for these deterministic host and TUI
  behaviors.
