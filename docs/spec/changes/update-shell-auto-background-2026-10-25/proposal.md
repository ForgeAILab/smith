---
created_at: 2026-10-25T20:16:29Z
updated_at: 2026-10-25T23:18:06Z
---

## Why

A foreground `shell` call blocked a turn for 12 hours. The command started a
server with `cd … && nohup node … > log 2>&1 &`. The redirect applied only to
`nohup`, so the forked `&` subshell kept the tool's output pipe open. The tool
waited for that pipe to close after the shell had exited, with no deadline and
no cancellation check. An earlier call in the same session blocked for 3.5
hours the same way.

The owner wants a long foreground call to move to the background on its own
instead of holding the turn. `timeout_ms` stays the time at which the command
is killed. Whatever `timeout_ms` is, a call still running at 5 minutes moves to
the background.

## What Changes

- **Shipped as a bug fix** (branch `fix/shell-drain-after-exit`): after the
  shell exits, Smith reads the remaining output for at most 2 s, and
  cancellation still reaches the process group during that wait. If a process
  the command started still holds the pipe, it becomes a session background
  task. When no background host can take it, it is stopped with its group. A
  background task whose shell exits first can still be stopped by `task_stop`,
  its deadline, and session shutdown.
- **Proposed**: a foreground call still running 300000 ms after it started
  moves to a session background task. It keeps the output captured so far and
  the remaining `timeout_ms` deadline. The tool result returns the task ID and
  says the command has not completed.
- `timeout_ms` keeps its meaning: the kill deadline, measured from the start
  of the call, whether the command is still in the foreground or has moved to
  the background. A call whose `timeout_ms` is 300000 ms or less is killed in
  the foreground at its timeout, as today.
- Without a background host, a call still running at 5 minutes is stopped
  with its process group, as a process left holding the pipe already is.
- Manual backgrounding (ctrl+b) still hands off with no deadline. If the host
  cannot adopt it, the process group is now stopped instead of left running.
- This replaces the rule that Smith never converts a foreground call to the
  background automatically.

## Impact

- Affected specs: `tool-execution`
- Affected code: `crates/smith-tools/src/shell.rs`,
  `crates/smith-tools/src/background.rs` (`adopt` gains the remaining
  deadline), `crates/smith-runtime/src/background_tasks.rs`, and the TUI and
  headless rendering of a backgrounded shell result.

## Open Questions

- Should the 600000 ms `timeout_ms` maximum rise now that a call longer than
  5 minutes runs as a background task instead of holding the turn?
