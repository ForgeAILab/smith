---
created_at: 2026-10-24T12:06:31Z
updated_at: 2026-10-24T13:06:37Z
completed_at: 2026-10-24T13:06:37Z
---

## 1. Implementation

- [x] 1.1 Pin Agent Runtime `8dfd6b8` and handle `ChildTaskOutcome::LimitReached` and `RuntimeEvent::ChildLimitReached` in delegation lifecycle, wait timers, and the client event projection.
- [x] 1.2 Report the limit through the `agent` tool (`wait`, `result`, status `stopped_at_limit`) without any stale result text.
- [x] 1.3 Show a limit-stopped child in the TUI, timeline report, and headless child summary; project it into the resume capsule and ephemeral-work recovery.

## 2. Validation

- [x] 2.1 Cover automatic delivery and `wait` of a limit-stopped child end to end, the TUI row and notice, the capsule projection, the timeline line, and resume of a journal written at event schema 16.
- [x] 2.2 Regenerate headless fixtures (schema 16 → 17 only) and run targeted tests, fmt, and Clippy.
- [x] 2.3 Run the full Smith gate (3,314 passed, 0 failed) and CI (run
  40094931229).
