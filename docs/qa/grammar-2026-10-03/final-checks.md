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

## Cache, 0.3.0 against this build

`cache_ab.py` repeats the 0.3.0 method: three headless turns on a project
with a ~50 KB file (read in turn 1, two follow-ups via `--resume`), the
owner's real configuration, the same prompts on each build. The new build is
the release build of `b1b8a91`; results are in [`cache-ab.json`](cache-ab.json).

| Model | Build | Turn 1 cached | Turn 2 cached | Turn 3 cached | Turn 2 / 3 uncached |
|---|---|---|---|---|---|
| zai/glm-5.3 | 0.3.0 | 4,032 | 1,728 | 2,880 | 1,200 / 106 |
| zai/glm-5.3 | this | 5,824 | 1,792 | 2,880 | 1,149 / 119 |
| google/gemini-3.8-flash | 0.3.0 | 0 | 0 | 0 | 1,967 / 2,051 |
| google/gemini-3.8-flash | this | 0 | 0 | 0 | 1,939 / 2,100 |
| xai/grok-4.3 | 0.3.0 | 1,024 | 2,496 | 2,752 | 302 / 315 |
| xai/grok-4.3 | this | 384 | 2,432 | 2,688 | 305 / 235 |

Follow-up turns carry the same uncached input on both builds, so the request
prefix did not grow. Turn 1 differs with how the model chose to read the file,
as in the 0.3.0 run. Gemini reports no cache reads on either build. No miss
or re-billed tokens were reported on any turn.
