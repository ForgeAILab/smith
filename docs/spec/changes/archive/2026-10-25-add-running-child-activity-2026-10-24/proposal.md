---
created_at: 2026-10-24T12:19:37Z
updated_at: 2026-10-25T07:44:59Z
---

## Why

A parent could not tell what a running child was doing. `agent list` and
`agent wait` reported only state, turns, and tokens, and the wait-expiry turn
said "No partial child output is included". In session 0c40e97f a root
parked for about 29 minutes per child while each child streamed one huge
`edit create` that was then discarded; the root learned this only by paging
`agent transcript`. The TUI already shows per-child activity; the model did
not get it.

## What Changes

- `agent wait` and `agent list` add an `activity` object for a running child:
  `elapsed_ms` since its current task started, `steps_this_task` (committed
  transcript entries after the task's user message), `last_step` (kind, tool
  name and pending/done, or a redacted 160-character tail of committed
  assistant text), `pending_tools`, and `streaming` (attempt number, age, time
  since last output, and text/reasoning character counts of the response still
  in flight, or null). Streaming content is never quoted, because the attempt
  may still be discarded.
- The `delegation.wait-timeout` turn carries the same `activity` per expired
  child and points to `agent transcript` for committed steps.
- The `agent` tool description says the transcript includes a running task's
  finished steps but not a response still streaming, and that a running
  child's status carries activity.
- Smith-only; no runtime change.

## Impact

- Affected specs: `child-agents`.
- Affected code: `smith-runtime` `delegation/progress.rs` (new),
  `delegation/tool.rs`, `delegation/wait_timers.rs`, `delegation/lifecycle.rs`,
  `factory/capabilities.rs`; `docs/configuration.md`,
  `docs/guides/sessions.md`.
- Tool description text changes the prompt prefix, so headless and
  skill-listing fixtures may shift.
