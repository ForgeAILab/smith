---
created_at: 2026-08-17T00:00:00Z
updated_at: 2026-08-17T00:00:00Z
---

## 0. Coordination and Baseline

- [x] 0.1 Approve this proposal and delta spec.
- [x] 0.2 Confirm Smith's checkpoint store accepts a same-turn `Terminal`
  successor followed by a new accepted turn (pre-existing
  `compare_checkpoint` rules).

## 1. Runtime Reconciliation

- [x] 1.1 Runtime: finalize interrupted turns at admission (agent-runtime
  commit `e3bad15` on `main`, cherry-picked onto the pinned lineage as the
  `smith-interrupted-turn-fix` worktree branch) with regression coverage.
- [x] 1.2 Enable the git-ignored `.cargo/config.toml` patch table for local
  sibling-checkout development.

## 2. Verification

- [x] 2.1 Full Smith workspace builds and passes against the patched runtime
  (1407 tests, via the `smith-interrupted-turn-fix` worktree of the pinned
  runtime lineage).
- [ ] 2.2 Pin the new runtime revision in the root manifest once it is
  upstreamed, and remove the local patch dependency.
