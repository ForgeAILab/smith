---
created_at: 2026-10-23T09:39:03Z
updated_at: 2026-10-23T11:48:59Z
---

## Why

Root agents use `agent` action `wait` as a blocking sleep instead of parking.
Recent sessions show parents holding one tool call open for 300 s, 300 s, then
four 120 s waits in a row (`session-f3cce6ec`), and 240 s / 300 s / 120 s in
`session-f5dcd569`. During those calls the parent can do nothing else, the user
sees a stalled turn, and the automatic completion delivery that parking already
provides goes unused. The tool description already says "do not poll", but a
five-minute foreground wait is still the most attractive action on offer.

## What Changes

- **BREAKING (model-facing tool behavior):** `agent` action `wait` no longer
  blocks. It returns immediately and arms a per-child wait timer. If the child is
  already terminal, it returns and acknowledges the outcome exactly as today.
- The parent is expected to end its turn and park. A terminal child outcome
  still wakes the parent through the existing `delegation.child-completion`
  continuation and cancels the timer.
- When an armed timer expires while the child is still running, Smith admits
  one idle-only internal continuation attributed `delegation.wait-timeout`,
  carrying a bounded status of the child (no partial output). If the parent is
  busy, the status is delivered at the next safe boundary instead.
- `timeout_ms = 0` keeps its immediate-status meaning and arms nothing.
  Re-arming a child replaces its earlier timer. Timers never re-arm themselves.
- A `timeout_ms` above the configured maximum is clamped to that maximum and
  the effective value is reported, instead of being rejected.
- Armed timers are in-memory only; shutdown cancels them and restart
  reconciliation supersedes them.
- The existing `child_agents.wait_default_timeout_ms` / `wait_max_timeout_ms`
  keys keep their names and ranges and now bound the timer, not a foreground
  call.
- Tool description and delegation guidance are rewritten around "arm, end your
  turn, get woken".
- Positive timers have no minimum. Agent/tool waits retain the five-minute
  maximum; the delegated-work panel shows a `monitor` tag with the wake time.
- The wake is built Smith-side on the session's existing idle-only internal
  turn admission; no Agent Runtime change or pin bump is expected.

## Impact

- Affected specs: `child-agents`, `configuration`, `runtime-integration`,
  `client-surfaces`
- Affected code: `crates/smith-runtime/src/delegation/tool.rs`,
  `crates/smith-runtime/src/delegation_parking.rs`,
  `crates/smith-runtime/src/delegation/lifecycle.rs`,
  `crates/smith-runtime/src/host/` (timer + internal-turn admission),
  `crates/smith-config/src/resolve/` (clamp wording only), delegation tests,
  TUI/headless lifecycle projection
