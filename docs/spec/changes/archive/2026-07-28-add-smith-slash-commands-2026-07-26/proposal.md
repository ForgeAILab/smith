---
created_at: 2026-07-27T02:57:00Z
updated_at: 2026-07-27T20:27:43Z
---

## Why

Live testing showed the Smith composer sends `/`-prefixed input straight to
the provider as a prompt: no local dispatch, provider spend on what the user
meant as a command, and no discoverable command surface. Slash commands are a
standard terminal-agent affordance and are not covered by any existing Smith
requirement.

## What Changes

- Intercept composer input whose first non-whitespace character is `/` and
  dispatch it locally instead of sending it to the model; unknown commands
  produce a local error with help, with no provider request.
- Add a command registry whose built-ins map to existing host actions
  (model picker, session controls, help) rather than duplicating logic.
- Add `/help` output listing available commands with one-line descriptions.
- Make `/status` expose the latest enforced context plan as percent left,
  tokens used, input budget, model window, reserves, provenance, and bounded
  segment totals. Before the first turn it names that no plan exists yet
  instead of inventing usage.
- Add `/context` as a focused inline visualization of the latest enforced
  plan, with a responsive-safe usage map, category legend, free input space,
  reserves, compaction state, and exact/estimated provenance.
- Add a documented escape so a literal message beginning with a slash can
  still be sent to the model.

## Impact

- Affected specs: `client-surfaces` (added requirements).
- Affected code: `crates/smith-tui/src/composer.rs` (submit path),
  `crates/smith-tui/src/app.rs` (action dispatch), rendering for help and
  local errors.
- No shared-runtime changes; this is Smith-local presentation and dispatch.
- Coordinates with the active `add-smith-agent-harness-2026-07-23` change,
  which owns `client-surfaces` truth-to-be; requirements here are additive
  and do not modify that change's requirements.
