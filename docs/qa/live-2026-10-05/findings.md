# Live checks for the six archived changes — 2026-10-05

Binary: `~/.local/bin/smith` 0.3.8 (release build at `d5299d8`), real
`~/.smith` configuration, default profile `code` on `zai/glm-5.3`, approval
`ask`. Sessions ran in a private tmux 3.7b server (`tmux -L smithqa`) in two
throwaway Git projects, `proj-alpha` and `proj-beta`.

## Terminal title (`add-terminal-title` 3.2)

Read through tmux's `#{pane_title}`, which tracks OSC 0.

| Step | Title |
|---|---|
| alpha idle | `smith · …/proj-alpha:main · glm-5.3` |
| beta idle | `smith · …/proj-beta:main · glm-5.3` |
| alpha during a turn | `smith · proj-alpha:main · glm-5.3 · working` |
| alpha after the turn | `smith · proj-alpha:main · glm-5.3` (beta unchanged) |
| pane running only Smith, after `/exit` | empty |

In a shell pane the shell's prompt rewrites the title after Smith exits, so the
clear was checked in a pane whose only process was Smith (with
`remain-on-exit`). `smith -p 'Reply with the single word ok.'` with stdout
redirected to a file wrote `ok\n` (3 bytes) and 0 ESC bytes.

Result: pass.

## Mixed-turn undo (`recover-smith-edits-in-mixed-turns` 4.3)

Prompt: edit `README.md`'s first line with the edit tool, then run
`echo shell-wrote > shell.txt`. Both tools were approved with `y`.

- Turn notice: `changes · Smith turn 1 · contains ambiguous changes; /undo
  covers Smith's own edits, /diff shows the rest`.
- `/undo` preview: `this turn also changed the workspace through shell; those
  changes are not attributable file by file and are left untouched — use
  /diff and /revert`, followed by the `README.md` reverse patch only
  (`-# alpha edited` / `+# alpha`).
- After `y`: `undo · restored the edits Smith made in the last turn`;
  `README.md` is `# alpha`; `shell.txt` still contains `shell-wrote`.

Capture: `mixed-turn-undo.txt`. Result: pass.
