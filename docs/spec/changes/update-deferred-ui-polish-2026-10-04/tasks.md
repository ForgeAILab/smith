---
created_at: 2026-10-04T00:00:00Z
updated_at: 2026-10-04T04:54:05Z
completed_at:
---

Approved 2026-10-03 ("i think you can continue with the road map. as we do
need to get this a good ui ux. and strong backend."); the three items were
deferred from `adopt-claude-code-grammar` (3.3b, 4.5b, 4.6c).

## 1. Child details

- [ ] 1.1 Workspace in the spawn row's words and resumability in words in
  the agent inspector, `/agents`, and their plain renderers.

## 2. Setup previews

- [ ] 2.1 Review and collision previews scroll to their last line with the
  arrow and page keys at 44x16 and 80x24; footer keys stay visible.

## 3. Streaming tables

- [ ] 3.1 A trailing table that is still arriving renders as its header and
  a dim "receiving table" line; it renders in full when a non-table line
  follows or the answer commits. Committed text is unchanged.

## 4. Verification

- [ ] 4.1 Tests for each; `cargo fmt --all -- --check`, strict Clippy,
  workspace tests; fixtures reviewed; `final_checks.py` passes.
