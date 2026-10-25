---
created_at: 2026-10-24T12:19:37Z
updated_at: 2026-10-25T07:44:59Z
completed_at: 2026-10-25T07:44:59Z
---

## 1. Implementation

- [x] 1.1 Compute a running child's activity from committed history, retained
  backend observations, and pending presentation events, redacted with the
  root's credential policy (`delegation/progress.rs`).
- [x] 1.2 Report `activity` from `agent wait` and `agent list` for running
  children only.
- [x] 1.3 Include `activity` in the wait-expiry turn without awaiting inside
  admission.
- [x] 1.4 Update the `agent` tool description and the user docs.

## 2. Verification

- [x] 2.1 Unit tests: current-task step counting, tool pairing, redacted text
  tail, latest-attempt streaming sizes, backend observations.
- [x] 2.2 Delegation tests: running status with committed tool steps and a
  streaming draft, idle and limit-stopped status without activity, expiry
  turn with activity and no streamed text.
- [x] 2.3 Full workspace gate and CI after rebasing onto the fallback and
  child-limit change: the v0.8.0 release gate (3,320 passed, 0 failed) and
  CI run 40172348400.
