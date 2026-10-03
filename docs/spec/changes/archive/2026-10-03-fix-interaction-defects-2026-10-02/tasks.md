---
created_at: 2026-10-02T10:20:00Z
updated_at: 2026-10-03T09:35:55Z
completed_at: 2026-10-03T09:35:55Z
---

## 1. Preconditions

- [x] 1.1 Obtain approval of this proposal and an answer to the approval-guard
  question (owner, 2026-10-02: approved; 500 ms quiet-window guard).
- [x] 1.2 Confirm `show-provider-retry-progress` and
  `show-idle-compaction-summary` are committed before editing `app/state.rs`.
  Resolved 2026-10-02 by committing `show-provider-retry-progress` together
  with this change, since they share six files; `show-idle-compaction-summary`
  is a proposal with no code, so there is nothing of it to commit first.
- [x] 1.3 Update `DESIGN.md` (setup, approvals, pickers, shell shortcut)
  before UI edits.

## 2. Setup

- [x] 2.1 Add working first-run handlers for the Anthropic and ChatGPT setup
  entries.
- [x] 2.2 Add a test that every id returned for the action step is handled.
- [x] 2.3 Derive the quick-start label and the review's catalog revision from
  the values being written.

## 3. Prompts and pickers

- [x] 3.1 Add the keystroke guard to approval, rotation, trust, and recovery
  prompts; keep the draft; cover the typing-through case with a reducer test.
- [x] 3.2 Clear the command text when a command-opened picker is cancelled.
- [x] 3.3 Give redo, MCP trust, and skill trust their own confirmation copy
  and hint row; add one render test per dialog.
- [x] 3.4 Preserve line breaks of prepared-action text in the approval.
- [x] 3.5 Run a user-typed `!command` without an approval prompt: authorize
  exactly the submitted prepared call once, grant nothing further, and leave
  model-requested shell calls on the resolved policy.

## 4. Command line

- [x] 4.1 Accept `help`, `-h`, and `--help` at every subcommand level.
- [x] 4.2 Print one recovery hint per parse error.
- [x] 4.3 Check for a terminal before entering the alternate screen on the
  configured interactive path.
- [x] 4.4 Resolve the headless exit policy as flag, then configuration, then
  default; add a contract test for the configured value.
- [x] 4.5 Render `sessions list` for a terminal; keep tab-separated output
  when stdout is not a terminal.

## 5. Verification

- [x] 5.1 PTY walkthrough: first-run setup through each listed entry; type
  through an arriving approval; `/model`, Esc, next command.
  Setup and picker with the fake provider; the approval guard live against
  `zai/glm-5.3` with a model-requested shell call
  ([walkthrough](../../../qa/smith-structure-2026-10-02/live-walkthrough-real-provider.md)).
  Not walked: a real ChatGPT OAuth sign-in from setup, and the exact 500 ms
  boundary (tests only).
- [x] 5.2 `cargo fmt --all -- --check`, strict Clippy, workspace tests.
  All pass: 1,726 passed, 0 failed, 6 ignored with `TMPDIR` on the internal
  disk. `TMPDIR` on `/Volumes/Data/tmp` reproduces the timeouts at the same
  load; see the results file.
- [x] 5.3 Re-run the 26-check command sweep and 40-check startup sweep.
  Rebuilt the drivers as `docs/qa/smith-structure-2026-10-02/sweeps/run_sweeps.py`;
  26/26 and 40/40 pass
  ([results](../../../qa/smith-structure-2026-10-02/fix-interaction-defects-results.md#sweeps-re-run)).
- [x] 5.4 Record results and remaining gaps in the audit folder
  ([results](../../../qa/smith-structure-2026-10-02/fix-interaction-defects-results.md)).
