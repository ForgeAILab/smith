---
created_at: 2026-10-04T19:35:32Z
updated_at: 2026-10-05T01:08:47Z
---

## Why

The live pass on 0.3.6's screens outside the main view
(`docs/qa/live-2026-10-04b/findings.md`) found that setup, `smith --resume`,
ChatGPT login, and `/connect` look and behave like three different programs:
an inline list inside a session, a centered box sized by entry count, and a
fixed 88x30 setup box, each with its own footer wording and Esc behaviour.
Five separate terminal loops (`choose_resume_session`, `pick_one`,
`choose_login_method`, `wait_for_login_surface`, setup) are the cause, and the
rest of audit item S5. The pass also found screens that hide entries (R1),
contradict themselves (R2), show internal text (S1, S2, S4, S6), and lose the
session screen during `/connect` (C1).

The owner chose Claude Code's presentation for all of them (2026-10-04,
"maybe not like modal just top left option choosing"; picked "Inline list"):
no frame, a title line, a left-aligned list from the top-left, one footer.

## What Changes

- **One list component.** Every chooser (setup steps, `smith --resume`,
  ChatGPT login method and account, `/connect`, `/resume`, `/model`,
  `/provider`, `/profile`) renders through one inline list: a title line, then
  rows, then one footer, with no frame. Short fixed choice lists are numbered
  and accept `1`–`9`; searchable inventories filter on typing and are not
  numbered. One footer vocabulary everywhere, lowercase like the session's
  hint row (`esc to interrupt`):
  `↑↓ choose · enter confirm · esc back` (or `esc cancel` on a first step).
- **Standalone screens start at the top-left.** Before a session exists,
  setup, `smith --resume`, and ChatGPT login draw from the top-left of the
  screen, sized to their content; nothing is centered or boxed.
- **One terminal loop for standalone screens.** A screen is a value that
  draws itself and turns an input event into an outcome; one runner owns the
  terminal, events, ticks, and theme for all of them. The five loops and the
  five copies of the theme-from-flags block go away.
- **`/connect` stays in the session.** The connection steps (credential
  method, key field, review, ChatGPT/xAI login progress, account choice)
  render inline above the composer as the same screens, and the session's
  transcript stays on screen. Only the host rebuild after a completed
  connection happens off-screen, as `/model` does today. Messages a
  connection prints today while the screen is suspended become notices.
- **Session rows read as sessions.** `smith --resume` and `/resume` rows lead
  with the latest prompt, then `2 min ago · 1 turn · zai/glm-5.3`; the session
  id appears only on the selected row's detail. Sessions with no user message
  are not offered by either picker or the terminal table of
  `smith sessions list` (the piped form is unchanged), and the exit report
  omits the `resume with …` line for them. This reverses the stated reason in
  `report_session_usage` ("an empty session is exactly the one a user is most
  likely to want to pick back up"): a session whose prompt failed before any
  spend keeps the line, because it holds a user message. Every session fits: the
  list scrolls with a position count instead of hiding rows (R1, R3, R4).
- **Esc means one thing.** In a multi-step screen Esc goes back one step and
  cancels on the first step; ChatGPT login reached from setup is a setup
  step, so Esc returns to setup. Cancelling setup prints
  `Setup cancelled · nothing was written`. Cancelling `smith --resume` exits,
  and its empty state says so: `No sessions to resume in this project · Esc
  exits` (R2, S5).
- **Setup speaks plainly.** The first step opens with `Welcome to Smith ·
  choose how to connect a model` and states that nothing is sent until setup
  completes. Descriptions name real values (`keychain:smith/zai`, not
  `<provider>`), name no command that cannot be typed there, and drop
  internal terms (`PKCE`, `auth.json`, `public API boundary`). Review is a
  labelled list with compact sizes (`1M context`, `131k output`), the
  destination as `~/.smith/config.toml`, and `Writes … then checks the
  configuration` instead of `pending action` (S1–S4, S6).
- **Picker rows.** `/model` opens on the current model; its detail line
  reads `1M context · 131k output · trusted catalog r5` with compact numbers;
  `/connect` gives the custom endpoint a description of its own; the label
  column width is computed over the whole list so scrolling does not reflow
  rows (C2, M1).

## Impact

- Affected specs: client-surfaces, code-organization.
- Affected code: `smith-tui` `picker.rs`, `setup.rs`, `render/layout.rs`,
  `app/resources.rs`; `smith-cli` `resources.rs`, `setup.rs`, `chatgpt.rs`,
  `xai.rs`, `connection.rs`, `runtime_host.rs` (`InteractiveExit::Connect`),
  `tui_driver.rs`, `terminal.rs`; `smith-client` session listing.
- Terminal fixtures for the five standalone screens are recorded before the
  loops are merged, so the merge is checked against unchanged output; they
  are then re-recorded for the new presentation and reviewed.
- `DESIGN.md`'s rule that modals are centered over the transcript stops
  covering choosers; approvals and confirmations are unchanged.
- No configuration, session, or credential format changes.
