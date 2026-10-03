---
created_at: 2026-10-02T10:41:40Z
updated_at: 2026-10-02T10:41:40Z
---

## Why

`DESIGN.md` says Smith's text hierarchy follows OpenAI Codex CLI 0.145.0.
The owner's direction is Claude Code, and parts of the code already follow it
(ephemeral turn summary, paste placeholders, result previews) while the
contract and specs still describe the Codex grammar. The result is a surface
that is consistent with neither: one shell command prints four rows, streamed
text is raw until commit, approvals lead with a hash and JSON, and pickers
lead with provenance.

Evidence: [audit](../../../qa/smith-structure-2026-10-02/report.md)
(findings U5, U6, U7, U9, U10, U14, U15). Decision recorded 2026-10-02.

## What Changes

- Rewrite the reference in `DESIGN.md` from Codex CLI to Claude Code, and
  list the Smith decisions that stay (16-color ANSI, no header, inline
  informational results, steer/queue keys, `y`/`a`/`n` approvals).
- One transcript row per tool call, with its result nested beneath it and an
  expand key. Remove the duplicate invocation row and the change notice for
  commands that changed nothing.
- A working row that carries elapsed time, token flow, and the interrupt key.
- The turn summary stays ephemeral and attaches to the turn, not to whatever
  was printed last. The truth spec is corrected to match.
- Streamed text renders through the same Markdown path as committed text;
  lists, quotes, tables, and links with visible targets are supported.
- Approvals lead with the action in plain form; identity hash and raw
  arguments move behind the expand key. The transcript stays scrollable
  while a prompt is open.
- Command menu, pickers, `/help`, and `/diagnostics` use aligned columns
  and word wrap, name first, state second, metadata on the selected row only.
- Composer: movement between lines of a draft, standard line-editing keys, a
  bash-mode prompt for `!`, and a shortcuts panel on `?`.
- First-run setup uses one frame and wraps descriptions.
- A user `!command` is saved with its bounded result so a resumed session
  shows it (decided 2026-10-03).
- `/diagnostics` values render exactly as stored, and a child's state reads
  the same on every surface (decided 2026-10-03).

## Impact

- Affected specs: client-surfaces, client-interaction.
- Affected code: `DESIGN.md`; `smith-tui` transcript, render, composer,
  picker, approval, setup; the report renderers introduced by
  `refactor-client-structure`.
- Depends on: `fix-interaction-defects` (approval guard, no-approval
  shortcut). Tasks in section 4 depend on `refactor-client-structure`
  (typed reports). Tasks in section 5 depend on the two in-flight TUI
  changes being committed.
- Persistence: one Smith-owned sidecar per session,
  `<session-id>.shell.jsonl`, for user shell shortcuts (decided 2026-10-03).
- No change to provider requests, approval policy, model-visible history,
  machine output, or the event journal.
