---
created_at: 2026-10-04T19:35:32Z
updated_at: 2026-10-04T19:35:32Z
completed_at:
---

Findings R1–R4, S1–S6, C1, C2, M1 from
`docs/qa/live-2026-10-04b/findings.md`; layout chosen by the owner
2026-10-04 ("Inline list"). Approved 2026-10-04 ("ok carry on").

## 1. Fixtures first

- [x] 1.1 Terminal fixtures for `smith --resume` (two sessions, none), first-run
  setup (first step, credential methods, key field, review), ChatGPT login
  method, ChatGPT account choice, and login progress at 44x16 and 100x32,
  recorded from the current build.

## 2. One runner

- [x] 2.1 A screen value in `smith-tui` (`draw`, `on_event` → outcome or
  effect, optional tick); `ResourcePicker`, `SetupApp`, and login progress
  implement it.
- [x] 2.2 One `run_screen` in `smith-cli` owning terminal, events, ticks,
  theme, and an optional raced future; `choose_resume_session`, `pick_one`,
  `choose_login_method`, `wait_for_login_surface`, and setup move onto it.
  The section 1 fixtures stay byte-identical.
- [x] 2.3 Structure test: only the runner and the session loop create an
  `EventStream`.

## 3. One chooser component

- [x] 3.1 Inline list in `smith-tui`: title, rows, footer, no frame; label
  column over the whole list; `n/total` when scrolling; numbered fixed lists
  with digit choice; filtered inventories; one footer vocabulary.
- [x] 3.2 Standalone screens draw from the top-left, sized to content; the
  centered boxes and `standalone_picker_area` go.
- [x] 3.3 In-session pickers use the same component; `/model` opens on the
  current model; compact detail line (M1); `/connect` custom endpoint
  description (C2).

## 4. Flows

- [x] 4.1 Esc goes back one step, cancels on the first; Ctrl+C cancels the
  flow; ChatGPT login is a setup step when reached from setup (S5).
- [x] 4.2 Setup wording: welcome line, real credential entry names, no
  untypeable commands or internal terms, plain review with compact sizes and
  `~` paths, `Setup cancelled · nothing was written` (S1–S4, S6).
- [ ] 4.3 `/connect` runs inside the session: an embedded `ScreenSession`
  draws the retained `App` with the connection screen above the composer
  (design revised 2026-10-04, see design.md), rebuild that keeps the screen,
  results as notices instead of `println!` (C1).
- [ ] 4.4 xAI device login gets a progress screen like ChatGPT's (code, URL,
  waiting, esc cancels) instead of `println!` lines, so it shows inside a
  session and standalone.
- [ ] 4.5 Screens repaint whole on a step change (step key on `Screen`,
  repaint in `ScreenSession`, standalone and embedded); the PTY waits go back
  to the phrases users see. Found in review.

## 5. Sessions

- [x] 5.1 Session rows: latest prompt, relative age, `1 turn`, model; id only
  in the selected detail (R3).
- [x] 5.2 Sessions without a user message: omitted from both pickers and the
  terminal table of `smith sessions list`; no `resume with …` line on exit;
  piped output unchanged (R4).
- [x] 5.3 `smith --resume` empty state says `esc exits` (R2); every session
  reachable by scrolling (R1).

## 6. Verification

- [ ] 6.1 Unit tests for each screen value and the chooser component;
  fixtures re-recorded and reviewed; fmt, strict Clippy (including the Rust
  1.88 toolchain), workspace tests with `--no-fail-fast`, `cargo deny`.
- [x] 6.2 `DESIGN.md` updated: choosers are inline lists; centered modals
  remain only for approvals and confirmations.
- [ ] 6.3 Repeat the live pass's steps on the release build, using the
  environment-variable credential method so review can be confirmed without
  touching the Keychain.
