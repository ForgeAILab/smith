---
created_at: 2026-10-04T00:00:00Z
updated_at: 2026-10-04T06:18:47Z
---

## Why

Two of the four large `smith-cli` functions named in audit item S8
(`docs/qa/smith-structure-2026-10-02/report.md`) remain. (`run_tui` was
split in `refactor-keymap-and-loop`; `handle_local_command` is now 41
lines.)

- **`headless::run_flow::run_with_io`, 477 lines.** It sets up a headless
  run, folds every runtime event into 17 local accumulators (usage, cache,
  sequence, goal and child-completion turns, pending interactions,
  lifecycle, errors) inside one 230-line `while` loop, then shuts the host
  down and builds the result envelope. The fold decides what a headless
  user sees as the turn's usage, cache, status, and exit code, but it can
  only be tested by running a whole host.
- **`resources::runtime_resources`, 511 lines.** It builds every picker's
  entries — models, providers, connections, profiles, child agents, main
  agents, sessions, files, thinking, effort, CLI-agent models — in one
  function.

## What Changes

- The headless event fold becomes a value with a method that takes one
  runtime event and returns whether the run continues. The accumulators
  are its fields; the result envelope is built from it after shutdown.
  `run_with_io` sets up, drives the fold, and finishes. Each `if`/`match`
  in the loop moves verbatim, in order.
- Unit tests feed recorded event sequences to the fold, without a host:
  usage from synthetic records is excluded, a goal continuation turn is
  counted, a child completion delivered after the turn keeps the run open
  until it is delivered, an out-of-order sequence becomes the lifecycle
  error.
- `runtime_resources` becomes one function per picker; the entries, their
  order, and their text are unchanged.
- No visible change: headless stream and result output, `/model` and other
  pickers, and every fixture are byte-identical.

## Impact

- Affected specs: code-organization.
- Affected code: `smith-cli` `headless/run_flow.rs` (and a new module for
  the fold), `resources.rs`.
- The five separate terminal prompt loops in `smith-cli` (setup, ChatGPT
  login, resource pickers) stay for a later change.
