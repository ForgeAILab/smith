---
created_at: 2026-10-24T12:06:31Z
updated_at: 2026-10-24T13:06:37Z
---

## Why

A child task that stopped at a limit (most often an output limit, where the
runtime discards the over-long response) was reported to the parent as a
completed task carrying the child's last earlier message. In session
0c40e97f a parent received a stale preamble as three children's "results",
with no sign that any of them had failed, and about 29 minutes of child work
was lost each time.

## What Changes

- Pin Agent Runtime `8dfd6b8` (0.7 line), which records a limit-stopped
  child task as `ChildTaskOutcome::LimitReached { limit, artifacts }` and
  emits `RuntimeEvent::ChildLimitReached` (event schema 17). The child stays
  idle and accepts follow-ups; its status carries no result and a `Limit`
  error naming the limit. Completion delivery names the limit.
- The `agent` tool's `wait` and `result` report a limit outcome with
  `stopped_at_limit`, a null `result`, the stopped turn's artifacts, and a
  note on how to continue; `list`/`wait` status adds `stopped_at_limit`.
- The TUI marks the child row "stopped at its <limit>", in warning tone, not
  running and not auto-dismissed, with a root-transcript notice.
- Headless child summaries add an optional `stopped_at_limit`; the timeline
  report, resume capsule, wait timers, and ephemeral-work recovery treat the
  event as a non-terminal task outcome.
- Older journals (event schema 16) still resume; no checkpoint constant
  changes.

## Impact

- Affected specs: child-agents
- Affected code: `crates/smith-runtime/src/delegation*`,
  `crates/smith-runtime/src/client.rs`, `crates/smith-runtime/src/cache_controller/`,
  `crates/smith-runtime/src/resume_capsule/`, `crates/smith-runtime/src/host/recovery.rs`,
  `crates/smith-client/src/{agent_report,timeline_report}.rs`,
  `crates/smith-cli/src/local_command/timeline.rs`, `crates/smith-tui/src/app/`,
  headless fixtures (event schema 17).
