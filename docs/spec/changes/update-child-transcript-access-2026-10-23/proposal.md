---
created_at: 2026-10-23T05:12:51Z
updated_at: 2026-10-23T05:55:57Z
---

## Why
The user sees a child's web-search activity but cannot reliably read its work
as a complete conversation. The main agent also has status/result operations
but no transcript-reading operation, so repeated status checks give it little
evidence with which to understand a slow child.

Session `session-f5dcd569-17e2-4e25-9584-0ce241072440` demonstrated the problem:
the parent repeatedly waited, then used the shell-task stop operation on a
child ID. Its claim that the child was stuck fetching was unsupported; the
child's last checkpoint was `calling_model`, request `req-3`.

The user confirmed that both the user and the main agent should be able to
read the full child transcript.

## What Changes

- Make the selected child view a complete, scrollable conversation using the
  main conversation's rendering rules: delegated task, assistant messages,
  provider-exposed reasoning under existing visibility rules, tool calls,
  tool results, and final reply. Existing root styling and navigation apply.
- Load history before attaching the live tail. Inspection after a late
  subscription or parent restart must recover retained child history without
  starting or resuming the child. Recoverable earlier content must not be
  replaced by a lifecycle summary or silently dropped at a client log limit.
- Add root-only `agent` action `transcript`, addressed by `child_id`, with
  stable pagination. It returns the child's committed conversation in order,
  with roles, call identities, lifecycle state, and a continuation cursor.
  Large entries use readable artifact references under the existing output
  policy so the parent can obtain the complete content.
- Keep routine coordination completion-driven. After delegation, the parent
  continues independent work or parks; it does not repeatedly read status or
  the transcript to obtain an eventual result. Transcript access is for an
  explicit inspection or diagnosis. Completion still arrives through the
  existing protected result-delivery path.
- Explain the distinction between child controls (`agent`) and background
  shell-task controls (`task_output`, `task_stop`) in capability guidance.
- Show observed provider/tool activity and the time since the last observed
  progress without claiming that silence proves thinking or a stuck fetch.

This change does not impose a child lifetime timeout, change cancellation
authority, or claim to repair the underlying Gemini request stall. Full
transcript access must not fabricate text a provider has not exposed.

## Impact

- Specs: `child-agents`, `client-surfaces`.
- Runtime composition: `crates/smith-runtime/src/delegation/tool.rs`,
  `host.rs`, `prompt.rs`, child history/persistence composition.
- Client: `crates/smith-client/src/agent_report.rs`,
  `crates/smith-cli/src/tui_driver.rs`, `tui_driver/events.rs`,
  `local_command/agent.rs`, and `crates/smith-tui/src/app/` conversation state.
- Tests: delegation integration, persistence/recovery, child event folding,
  and terminal transcript fixtures.
- Dependency boundary: Agent Runtime already exposes live
  `with_child_history` and `child_events`. Implementation must establish a
  gap-free history/live boundary and durable read access; any missing shared
  seam belongs in Agent Runtime, followed by an immutable Smith pin update.
- No competing active child-transcript change was found. The active session
  fork proposal must retain its existing running-child restrictions.
