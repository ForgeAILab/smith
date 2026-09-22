---
created_at: 2026-09-22T16:00:00-04:00
updated_at: 2026-09-22T22:08:21Z
completed_at: 2026-09-22T22:08:21Z
---

# Tasks: Add selectable context windows

## 1. Configuration contract

- [x] 1.1 Add `[models."p/m".context_windows.<name>]` (`context_tokens`,
      optional `max_input_tokens`) plus
      `[models."p/m"].default_context_window`; validate names, a declared
      default, and limit ordering; reject a mix with flat `context_tokens`.
- [x] 1.2 Add profile `context_window`, the `--context-window` flag, and the
      session override field, all with provenance, and add
      `smith config explain context_window`.
- [x] 1.3 Extend `TrustedModelRecord` with optional windows. Add ChatGPT
      records for `gpt-5.6-sol`, `gpt-5.6-luna` and `gpt-6-astra` next to
      Terra, each with `272k` (default) and `872k` windows and
      a 16,384-token output cap, and offer them in the `/connect chatgpt`
      model choices.
- [x] 1.4 Add a `272k` window to the endpoint-bound OpenAI Platform entries
      for the same four models, keeping the catalog window (named `1m`) as
      the default.

## 2. Resolution

- [x] 2.1 In `factory.rs`, select the window before building catalog layers
      and feed its limits in at the layer they came from (embedded or
      explicit), keeping `max_output_tokens` unchanged.
- [x] 2.2 Reject an unknown window name before any credential or provider
      I/O, and list the valid names in the error.

## 3. Session surfaces

- [x] 3.1 Parse `/context <NAME|default>` and apply it at the idle boundary
      through the same reconfigure path as `/effort`; persist it on resume.
- [x] 3.2 Show the available windows in `/context`, and the active one in
      the status line and `/model` detail (only for models with multiple
      windows).
- [x] 3.3 Test that switching from a larger to a smaller window mid-session
      compacts before the next provider request. If the runtime planner
      does not, fix it in agent-runtime-compat and bump the pins.

## 4. Docs and verification

- [x] 4.1 Document windows, selection precedence, and the ChatGPT 2x usage
      note in `docs/configuration.md`.
- [x] 4.2 Run `cargo test` for smith-config, smith-runtime, smith-cli, and
      smith-tui, plus clippy.
