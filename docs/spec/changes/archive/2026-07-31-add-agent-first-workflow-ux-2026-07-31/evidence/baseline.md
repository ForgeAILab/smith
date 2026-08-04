# Pre-change baseline

Captured from Smith commit `2183a57fa8781610d972781e4be02782f78ad7d7`
before agent-first workflow implementation.

## Compatibility fixture identities

- `machine-result-v2.json`:
  `sha256:b5134d44da81256b0ede5b1d6dbb8b10d996e9707b64e19cb61f36aa14963e4c`
- `reducer-events-v5-attempt-scoped.json`:
  `sha256:293182515582ad9b63b31961a307d3b8122d7f2b1bf4b9587b72686e7c2f809b`

## Executed baseline gates

- `cargo test -p smith-tui live_reducer_and_journal_replay_produce_equivalent_ui_state`
  — 1 passed.
- `cargo test -p smith-cli --test cli_contract` — 13 passed.
- `cargo test -p smith-runtime --test composition live_factory_emits_registry_view_retrieval_activation_and_context_lifecycle`
  — 1 passed.
- `cargo test -p smith-runtime --test host_session unavailable_checkpoint_key_keeps_completed_turn_persistence_honest`
  — 1 passed.
- `cargo test -p smith-cli --test setup_pty deterministic_host_renders_in_real_narrow_normal_and_wide_terminals`
  — 1 passed at the existing 44×14, 74×24, and 120×32 contract sizes.

All commands exited zero on macOS. These commands are rerun after each
corresponding implementation slice and again in the final workspace gate.
