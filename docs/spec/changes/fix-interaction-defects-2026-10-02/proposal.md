---
created_at: 2026-10-02T10:20:00Z
updated_at: 2026-10-02T10:40:03Z
---

## Why

The 2026-10-02 audit reproduced defects that a user meets in the first ten
minutes: two first-run setup entries do nothing, an approval can be answered
by text the user was already typing, a cancelled picker corrupts the next
command, three confirmation dialogs show another dialog's text, a documented
configuration key is ignored, and the command line refuses `--help` on its own
subcommands.

Evidence: [audit and captures](../../../qa/smith-structure-2026-10-02/report.md)
(findings U1, U2, U4, U6, U8, U11, U12).

## What Changes

- Every entry offered by guided setup starts its flow. "Anthropic Messages
  API" and "Connect ChatGPT (experimental)" get handlers, and a test fails if
  an offered entry has none.
- Setup labels and review text name the model and catalog revision that are
  actually written.
- A consequential prompt (approval, trust, rotation, recovery confirm) is not
  resolved by keystrokes that were in flight when it appeared. The draft is
  kept.
- Cancelling a picker opened by a command leaves the composer empty.
- Redo, MCP-trust, and skill-trust confirmations state their own action.
- Multi-line prepared-action text keeps its line structure in the approval.
- `smith help`, `smith <subcommand> --help`, and `-h` print usage. A parse
  error prints one recovery hint, not two.
- An interactive launch without a terminal fails with guidance instead of an
  operating-system error.
- A configured `background.exit_policy` applies to headless runs; the flag
  still overrides it.
- A user-typed `!command` runs without an approval prompt; it grants no
  future authority and model-requested shell calls still follow policy.
- `smith sessions list` on a terminal prints a header, aligned columns, and
  local time. Piped output keeps the existing tab-separated contract that the
  Claude Code plugin reads.

Not in this change: restructuring, new commands, approval layout, help and
diagnostics wording, picker row layout, composer editing keys. Those follow
`refactor-client-structure` (see the audit roadmap).

## Impact

- Affected specs: client-surfaces, client-interaction.
- Affected code: `smith-tui` setup, input, prompts, picker, modal, approval
  rendering; `smith-cli` argument parsing, `main.rs` startup checks, session
  listing; `smith-tools` shell prepared-action text only if the renderer fix
  is not sufficient.
- Overlap: `app/input.rs` and `app/prompts.rs` are not touched by the two
  in-flight changes. `app/state.rs` is; the approval guard needs one field
  there and should land after `show-provider-retry-progress` and
  `show-idle-compaction-summary` are committed.
- No change to credentials, provider requests, approval policy, persistence,
  or machine-output schemas.

## Decisions (owner, 2026-10-02)

- Approval guard: decision keys are ignored until the prompt has been visible
  for 500 ms with no key arriving in that window; each arriving key restarts
  the window. The `y` / `a` / `n` controls and the rule that Enter never
  answers an approval are unchanged.
- "Connect ChatGPT (experimental)" stays in first-run setup and gets a
  working handler, as does "Anthropic Messages API".
- A user-typed `!command` no longer asks for approval. Submitting it is the
  authorization for that one exact command. It still runs through the
  prepared executor, and model-requested shell calls are unaffected.
- Reference grammar for later UX work is Claude Code, not Codex CLI. See
  `adopt-claude-code-grammar`.
