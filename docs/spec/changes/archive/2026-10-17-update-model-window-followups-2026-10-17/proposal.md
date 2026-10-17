---
created_at: 2026-10-17T06:23:25Z
updated_at: 2026-10-17T20:03:09Z
---

## Why

`update-model-window-inline-2026-10-16` made setup's default-model step the
`/model` picker and flagged four follow-ups. The owner approved all four on
2026-10-17:

- **`/context` has no picker.** It lists the active model's named windows
  inside its usage report and switches only through `/context NAME`, while
  `/model`, `/effort`, `/think`, and `/advisor` open a picker on the current
  choice.
- **Setup lists installed agents differently from `/model`.** Change default
  shows a configured agent pair as `cli/claude-code/opus  google · 200k
  context`; `/model` shows one row per agent model, `opus  Claude Code CLI ·
  200k context`, and lists agents that no profile names yet.
- **A model configured only with named windows can be missing from `/model`
  and setup.** The earlier captures show it:
  `before-model-windowed-filter.txt` filters `/model` for `window` and gets
  `No matches` while `remote/window-model` is the active model. The local
  inventory gives a windows-only model limits only when it is active and its
  window declares `max_input_tokens`. Otherwise it drops the model. The
  runtime runs the same model by taking the default window and deriving the
  input limit as context minus output.
- **Setup's layout is mixed.** The default-model step sits at the bottom of
  the screen; every other pre-session screen (welcome, credentials, fields,
  review, the ChatGPT login screens setup chains into, and `smith --resume`)
  starts at the top-left.

## What Changes

- **`/context` picker.** Bare `/context` still appends the usage report,
  unchanged. When the session is idle and the active model has more than
  one named window, it also opens `Choose context window` above the composer,
  the same compact pane as `/model`. The pane opens on the current window,
  marked `✓ current`, and Enter switches to the chosen window exactly as
  `/context NAME` does. Each row is the window's name; the selected row
  reads `same as /context NAME`. With one window or none, or during a turn, only the
  report appears, as today (a turn is refused a window switch anyway).
  `/context NAME` and `/context default` are unchanged, and so is the
  headless and machine `/context` command.
- **Setup's agent rows.** Setup's default-model choice builds its installed
  agent rows with the code `/model` uses: one row per agent model, the same
  name, description, detail, and the same `not on PATH` availability. Setup
  still lets every listed provider model become the default. An agent still
  needs a provider to plan its limits against. As `/model` does, setup keeps
  the current default's provider, or uses the first configured provider when
  that one is unavailable.
- **Named-window models listed.** The inventory previews a windows-only
  model the way the runtime runs it. A model that is not active uses its
  `default_context_window`. A window without `max_input_tokens` gets context
  minus the model's output ceiling, sourced from that window. So such models
  appear, and are marked current, in `/model`, setup, and `/provider`
  counts.
- **Pre-session screens sit at the bottom.** Every screen shown before a
  session exists draws its title, content, and controls at the bottom of the
  terminal, with its controls on the last row. That covers every setup step,
  the ChatGPT and xAI login screens, the connection review, and the
  `smith --resume` picker. Content taller than the terminal still uses the
  whole screen and scrolls as before. One shared layout function anchors all
  of these screens, so a setup flow that hands off to login does not jump
  from the bottom to the top.

This change supersedes two statements in the still-active
`update-model-window-inline-2026-10-16` proposal: "Setup's other steps keep
their top-left placement" and the paragraph that left `/context` without a
picker. That proposal now points here.

Unchanged: what each chooser selects and writes, Escape going back a step in
setup and cancelling elsewhere, setup's review and field text, and the
embedded (in-session) placement of every flow.

## Impact

- Affected specs: `client-surfaces`
- Affected code: `crates/smith-config/src/inventory.rs` (window preview),
  `crates/smith-tui/src/app/{resources,state}.rs` (`/context` picker),
  `crates/smith-tui/src/picker/rendering.rs` (bottom-anchored standalone
  layout), `crates/smith-cli/src/resources.rs` and
  `crates/smith-cli/src/setup/choices.rs` (shared agent rows), standalone
  screen fixtures under `crates/smith-cli/tests/fixtures/standalone/`
