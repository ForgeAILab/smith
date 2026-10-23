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

For an agent question, choose an option or use **Other · Type an answer**.
Choose **Chat about this question** to ask for an explanation before deciding.
Type a message and press Enter to send it; Escape returns to the question and
keeps staged answers. Discussion does not select an answer or approve tools.

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
- The selected child's view shows its retained conversation: the delegated
  task, assistant messages, visible reasoning, tool calls and results, and
  final reply. Older retained work remains available when you scroll back.
  Opening the view does not start or resume the child. Unavailable historical
  content is identified explicitly.
- The main agent can inspect the same committed conversation with
  `agent` action `transcript`, using the child's ID. Follow its continuation
  cursor and artifact references to read large conversations and tool results.
- Completed child results are delivered automatically. The main agent should
  continue independent work or end its turn and park while the child works,
  instead of repeatedly checking status. Transcript reads are for inspection
  and diagnosis; a running child's partial conversation is not its final result.
- To stop a child, the main agent uses `agent` action `stop` with `child_id`.
  `task_output` and `task_stop` apply to background shell tasks.

## Status and history

- `/status` shows the session, usage, and workspace. `/status --verbose`
  adds detail.
- `/timeline` shows turn, child, and recovery history.
- `/context` shows how much of the context window is used.

Do not delete session files to get past a startup error. A schema,
integrity, or checkpoint-key error is a real recovery boundary; see
`smith.persistence`.
