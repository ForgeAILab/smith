---
created_at: 2026-09-21T00:00:00Z
updated_at: 2026-09-22T03:52:10Z
---

## Why

The feature audit reproduces a disconnected starting experience: the first chat
screen is empty, help lands at its tail, command discovery depends on memorized
names, and picker state/recovery guidance disappears behind metadata.

Evidence: [audit and checklist](../../../qa/smith-ux-2026-09-21/report.md).

## What Changes

- Show a small local getting-started guide only while the transcript is empty:
  type a task, `/model` to select a model, `/connect` to add a connection, and
  `/help` for commands. Keep the existing composer and footer.
- Start `/help` at the beginning of its result, with a short starting-work guide
  before the complete existing command/keyboard reference.
- Make Enter execute the highlighted completion, preserving exact commands
  with explicitly supplied arguments and all idle/approval checks. Search
  command descriptions as a fallback after name-prefix matching.
- Keep `current` and `unavailable` states ahead of optional picker metadata.
  Distinguish no search matches from an empty inventory and explain how to
  clear the filter; support Ctrl+U to clear a resource filter. Keep the
  confirmation/cancellation keys visible at 44 columns.
- Keep command completion to five visible rows so it does not replace the
  conversation, following the existing compact-picker design contract.
- Preserve the entered non-secret provider name, endpoint, and model when
  navigating backward in setup so correction does not require retyping them.
- Retest the setup → connection → model → first-prompt path and the full
  feature inventory; record live-service and device-specific gaps honestly.

## Impact

- Affected specs: client-surfaces.
- Affected code: smith-tui command/input/picker/transcript/layout/setup modules.
- Existing provider setup and automatic-limit changes remain intact. No new
  providers, credential formats, network requests, runtime permissions, or
  persistent session schema are introduced.
