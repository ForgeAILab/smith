---
created_at: 2026-10-03T00:00:00Z
updated_at: 2026-10-04T04:52:45Z
completed_at:
---

Approved 2026-10-03 ("i think you can continue with the road map. as we do
need to get this a good ui ux. and strong backend."). Roadmap step 6; the
remove-or-finish decision is recorded in proposal.md.

## 1. Protocol

- [x] 1.1 Remove `SmithInput`, `TurnReceipt`, `SteerReceipt`,
  `SteerRejection`, and `SmithSession::{submit, steer, cancel}`; keep
  `SmithSession` as the event adapter (`id`, `events`).
- [x] 1.2 Remove the deprecation on the `SessionHandle` re-export and
  rewrite `docs/architecture.md`'s protocol paragraph to what is true.
- [x] 1.3 Mirror `BudgetFailure` and `RateLimitObservation` in
  `SmithEventKind` with their projections; the TUI shows a budget failure as
  a transcript notice; headless streams both as typed events.
- [x] 1.4 A test matches every runtime `RuntimeEvent` variant exhaustively
  and asserts each projects to a known `SmithEventKind`.

## 2. Boundaries

- [x] 2.1 Make `smith-runtime` modules with no consumer outside the crate
  `pub(crate)` (consumer table with evidence first) and deny
  `unreachable_pub` for the crate.
- [x] 2.2 Replace the name-based architecture tests with structural ones
  (see the code-organization delta) and correct the module docs S11 names.

## 3. Verification

- [x] 3.1 `cargo fmt --all -- --check`, strict Clippy, workspace tests;
  local-command fixtures and headless fixtures unchanged except the two new
  typed event kinds where a fixture contains them.
- [x] 3.2 `final_checks.py` passes; release cache comparison against 0.3.3.
