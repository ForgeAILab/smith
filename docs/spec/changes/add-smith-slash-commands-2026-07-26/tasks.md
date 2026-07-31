---
created_at: 2026-07-27T02:57:00Z
updated_at: 2026-07-28T16:19:23Z
completed_at: 2026-07-28T16:19:23Z
---

## 1. Implementation

- [x] 1.1 Add a command registry (name, one-line description, host action)
  and parse `/`-prefixed composer input in the submit path before any
  provider dispatch.
- [x] 1.2 Wire built-in commands to existing host actions (model picker,
  session controls, quit/help) without duplicating their logic.
- [x] 1.3 Render `/help` output and unknown-command local errors in the
  transcript without provider spend.
- [x] 1.4 Implement and document the literal-slash escape passthrough.
- [x] 1.5 Add tests: known command dispatch, unknown command local error,
  keybinding/command parity, escape passthrough — each asserting no provider
  request on local paths.

## 2. Context visibility

- [x] 2.1 Fold `ContextPlanned` telemetry into bounded TUI status state,
  retaining input usage, enforced budget, reserves, confidence, and segment
  totals without retaining context content.
- [x] 2.2 Render Codex-style context-window usage inside `/status`, including
  an honest pre-plan state and a distinction between the latest request plan
  and cumulative provider-reported session input.
- [x] 2.3 Update the compact footer to show latest-plan context rather than
  presenting cumulative provider input as active context.
- [x] 2.4 Add reducer, rendering, command, resize, and no-provider-spend tests;
  revalidate the change and both focused and workspace checks.

## 3. Focused context command

- [x] 3.1 Register `/context` as a zero-provider-spend local command and route
  it through the existing slash-command reducer.
- [x] 3.2 Render the latest enforced plan as an inline 5×10 usage map and
  category legend, distinguishing segment usage, free input space, reserved
  output/reasoning capacity, provenance, and compaction state.
- [x] 3.3 Render an honest pre-plan state without inventing usage or retaining
  raw context content.
- [x] 3.4 Add parser, discovery, dispatch, output, width, no-color, and
  no-provider-history tests; run focused and workspace validation.
