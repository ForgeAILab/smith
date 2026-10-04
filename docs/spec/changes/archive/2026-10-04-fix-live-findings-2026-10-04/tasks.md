---
created_at: 2026-10-04T00:00:00Z
updated_at: 2026-10-04T07:46:03Z
completed_at: 2026-10-04T07:46:03Z
---

Approved 2026-10-03 ("i think you can continue with the road map. as we do
need to get this a good ui ux. and strong backend."); findings L1–L9 from
`docs/qa/live-2026-10-04/findings.md`.

## 1. Input

- [x] 1.1 The slash command menu passes keys it does not use to the composer
  and refreshes its matches; key-table cases for line editing in a slash
  draft (L1).

## 2. Agents

- [x] 2.1 `/agent` heading, labelled counts, compact tokens; agents panel row
  without the session id (L3, L4).
- [x] 2.2 Inspector states each fact once, renders the result as Markdown,
  offers `/agent resume` only with an exact checkpoint (L2).
- [x] 2.3 Spawn row result preview in words (L8).

## 3. Transcript

- [x] 3.1 Capability activation notices only in expanded detail and
  `/diagnostics` (L5).

## 4. Recovery and approvals

- [x] 4.1 Project-relative paths in approval, undo, redo, revert (L6).
- [x] 4.2 Hunked line diffs with three lines of context for undo, redo,
  revert; preview and journaled fingerprint use the same text (L7).
- [x] 4.3 Wording and padding: `1 unchanged line`, the `a` choice, undo box
  padding, undo-after-resume message, compact `/model` context sizes (L8).

- [x] 4.4 Child-agent approval in the spawn row's words; limits only when
  set; no internal names, sentinels, or target ids (L9).

## 5. Verification

- [x] 5.1 Tests for each; fmt, strict Clippy, workspace tests, `cargo deny`;
  re-recorded fixtures reviewed; `final_checks.py` passes; a repeat of the
  live pass's L1, L2, L5, L6, L7 steps on the release build.
