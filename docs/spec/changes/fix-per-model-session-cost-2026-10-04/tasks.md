---
created_at: 2026-10-04T19:35:32Z
updated_at: 2026-10-04T19:35:32Z
completed_at:
---

From the open question in `docs/qa/live-2026-10-04/findings.md`, confirmed
as a pricing bug in `docs/qa/live-2026-10-04b/findings.md`. Approved 2026-10-04 ("ok carry on").

## 1. Accounting

- [x] 1.1 `Status` keeps root counters per binding with that binding's price;
  `switch_model` closes the current binding and never reprices it; the
  exact/estimated signal is per binding.
- [x] 1.2 Delegated counters per child binding: `tui_driver` resolves the
  child's profile to a binding and catalog price at spawn; unresolved
  children are unpriced.
- [x] 1.3 `SessionCost::compute` sums each binding at its own price;
  unpriced bindings are left out, downgrade the label, and are named.
- [x] 1.4 Synthetic cache counters (keepalive, handoff) are priced by the
  binding active when they were reported, not the last one; found in
  review.

## 2. Surfaces

- [x] 2.1 Exit report and `/status`: total, label, then each binding with
  its share when more than one contributed; single-binding output
  byte-identical to today.
- [x] 2.2 Usage log schema version 5 with per-binding counters; readers
  accept 1–5.

## 3. Verification

- [x] 3.1 Unit tests: two root bindings, a child on another model, an
  unpriced binding, an unresolved child, a version-4 log record; existing
  cost tests unchanged.
- [ ] 3.2 fmt, strict Clippy (including Rust 1.88), workspace tests with
  `--no-fail-fast`, `cargo deny`; fixtures reviewed.
- [x] 3.3 Live: GLM then `/model` to Gemini, one turn each; the exit line
  names both with their shares and the shares match each model's catalog
  rates.

  Live 2026-10-04 on a release build of c5b9026: exit line
  `$0.000875 exact · zai/glm-5.3 $0.000 · google/gemini-3.8-flash $0.000875`
  (the Z.AI Coding Plan prices every counter at 0); usage log v5 lists both
  bindings.
