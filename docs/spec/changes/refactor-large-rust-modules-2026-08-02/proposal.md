---
created_at: 2026-08-02T16:31:46Z
updated_at: 2026-08-02T17:29:48Z
---

## Why

Five Rust source files now concentrate unrelated responsibilities and large
in-file test suites. At the current baseline, `smith-tui/app.rs` is 6,852
lines, `smith-cli/main.rs` is 4,002 lines, `smith-config/resolve.rs` is 3,452
lines, `smith-tui/render.rs` is 3,944 lines, and
`smith-runtime/factory.rs` is 2,229 lines. This makes behavior-preserving
changes harder to review, increases merge conflicts between active changes,
and hides ownership boundaries already present in the architecture.

## What Changes

- Keep the existing public module paths as facades while extracting cohesive
  private child modules for TUI application state, CLI hosting, configuration
  resolution, and rendering.
- Split tests by responsibility so reducer, input, prompt, rendering,
  resolution, and lifecycle coverage remain close to the behavior they
  protect without dominating production files.
- Preserve all public Rust paths and signatures, configuration precedence,
  runtime event ordering, input ownership, rendering output, redaction, and
  session behavior.
- Refactor the runtime factory's 431-line `build` function into explicit
  preparation and assembly stages before deciding whether a physical module
  split improves ownership further.
- Apply the work in independently compiling slices, beginning with
  `smith-tui::app`, then the CLI and configuration resolver, followed by the
  renderer and runtime factory.

## Impact

- Affected specs: `code-organization`
- Affected code: `crates/smith-tui/src/app.rs`,
  `crates/smith-cli/src/main.rs`, `crates/smith-config/src/resolve.rs`,
  `crates/smith-tui/src/render.rs`, `crates/smith-runtime/src/factory.rs`, and
  new private child modules beneath those source areas
- Compatibility: no product behavior, public API, serialized contract,
  configuration key, command, keyboard binding, or rendered presentation is
  intentionally changed
- Dependencies: no new runtime or development dependency is required

## Existing Change Coordination

- `add-turn-steering-and-input-queue-2026-08-02` is the landed baseline for
  prepared submissions, pending-input reduction, host dispatch, and anchored
  rendering. This refactor moves that behavior without redesigning it.
- `add-persistent-session-goals-2026-08-02` remains authoritative for goal
  composition, local commands, reducer projection, and rendering. Its two
  remaining dependency/release tasks do not authorize semantic edits here.
- `add-file-backed-project-memory-2026-08-02` is an unimplemented proposal
  that expects configuration, factory, CLI, and TUI changes. If approved
  first, this refactor SHALL rebase on its landed structure; if this refactor
  lands first, memory implementation SHALL target the new module boundaries.
- `add-smith-agent-harness-2026-07-23` and
  `integrate-stable-session-harness-2026-07-31` remain authoritative for the
  single factory, runtime composition, reducer equivalence, and surface
  parity contracts.

## Approval Boundary

Approval authorizes only behavior-preserving source and test decomposition in
the five named areas. It does not authorize feature work, public API changes,
new dependencies, configuration changes, altered TUI behavior, or edits to
the separate project-memory proposal.
