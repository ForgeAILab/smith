---
created_at: 2026-10-04T00:00:00Z
updated_at: 2026-10-04T06:29:09Z
---

## Why

A live pass on the installed 0.3.5 against the real configuration
(`docs/qa/live-2026-10-04/findings.md`) found one input bug and several
surfaces that still print internal values, repeat themselves, or cannot be
reviewed:

- In a draft that starts with `/`, Ctrl+U, Ctrl+W, and the other
  line-editing keys do nothing, so a refused command cannot be cleared and
  the next command is appended to it (L1).
- The child inspector shows the child's answer as raw Markdown and repeats
  its identity; `/agent` is headed `/agents`; the agents panel row is filled
  by a session id (L2–L4).
- Each tool activation adds an internal `activation epoch` line to the
  transcript (L5).
- Approval and undo boxes print absolute paths, and undo, redo, and revert
  show whole-file patches instead of the lines that changed (L6, L7).
- Assorted wording slips (L8).

## What Changes

- The slash command menu passes every key it does not use itself to the
  composer, so line editing works in a slash draft; the key-table check
  covers slash drafts.
- `/agent` output is headed `/agent`; counts read `1 turn` / `2 turns`;
  token counts use Smith's compact form (`3.1k`). The agents panel row
  leaves out the session id.
- The inspector states each fact once, labels the turn count, renders the
  child's result as Markdown, offers `/agent resume` only when an exact
  checkpoint exists, and drops the `no activity recorded` line when a result
  is shown.
- The spawn row's result preview reads as words (`child-1 started · its
  result arrives when it completes`) instead of JSON.
- Capability activation notices appear only with work detail expanded
  (Ctrl+O) and in `/diagnostics`, like the `registry.search` row that
  causes them.
- Approval, undo, redo, and revert name files relative to the project when
  they are inside it.
- Undo, redo, and revert patches are line diffs in hunks with three lines of
  context, built once in `smith-tools` and used by both the preview and the
  fingerprint it journals.
- Wording: `1 unchanged line`; the `a` choice reads `Yes, and don't ask
  again for edit in this target this session`; the undo box has the
  approval box's inner padding; undo after resume says `undo is not
  available for turns from before this session was resumed`; `/model` writes
  every context size in the compact form.

## Impact

- Affected specs: client-surfaces, change-review.
- Affected code: `smith-tui` `app/input.rs`, `render/transcript.rs`,
  `render/approval.rs`, confirm rendering, `app/reducer.rs`; `smith-client`
  `agent_report.rs`, `keymap.rs`; `smith-tools` `change.rs`, `display.rs`;
  `smith-cli` `resources.rs`.
- Fixtures for `/agent`, the inspector, approvals, undo/redo/revert, and
  `/model` are re-recorded and reviewed.
- Undo fingerprints change for previews made by this version; previews
  journaled by an older version cannot be applied after resume anyway.
- The exit summary's single model name stays an open question.
