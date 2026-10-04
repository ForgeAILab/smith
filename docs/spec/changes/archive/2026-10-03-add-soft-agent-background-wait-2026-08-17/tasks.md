---
created_at: 2026-08-17T20:54:25Z
updated_at: 2026-08-17T21:08:00Z
completed_at: 2026-08-17T21:08:00Z
---

## 1. Foreground wait policy

- [x] 1.1 Raise the source-resolved child-agent default and maximum foreground
  wait to 300,000 milliseconds while retaining zero as an immediate status
  check.
- [x] 1.2 Keep the pinned Agent Runtime call within its per-call wait ceiling by
  using bounded slices for longer Smith waits.
- [x] 1.3 Mark an expired foreground wait in the structured result without
  changing the child lifecycle or cancellation path.

## 2. Tests and documentation

- [x] 2.1 Add a focused test proving an omitted wait returns a running child
  after the configured foreground boundary and leaves it running.
- [x] 2.2 Update delegation/configuration unit and integration expectations.
- [x] 2.3 Update the child-agent and configuration truth/reference specs and
  complete the change validation.
- [x] 2.4 Run formatting and the proportionate Rust checks, recording any
  dependency/workspace blocker.
