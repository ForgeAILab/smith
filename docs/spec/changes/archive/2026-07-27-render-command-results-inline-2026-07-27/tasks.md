---
created_at: 2026-07-27T06:38:48Z
updated_at: 2026-07-27T16:28:54Z
completed_at: 2026-07-27T16:28:54Z
---

## 1. Implementation

- [x] 1.1 Update `DESIGN.md` so informational command output is a transcript
  primitive, including title, content, empty/error state, wrapping, scroll
  follow, and non-persistence to provider conversation history.
- [x] 1.2 Add an attributed bounded local-result transcript block and render
  it legibly at narrow, normal, wide, short, and no-color terminal sizes.
- [x] 1.3 Route `/help`, `/status`, `/agent`, and every `/diff` scope to inline
  local-result blocks while keeping composer focus and issuing no provider
  request.
- [x] 1.4 Remove the informational viewer overlay and its close-only input
  state; retain command discovery and all approval/review/recovery
  confirmations.

## 2. Verification

- [x] 2.1 Add reducer tests proving inline results do not create provider
  sends, do not steal composer input, and append rather than replace earlier
  command results.
- [x] 2.2 Add render tests for titled, multiline, empty, error, oversized, and
  no-color local results across narrow, normal, wide, and short terminals.
- [x] 2.3 Add host-loop tests covering `/help`, `/status`, `/agent`, `/diff`,
  non-Git errors, and consecutive informational commands.
- [x] 2.4 Run workspace fmt, warnings-denied Clippy, tests, MSRV/CI gates, and
  real cmux QA for consecutive inline command output plus continued composing.
