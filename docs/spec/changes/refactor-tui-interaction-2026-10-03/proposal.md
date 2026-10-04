---
created_at: 2026-10-03T00:00:00Z
updated_at: 2026-10-04T02:22:04Z
---

## Why

The TUI has ten confirmation dialogs (undo, redo, revert, MCP trust, skill
trust, review, child invocation, child follow-up, child resume, credential
rotation) plus the exit confirmation, each an `Overlay` variant with its own
renderer and key handling. They show a fixed number of body lines, so undo,
redo, and revert say "Review the complete reverse patch" and silently cut the
patch off. Seventeen sites assign the single overlay slot directly, so one
dialog can replace another; only approvals and questionnaires queue. Notices
are a free-form source string and text from 64 call sites, and every one
becomes a permanent transcript row, including feedback such as "/model
requires an idle turn; draft preserved". This is the rest of roadmap step 3
(S2, S5, S6) in the
[2026-10-02 audit](../../../qa/smith-structure-2026-10-02/report.md).

## What Changes

- One confirmation component replaces the ten confirm overlays and the exit
  confirmation: a title, an optional warning line, a body, an accept label,
  and the actions accept and cancel produce. One renderer and one key
  handler; the body scrolls like an approval's detail, so a long patch can
  be read to the end.
- Opening an overlay goes through one policy. Prompts that must be answered
  (approvals, questionnaires, confirmations) queue in arrival order and are
  never replaced. Pickers, the palette, history search, and the shortcuts
  panel never replace a prompt, and close when a prompt arrives.
- Notices get a typed kind that fixes their label, and an explicit
  persistence. Most stay transcript rows as today. Feedback that answers a
  keypress and records nothing (a refused command, an empty clipboard, an
  already-current selection) shows in the hint row until the next key and is
  not a transcript entry.
- Keymap-driven dispatch (S5) stays out of this change.

## Impact

- Affected specs: client-interaction, client-surfaces.
- Affected code: `smith-tui` `app/state.rs` (`Overlay`), `app/input.rs`,
  `app/prompts.rs`, `app/resources.rs`, `render/modal.rs`, `render/layout.rs`,
  `render/composer.rs` (hint row), `transcript.rs` (notices); notice call sites
  in `smith-cli`; a notice kind type in `smith-client`.
- Visible changes: confirmation bodies scroll; feedback moves from the
  transcript to the hint row. Every other label and confirmation text stays.
