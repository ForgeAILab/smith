# Approval prompt, live (3.4, 3.5)

zai/glm-5.3, `--approval ask`, 100x32, debug build of feat/claude-code-grammar.
Prompt: run `git status --short` with the shell tool.

Default view, in order: `git status --short`; `in <cwd> · up to 2 min ·
deadline no deadline`; `Warning: Runs outside the sandbox with your files,
environment and credentials, child processes, network, and data egress.`;
`Do you want to proceed?`; `y Yes`, `a Yes, don't ask again for this exact
shell action this session`, `n No (esc)`; `ctrl+o details`. No identity hash,
permission list, or raw arguments.

- Enter: the prompt stayed open.
- Ctrl+O: identity, target hash, permissions, and raw arguments appear; ↑↓
  scrolls them inside the box; Ctrl+O folds again.
- PageUp: the prompt stayed open and pending.
- `n`: the call row became `● Bash(git status --short) failed` with
  `⎿  approval declined: the user declined`, followed by
  `● approval · shell denied` ([capture](approval-denied-100x32.txt)).

Polish found: no inner padding between the border and the text; the deadline
reads `deadline no deadline`; the tool row says `running 10s` while it waits
for approval; a denied call reads `failed` rather than `denied`, and the
separate `approval · shell denied` row repeats it.

## After the 3.5a polish

Same prompt and build after the fixes
([capture](approval-denied-after-polish-100x32.txt)):

- The box text is padded two spaces from the border.
- No deadline: the place line reads `in <cwd> · up to 2 min`.
- While the prompt is open the call row reads
  `● Bash(git status --short) waiting for approval`.
- After `n`: `● Bash(git status --short) denied` with
  `⎿  approval declined: the user declined`, and no separate
  `approval · shell denied` row.
