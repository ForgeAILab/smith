---
created_at: 2026-10-02T10:43:19Z
updated_at: 2026-10-02T10:43:19Z
completed_at:
---

## 1. Contract

- [x] 1.1 Rewrite the reference paragraphs and sections 2 to 6 of
  `DESIGN.md` to the grammar in `design.md`; add the "Kept" table.
- [x] 1.1a Reconcile `DESIGN.md` sections outside 2 to 6 that still state
  the old grammar: `?` unknown values, older tool examples and
  `details unavailable` fallbacks, `•` notices and spawn rows, the
  dot-separated retry row, and the conflicting todo-retirement rules.
- [ ] 1.2 Obtain approval of this proposal and of the `DESIGN.md` rewrite.
  Proposal approved 2026-10-03 ("can we use codex to start the ui ux
  polish"); the `DESIGN.md` rewrite awaits review.
- [x] 1.3 Confirm the glyph set reports width 1 and add a test that rejects
  emoji-capable code points in the glyph table.

## 2. Transcript (after `fix-interaction-defects`)

- [x] 2.1 Role markers `>`, `●`, `⎿`; hanging indent under text.
- [x] 2.2 One row per tool call with reviewed label and summary; nested
  bounded result; one-line summaries for tools without a useful preview.
- [x] 2.3 Echo a user shell shortcut as `! command` with a nested result.
- [x] 2.4 Remove the change notice for turns that changed nothing.
- [x] 2.5 `Ctrl+O` expands and folds; `/details` shares the toggle.
- [x] 2.6 Live-versus-replay parity for every changed row. History and
  journal replay match live rendering at 44, 80, and 100 columns, folded,
  expanded, and monochrome. User `!` shortcut echoes and spawn enrichment are
  process-local and were never saved, so a resumed session does not show
  them; `DESIGN.md` now says so.
- [ ] 2.7 Decide whether user `!` shortcut echoes should be saved so a resumed
  session shows them (a durability change, outside this proposal).

## 3. Progress, turn end, Markdown, approvals

- [x] 3.1 Move the working row above the composer with elapsed time, token
  flow, and the interrupt key; keep retry and backoff wording.
- [x] 3.2 Attach the turn summary to its turn; hide it when a later block is
  appended.
- [x] 3.3 One Markdown renderer for streamed and committed text; lists,
  quotes, tables, fences with a language label, links with visible targets.
- [ ] 3.3a Follow-ups from 3.3: keep table columns stable while a table is
  still streaming (later wider cells reflow earlier rows today); emit OSC 8
  links once terminal support can be detected; decide whether code blocks
  drop the visible ``` fence lines and keep only the dim language label.
- [x] 3.4 Approval layout: action, place and deadline, warning, question,
  choices; detail behind `Ctrl+O`; diffs expandable.
- [x] 3.5 Keep transcript scrolling available while a prompt is open.
- [x] 3.5a Approval polish from the live check
  (docs/qa/grammar-2026-10-03/approval-live.md): inner padding inside the
  box; `deadline no deadline` wording; the tool row reads `running` while it
  waits for approval; a denied call reads `failed` instead of `denied`, and a
  second `approval · shell denied` row repeats it.

## 4. Menus and informational results (after `refactor-client-structure`)

- [x] 4.1 Two-column command menu and palette; argument grammar on the
  selected row only; one order everywhere.
- [x] 4.2 Picker rows: name, short description, state at the right edge;
  detail line for the selected row.
- [ ] 4.2a Picker and menu polish from the live check: the selected-row
  detail line repeats text already in the row (account usage, profile
  description); `/goal`'s argument hint spaces its `|` separators unlike every
  other command; the startup guide hardcodes three command suggestions
  instead of reading the command table.
- [x] 4.3 `/help`, `/status`, `/context`: aligned columns, word wrap,
  open at the top.
- [ ] 4.3a Flaky test seen once under full-suite load:
  `a_cleanly_finished_row_retires_itself_but_the_child_stays_known` takes
  `Instant::now()` after the app records the child's finish time, so a slow
  scheduler makes `due - 1ms` reach the expiry. Pass the finish instant in
  instead of reading the clock twice.
- [ ] 4.4 `/diagnostics`: grouped, one fact per line, `unknown` for
  unknown.
- [x] 4.5 First-run setup: one frame, wrapped descriptions.

- [ ] 4.5a Setup polish: indent each description under its name (today both
  start at the same column, so without colour names and descriptions look
  alike); long review and collision-preview bodies still clip.
- [ ] 4.6 Settle two presentation rules `refactor-client-structure` kept
  for byte identity: (a) the inline-code colon exception in the single
  free-text renderer (`smith-tui` `render/transcript.rs`): decide whether
  `DiagnosticsRow::Field` values render verbatim and free lines get
  unconditional inline Markdown, then re-record `skills-populated` and the
  diagnostics fixtures; (b) child-state wording, which differs by surface
  for one state (`Running` local, `running` headless, `working`
  submission) and in durability capitalization.

## 5. Composer (after the in-flight TUI changes are committed)

- [x] 5.1 Rules above and below; `>` prompt; bash-mode prompt for `!`.
- [x] 5.2 Line movement inside a multi-line draft; history only from the
  first or last line.
- [x] 5.3 Line-editing keys; Home and End act on a non-empty draft.
- [x] 5.4 Hint row: `? for shortcuts` when idle; hints dropped last when
  narrow.
- [x] 5.5 Shortcuts panel on `?` in the anchored pane.

- [x] 5.5a Key table content, shared by /help and the shortcuts panel:
  capitalize every description; `?` shows shortcuts, not help; Home/End
  describe the draft-first behaviour; list Up/Down line movement, `\` then
  Enter, Ctrl+A/E/W/U/K, and Alt+B/F.

## 6. Verification

- [ ] 6.1 PTY captures at 100×32, 80×24, 44×16, and no-colour for every
  surface, stored under `docs/qa/`.
- [ ] 6.2 `cargo fmt --all -- --check`, strict Clippy, workspace tests.
- [ ] 6.3 Command sweep and startup sweep.
