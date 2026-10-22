---
created_at: 2026-10-21T20:59:02Z
updated_at: 2026-10-21T21:47:56Z
completed_at: 2026-10-21T21:47:56Z
---

## 1. Shared command catalog

- [x] 1.1 Add a typed skill invocation and descriptor-only skill menu rows to
  parsing, slash completion, command palette, and help.
- [x] 1.2 Preserve built-in and file-command precedence; add
  `/skills use <name> [task]` as the collision-safe route.
- [x] 1.3 Test exact names, prefixes, namespaced plugin skills, argument
  preservation, unknown names, collisions, and Tab without execution.

## 2. Host activation

- [x] 2.1 Resolve an explicit invocation against the session's winning skill
  descriptor and authorized capability view, with current trust and content
  identity checks.
- [x] 2.2 Add explicit skill pins to the existing factory/session rebind path.
  Complete materialization and normal budget admission before swapping the
  session; preserve its identity, history, approvals, and workspace policy.
- [x] 2.3 Handle already-active skills idempotently and reject unavailable
  skills by name without changing the running session.
- [x] 2.4 Preserve the normal activated-skill lifecycle and checkpoint rules;
  test resume and reconfiguration under changed content or narrowed scope.
- [x] 2.5 Disable direct activation on installed CLI backends with guidance to
  their existing skill bridge. Test that no false activation notice is emitted.

## 3. Interactive invocation

- [x] 3.1 Populate skill rows from resolved discovery metadata, including
  descriptions, source layers, collision guidance, and unavailable reasons.
- [x] 3.2 Route skill-only Enter to local activation, and task-bearing Enter to
  activation followed by ordinary prepared submission. Preserve typed history,
  task text, references, and attachments.
- [x] 3.3 Reject activation during a busy turn without reading a body or
  changing the in-flight epoch; keep completion available while busy.
- [x] 3.4 Verify the existing terminal layout and keyboard behavior with
  narrow/wide snapshots, disabled/empty states, Enter, Tab, and Escape.

## 4. Acceptance and documentation

- [x] 4.1 Prove with a fake provider that a chosen skill's instructions reach
  the first task request and selection alone makes zero provider calls.
- [x] 4.2 Cover duplicate layers, already-active selection, withheld trust,
  edited files, denied capabilities, over-budget instructions, and collision
  fallback; verify failure leaves the current session usable.
- [x] 4.3 Update skill and slash-command documentation with both invocation
  forms, activation lifetime, collision rules, and backend availability.
- [x] 4.4 Run formatting, Clippy for affected crates, and focused runtime,
  client, CLI, TUI, and snapshot tests; validate the completed spec change.
- [x] 4.5 Refresh the changed help recordings and the headless recordings
  still carrying the previous retry defaults. Normalize positive jitter delays
  as durations while retaining zero-delay and no-retry distinctions.

## Verification

- PASS: `cargo test -p smith-client -p smith-tui --lib --locked --offline`
  completed with 157 client tests and 710 TUI tests passing.
- PASS: `cargo test -p smith-cli --bin smith --locked --offline` completed
  with 412 tests passing and one existing ignored test.
- PASS: `cargo test -p smith-runtime --lib --test composition --locked
  --offline` completed with 673 unit tests and 64 composition tests passing;
  two existing unit tests are ignored. Total across these suites: 2,016 passed.
- PASS: Clippy with `--all-targets -- -D warnings` for `smith-runtime`,
  `smith-client`, `smith-tui`, and `smith-cli`; `cargo fmt --all -- --check`.
- PASS: production-loop tests exercise activation without provider spend,
  rebind onto the same session, immediate task dispatch, already-active
  selection, and file-reference materialization. Runtime tests cover resumed
  activation, scope narrowing, content edits, trust, and instruction budgets.
- PASS: terminal rendering at 40x12, 80x24, and 120x32 covers normal,
  unavailable, and empty completion states. Keyboard tests cover Tab without
  execution, Enter, Escape, and busy-turn draft preservation. Existing native
  primitives and theme tokens are reused; web/React/browser gates do not apply.
- PASS: strict spec validation. The `rust-review` scenario example avoids
  colliding with Smith's existing built-in `/review` command.
