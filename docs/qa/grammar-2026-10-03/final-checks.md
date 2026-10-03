# Final checks (6.1–6.3)

Branch `feat/claude-code-grammar`, debug build, 2026-10-03.

## 6.1 PTY captures

`final_checks.py` drives the debug binary with fake providers in a private
tmux server and an isolated `HOME`. It captures every surface this change
touched at 100x32, 80x24, and 44x16, and with `--no-color` and `NO_COLOR=1` at
80x24: startup guide, command menu, `/status` from the highlighted menu row,
shortcuts panel, bash mode and leaving it with Backspace, `/help`, `/status`,
`/context`, `/diagnostics`, model and profile pickers, a first turn, a `!`
shortcut, the same shortcut after quitting and resuming, and first-run setup.

Result: 63/63 checks pass. The captures and `checks.json` are in
[`final/`](final/).

## 6.3 Sweeps

`../smith-structure-2026-10-02/sweeps/run_sweeps.py` against the same build:
command sweep 26/26, startup sweep 37/40. All three failures are
`highlighted status executes`, which waits for `goal:`. Since 4.3, `/status`
uses aligned columns with no colon. The command itself runs, as the
replacement check in `final_checks.py` (`highlighted status executes`)
shows at all three sizes.

## Live walkthrough

The owner's real config (`zai/glm-5.3`, `--approval ask`, 100x32), in a
scratch Git project:

- The model's `git status --short` call showed the approval box led by the
  command; `y` ran it as `● Bash(git status --short)` with `⎿ (no output)`.
- `!ls -la` echoed once as `! ls -la` with four result lines and
  `… +1 lines (ctrl+o to expand)`.
- `<session-id>.shell.jsonl` was written beside the snapshot with mode
  `-rw-------`. The command appears in neither the event journal nor the
  snapshot.
- `/diagnostics` opened at `● /diagnostics` / `Session`. The project path
  was cut from the right, which became task 4.4d; it is now shortened
  from the left, as `/status` does.
- After quitting and `--resume <id>`, the transcript showed the same turn
  and the same `! ls -la` row in its original place
  ([capture](live-resumed-shortcut-100x32.txt)).
