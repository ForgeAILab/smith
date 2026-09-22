---
created_at: 2026-09-22T16:00:00-04:00
updated_at: 2026-09-22T22:08:21Z
---

# Proposal: Add selectable context windows

## Why

Some models are served with more than one context window, and the choice
changes both cost and how much transcript survives before compaction:

- On a ChatGPT subscription, the GPT-5.6 family (`gpt-5.6-sol`,
  `gpt-5.6-terra`, `gpt-5.6-luna`) and `gpt-6-astra` all ship a
  272,000-token default window with an opt-in extended window. The backend
  catalog reports `max_context_window = 872000`, and usage beyond 272k counts
  about 2x.
- The same models on the OpenAI Platform advertise 1,050,000 tokens, but
  input over 272k is billed at a premium tier, so a 272k window is a sensible
  choice there too.

Smith resolves exactly one limit set per `provider/model`. The only way to
change it today is a hand-written `[models."p/m"]` override that must set
`context_tokens` and `max_input_tokens` together, because the trusted
`max_input_tokens` (255,616 for Terra) otherwise silently caps the larger
window. Only Terra has trusted ChatGPT metadata at all; Sol, Luna and Astra
each need a hand-written `[models]` block. You cannot switch windows during a
session.

## What Changes

- A model binding MAY declare named **context windows**, one of which is its
  default. Each window carries its own `context_tokens` and optional
  `max_input_tokens` (derived as `context_tokens - max_output_tokens` when
  omitted).
- Trusted ChatGPT metadata covers `gpt-5.6-sol`, `gpt-5.6-terra`,
  `gpt-5.6-luna` and `gpt-6-astra`, each with a `272k` (default) and an
  `872k` window and Terra's conservative 16,384-token output
  cap. Sol, Luna and Astra then work on ChatGPT without a `[models]` block.
- The endpoint-bound OpenAI Platform catalog entries for the same four models
  gain a `272k` window next to their catalog window, named `1m` (1,050,000
  tokens), which stays the default.
- Other models get windows only from explicit config.
- Selection sources, lowest to highest precedence: model default, profile
  `context_window`, `--context-window` flag, and the `/context <NAME|default>`
  session override. The session override persists on resume, like `/effort`.
- `/context` with no argument keeps its current report and adds the list of
  available windows, marking the active one. `/model` detail and the status
  line show the active window name when a model has more than one.
- `smith config explain context_window` reports the winner and every source
  that was overridden.
- Switching to a smaller window at an idle boundary rebuilds the runtime with
  the new limits. The next turn compacts first if the transcript no longer fits.
- **Not breaking:** models with no declared windows resolve exactly as before.
  An existing explicit flat `[models]` `context_tokens` (for example a
  hand-written Luna block) still wins and pins the model to that one window.
  Selecting a named window then fails and names the pinning key. Declaring
  both flat limits and `context_windows` in the same config layer is rejected
  as ambiguous.

## Non-Goals

- Discovering windows live from `chatgpt.com/backend-api/codex/models`
  (`context_window` / `max_context_window`). This is a good follow-up. The
  catalog response depends on the `originator` header, so it needs its own
  review.
- Anthropic `context-1m` beta headers. The current Claude/Fable models in the
  catalog are natively 1M.
- Changing any model's default window.
- Model ids that are missing from both the Codex backend catalog and
  Models.dev (for example a `gpt-6-sol` or `gpt-6-luna`). Once one ships it
  gets a trusted record, and until then a `[models]` block works.

## Impact

- Affected specs: configuration, client-surfaces, provider-runtime
- Affected code: `smith-config` (model schema, resolve, setup trusted
  records), `smith-runtime` (`catalog.rs`, `factory.rs`), `smith-cli`
  (flag, runtime host session overrides, config explain, resources),
  `smith-tui` (`/context` parsing, status line, model picker detail), and
  `docs/configuration.md`.
- Agent Runtime: no API change is expected. Limits already reach it through
  `ResolvedModelProfile`, and nothing in its checkpoints pins them. Task 3.3
  checks that its planner compacts once the budget shrinks. If it does not,
  that one fix lands in the runtime compat branch.
