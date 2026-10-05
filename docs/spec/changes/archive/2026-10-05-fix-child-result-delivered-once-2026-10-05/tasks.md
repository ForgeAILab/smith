---
created_at: 2026-10-05T05:00:00Z
updated_at: 2026-10-05T07:20:00Z
completed_at: 2026-10-05T07:20:00Z
---

Approved 2026-10-05 ("we need to fix the double delivery", "do both and
delivery").

## 1. Implementation

- [x] 1.1 Agent Runtime pin moves to `fc92efb` (`smith-baseline-v0.3.8`).
- [x] 1.2 The agent tool acknowledges the outcome returned by `wait` and
  `result` against its tool call.
- [x] 1.3 Headless exit tracks the delivery cursor revision and live
  coordinator state instead of a pending flag set by every child completion.
- [x] 1.4 The host-restart delegation test waits for the delivery turn before
  shutting down.

## 2. Verification

- [x] 2.1 `smith-runtime` tests: a result read with `wait` or `result` is
  answered once (they fail without 1.2: 4 vs 3 and 5 vs 4 parent calls).
- [x] 2.2 `smith-cli` test: a headless run that read its child result exits
  (times out with the old fold).
- [x] 2.3 Live on `zai/glm-5.3`: 0.3.7 answered `pong` twice in two turns;
  0.3.8 answers once in one turn. A child the model does not wait for is
  still delivered automatically.
