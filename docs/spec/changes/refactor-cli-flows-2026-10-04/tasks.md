---
created_at: 2026-10-04T00:00:00Z
updated_at: 2026-10-04T05:54:59Z
completed_at:
---

Approved 2026-10-03 ("i think you can continue with the road map. as we do
need to get this a good ui ux. and strong backend."); the rest of audit item
S8. No visible change.

## 1. Headless fold

- [ ] 1.1 Headless event fold as a value with a per-event method returning
  whether the run continues; accumulators are its fields; `run_with_io`
  sets up, drives it, and builds the result after shutdown. Loop body
  moved verbatim, in order.
- [ ] 1.2 Unit tests drive the fold with recorded events: synthetic usage
  excluded, goal continuation counted, pending child-completion delivery
  keeps the run open, a sequence gap becomes the lifecycle error.

## 2. Picker entries

- [ ] 2.1 `runtime_resources` split into one function per picker; entries,
  order, and text unchanged.

## 3. Verification

- [ ] 3.1 `cargo fmt --all -- --check`, strict Clippy, workspace tests,
  `cargo deny`; every fixture byte-identical; `final_checks.py` passes.
