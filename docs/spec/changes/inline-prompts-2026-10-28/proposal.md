---
created_at: 2026-10-28T00:00:00-04:00
updated_at: 2026-10-28T00:00:00-04:00
---

# Proposal: Inline prompts share one numbered select

## Why

Choosers already render as inline lists above the composer, but tool
approvals and confirmations still drew centered bordered modals over the
transcript, and every interactive surface used a different select shape:
numbered `❯ 1. Label` rows (questionnaire, setup), `y  Yes` key rows
(approvals), and a one-line `y … n/esc …` footer (confirmations). The owner
asked to drop modals entirely and make the selects look the same.

## What Changes

- Approvals and confirmations render in the anchored pane, in place of the
  composer while open (draft kept), like the questionnaire: indented title, indented evidence, numbered choices.
  No frame, no centering, no 72-column / 60%-height rules.
- Approvals and confirmations become numbered selects: Up/Down highlight,
  Enter answers, digits answer directly; `y`/`a`/`n`/Esc keep working.
  Shift+↑↓ scrolls long evidence; PgUp/PgDn keep scrolling the transcript.
- Approval highlight starts on `Yes` (owner's choice, matching Claude Code);
  the existing quiet window still swallows keys typed before the prompt
  appeared. Confirmations start on cancel, so `/undo`, `/revert` and trust
  prompts keep having no Enter-applies default.
- A prompt may take the transcript and composer rows; a pane too short for the list
  shows the choices on one row.

## Impact

- `crates/smith-tui`: render/approval.rs, render/modal.rs, render/layout.rs,
  app/input.rs, app state; render and state tests.
- `DESIGN.md` §2, §5, change-view and accessibility text.
- Spec: `client-surfaces` (one ADDED, two MODIFIED requirements).
