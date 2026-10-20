# Sessions, undo, goals, and child agents

Smith saves each session per project under `~/.smith/sessions`, unless
`persistence.enabled = false`.

## Resume and start over

- `smith sessions list` prints this project's session IDs. You may run it.
- The user resumes with `smith --resume <id>`, or `smith --resume` for a
  picker. Inside the TUI, `/resume` opens the picker and `/resume <id>`
  resumes directly.
- `/new` starts a fresh session. The old one stays resumable.
- A headless run resumes with `smith -p "<prompt>" --resume <id>`. It needs
  the exact ID, because there is no picker.

When a session has a pending approval or question, resume it interactively
so the user can answer.

## Undo changes

The user types these:

- `/undo` reverts the files changed by the last turn. `/redo` reapplies
  them.
- `/revert <path>` reverts one file or hunk.
- `/diff` shows workspace changes; `/diff staged` shows staged ones.
- `/review` runs a read-only review of the changes.

## Goals

`/goal <objective>` sets a multi-turn goal. Smith keeps working on it
across turns until it is done, blocked, or out of budget. `/goal` alone
shows status. Only persisted root sessions have goals.

## Child agents

- With the `agent` tool, you can delegate to a profile whose `use` includes
  `"child"`. The user can do the same with `@<profile> <task>` in the
  composer.
- `/agent` lists child agents, and `/agent <id>` inspects one.
  `/agent resume <id>` continues an interrupted task. For a new follow-up,
  the user types `@<child-id> <message>`.

## Status and history

- `/status` shows the session, usage, and workspace. `/status --verbose`
  adds detail.
- `/timeline` shows turn, child, and recovery history.
- `/context` shows how much of the context window is used.

Do not delete session files to get past a startup error. A schema,
integrity, or checkpoint-key error is a real recovery boundary; see
`smith.persistence`.
