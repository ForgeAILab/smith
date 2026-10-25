---
created_at: 2026-10-24T10:14:29Z
updated_at: 2026-10-24T11:24:21Z
---

## Why

A bare `/<skill-name>` invocation only activated the skill and returned the
composer, so the user had to type a second prompt before the skill did
anything. The owner wants invoking a skill to start work right away, the way
Claude Code runs a skill's instructions as the turn prompt.

## What Changes

- `/<skill-name>` (and `/skills use <name>`) admits the skill and immediately
  submits an ordinary user turn whose prompt body is the skill's verified
  instruction text. No second prompt is needed.
- `/<skill-name> <task>` sends the verified instructions, a blank line, and the
  task text through the ordinary prepared submission path. Only the task text
  is parsed for composer references; the authored instructions are preserved
  verbatim.
- Invoking a skill that is already active starts another turn with its body
  and does not append a duplicate active instruction fragment.
- Refusals, the busy-turn rule, collisions, and the installed-CLI restriction
  from `add-explicit-skill-activation-2026-10-21` are unchanged.

## Impact

- Affected specs: `client-surfaces`.
- Affected code: `smith-runtime` `factory/skill_activation.rs`
  (`prepare_skill_invocation` returns the admitted pin with its instructions;
  `prepare_skill_activation` keeps its contract on top of it); `smith-cli`
  `skills.rs`, `runtime_host.rs`, `tui_driver.rs`, `tui_driver/actions.rs`;
  `smith-tui` `app/pending_input.rs`; `docs/skills.md`.
- Recorded after implementation: the change landed in the shared checkout as a
  direct truth-spec edit, and this folder moves that edit into a delta so the
  truth spec changes only at archive.
