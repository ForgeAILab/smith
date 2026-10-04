---
created_at: 2026-10-03T00:00:00Z
updated_at: 2026-10-04T01:36:07Z
completed_at: 2026-10-04T01:36:07Z
---

Approved 2026-10-03 ("i think you can continue with the road map. as we do
need to get this a good ui ux. and strong backend."). Roadmap step 5.

## 1. Cache

- [x] 1.1 Per-block revision in `Transcript`, bumped by one mutation accessor
  that every block change goes through; history replacement resets them.
- [x] 1.2 Render cache keyed by (revision, width, fold state, theme) holding
  each block's wrapped rows; assembly applies the same skips and separators
  as `block_lines` and appends speculative text and the turn summary
  uncached. The child inspector and getting-started screen stay uncached.
- [x] 1.3 Draw only the rows in view; scroll limit and a result's start row
  come from cached row counts (prefix sums).
- [x] 1.4 Transcript scroll offsets and limits are `usize` (approval-box
  scrolling is out of scope).

## 2. Read-only drawing

- [x] 2.1 A pure layout step computes the scroll limit and resolves follow
  and scroll-to-block; the driver applies it, then draws with `&App`.

## 3. Verification

- [x] 3.1 Property test: cached and uncached transcript output are identical
  for a long mixed transcript at 44, 80, and 100 columns, folded and
  expanded, after appends, tool status and preview updates, streaming
  deltas, expand toggles, history replacement, and resizes.
- [x] 3.2 A transcript taller than 65,535 rows scrolls to its first row.
- [x] 3.3 Drawing the same state twice yields identical frames and leaves
  the app unchanged.
- [x] 3.4 Measure frame build time for a long transcript before and after
  (an ignored test or small harness that prints timings); record it in
  `docs/qa/`.
- [x] 3.5 Existing render tests and local-command fixtures unchanged;
  `cargo fmt --all -- --check`, strict Clippy, workspace tests;
  `final_checks.py` passes.
