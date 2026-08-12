---
created_at: 2026-08-04T16:55:00Z
updated_at: 2026-08-04T19:46:09Z
---

## Why

A long shell command today has exactly two outcomes: it finishes inside its
timeout, or Smith kills its process group and returns an error. Session
`761404fc` showed the failure shape — a repo-wide `grep` hit the 120 s default
and died, and while the partial output let the model recover, nothing lets the
model *choose* to keep a legitimately long command (builds, test suites,
package installs) running without blocking the turn. Monitors watch streams;
nothing runs a command to completion off-turn. The timeout error text also
teaches the model nothing about its options.

## What Changes

- Add an explicit `run_in_background` option to the `shell` tool. The call
  returns immediately with a session-scoped task ID and a bounded output-spool
  reference; the process group stays session-owned and its exit produces one
  terminal notification through the existing session inbox.
- Add `task_output` and `task_stop` tools so the model can read a background
  task's spooled output incrementally and stop it by task ID.
- Enrich the foreground timeout outcome: it MUST keep killing the group
  (no automatic conversion to background — explicit non-goal) and MUST state
  the actionable options — raise `timeout_ms` toward the maximum, narrow the
  command, or rerun with `run_in_background`.
- Add a TUI affordance to manually background a running foreground shell call
  (Claude Code's ctrl+b shape): the pending tool call resolves with the output
  so far plus the task ID, and the process continues as a background task.
- Fold background tasks into the existing ephemeral-work family: active-work
  exit policy, resume reconciliation as `interrupted_by_process_exit`,
  operational status display, and metadata-only journal lifecycle markers.

## Impact

- Affected specs: `tool-execution` (ADDED), `client-interaction` (ADDED),
  `agent-session` (MODIFIED), `client-surfaces` (MODIFIED)
- Affected code: `crates/smith-tools/src/shell.rs` (new option, timeout text,
  group adoption seam), new task registry + spool in `crates/smith-runtime`
  (journal markers, inbox notification, reconciliation, exit policy),
  `crates/smith-tools` (`task_output`, `task_stop`), `crates/smith-tui`
  (status display, manual-background key), `crates/smith-cli` (headless exit
  policy coverage)
- No dependency on an `agent-runtime` change is expected; the tools and
  registry live in Smith-owned crates. If group adoption needs a runtime seam,
  that surfaces during design review.
