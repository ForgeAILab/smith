---
created_at: 2026-10-02T10:43:19Z
updated_at: 2026-10-02T10:43:19Z
completed_at:
---

## 1. Contract

- [ ] 1.1 Rewrite the reference paragraphs and sections 2 to 6 of
  `DESIGN.md` to the grammar in `design.md`; add the "Kept" table.
- [ ] 1.2 Obtain approval of this proposal and of the `DESIGN.md` rewrite.
- [ ] 1.3 Confirm the glyph set reports width 1 and add a test that rejects
  emoji-capable code points in the glyph table.

## 2. Transcript (after `fix-interaction-defects`)

- [ ] 2.1 Role markers `>`, `●`, `⎿`; hanging indent under text.
- [ ] 2.2 One row per tool call with reviewed label and summary; nested
  bounded result; one-line summaries for tools without a useful preview.
- [ ] 2.3 Echo a user shell shortcut as `! command` with a nested result.
- [ ] 2.4 Remove the change notice for turns that changed nothing.
- [ ] 2.5 `Ctrl+O` expands and folds; `/details` shares the toggle.
- [ ] 2.6 Live-versus-replay parity for every changed row.

## 3. Progress, turn end, Markdown, approvals

- [ ] 3.1 Move the working row above the composer with elapsed time, token
  flow, and the interrupt key; keep retry and backoff wording.
- [ ] 3.2 Attach the turn summary to its turn; hide it when a later block is
  appended.
- [ ] 3.3 One Markdown renderer for streamed and committed text; lists,
  quotes, tables, fences with a language label, links with visible targets.
- [ ] 3.4 Approval layout: action, place and deadline, warning, question,
  choices; detail behind `Ctrl+O`; diffs expandable.
- [ ] 3.5 Keep transcript scrolling available while a prompt is open.

## 4. Menus and informational results (after `refactor-client-structure`)

- [ ] 4.1 Two-column command menu and palette; argument grammar on the
  selected row only; one order everywhere.
- [ ] 4.2 Picker rows: name, short description, state at the right edge;
  detail line for the selected row.
- [ ] 4.3 `/help`, `/status`, `/context`: aligned columns, word wrap,
  open at the top.
- [ ] 4.4 `/diagnostics`: grouped, one fact per line, `unknown` for
  unknown.
- [ ] 4.5 First-run setup: one frame, wrapped descriptions.

## 5. Composer (after the in-flight TUI changes are committed)

- [ ] 5.1 Rules above and below; `>` prompt; bash-mode prompt for `!`.
- [ ] 5.2 Line movement inside a multi-line draft; history only from the
  first or last line.
- [ ] 5.3 Line-editing keys; Home and End act on a non-empty draft.
- [ ] 5.4 Hint row: `? for shortcuts` when idle; hints dropped last when
  narrow.
- [ ] 5.5 Shortcuts panel on `?` in the anchored pane.

## 6. Verification

- [ ] 6.1 PTY captures at 100×32, 80×24, 44×16, and no-colour for every
  surface, stored under `docs/qa/`.
- [ ] 6.2 `cargo fmt --all -- --check`, strict Clippy, workspace tests.
- [ ] 6.3 Command sweep and startup sweep.
