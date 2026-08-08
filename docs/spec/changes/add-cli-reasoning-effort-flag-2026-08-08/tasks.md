---
created_at: 2026-08-08T04:55:00Z
updated_at: 2026-08-08T04:55:00Z
completed_at:
---

## 1. Configuration layer

- [ ] 1.1 Add an invocation-scoped effort field to `Overrides` in
  `crates/smith-config/src/resolve/provenance.rs`, distinct from the existing
  `reasoning_effort` that carries session selections, and contribute it to
  `reasoning.effort` from `Overrides::contributions` only for the command-line
  layer.
- [ ] 1.2 Make `Source`'s command-line `Display` in the same file name the flag
  the user can actually type for `reasoning.effort` (`--effort`), rather than
  the mechanical `flag_spelling` result `--reasoning-effort`.
- [ ] 1.3 Add a precedence test to `crates/smith-config/tests/precedence.rs`
  proving a command-line effort beats a profile `[profiles.<name>.reasoning]
  effort` and `SMITH_REASONING_EFFORT`, and loses to a session override.
- [ ] 1.4 Add a test proving `explain("reasoning.effort")` reports the
  command-line layer and keeps the overridden profile entry visible.

## 2. CLI surface

- [ ] 2.1 Add the invocation effort field to `Selection` in
  `crates/smith-cli/src/cli.rs` and map it through `Selection::overrides()`,
  leaving `session_overrides()` untouched.
- [ ] 2.2 Parse `--effort <NAME>` in `parse_selection_flag` using
  `text_value` + `set_once`, so it is accepted by `smith`, `smith -p`,
  `smith config explain`, and `smith sessions list` through the shared parser.
- [ ] 2.3 Add `--effort <NAME>` to the `HELP` golden constant's `RUN OPTIONS:`
  block, describing it as a provider-advertised effort for this run.
- [ ] 2.4 Add parser tests in `crates/smith-cli/src/cli.rs`: the flag parses
  inline and spaced, is rejected when supplied twice, is rejected without a
  value, and does not populate the session-override field.
- [ ] 2.5 Add a black-box case to `crates/smith-cli/tests/cli_contract.rs`
  proving `--help` lists the flag and that an unknown effort fails with a
  non-success status naming the value and the supported alternatives.

## 3. Resume interaction

- [ ] 3.1 In `crates/smith-runtime/src/host.rs`, teach the resume path that an
  explicit command-line effort suppresses the persisted effort for this run,
  reusing the existing `reasoning_reset` request channel rather than adding a
  parallel one.
- [ ] 3.2 In the same file, carry a suppressed persisted effort forward into
  what the session store writes back, so a run made with `--effort` does not
  erase the session's own `/effort` choice.
- [ ] 3.3 Wire the suppression from `crates/smith-cli/src/runtime_host.rs`
  where `HostSessionRequest::reasoning_reset` is already built from the
  selection.
- [ ] 3.4 Add a `crates/smith-runtime/tests/host_session.rs` case:
  resume with `--effort` uses the flag, and resuming again without it restores
  the persisted override unchanged.
- [ ] 3.5 Add a case proving a mid-run save while the flag is active does not
  rewrite the persisted override to the flag's value.

## 4. Startup behavior

- [ ] 4.1 In `crates/smith-cli/src/runtime_host.rs`, exclude an explicitly
  supplied invocation effort from the `is_reasoning_startup_error` recovery
  arm, so the run fails with the reasoning diagnostic instead of silently
  clearing the selection and continuing.
- [ ] 4.2 Clear the invocation effort when composing child-profile selections
  in the same file, alongside the existing `reasoning_enabled` /
  `reasoning_effort` suppression, so a child profile on a non-controllable
  binding does not abort startup.
- [ ] 4.3 Add a test proving an unsupported `--effort` on an interactive start
  fails rather than starting with a notice.
- [ ] 4.4 Add a test proving a controllable main binding plus a
  non-controllable child profile still starts when `--effort` is supplied.

## 5. Refusal coverage

- [ ] 5.1 Add a `crates/smith-runtime/src/reasoning.rs` test proving a
  command-line effort on a binding with no adjustable controls fails with the
  provider, model, and capability source, before any credential lookup.
- [ ] 5.2 Add a test proving a controllable binding that advertises no ladder
  (toggle-only) refuses the flag with "no effort levels are advertised".
- [ ] 5.3 Add a test proving `--effort off` is refused as an unadvertised value
  and does not disable reasoning on any dialect.

## 6. Documentation

- [ ] 6.1 Add `--effort NAME` to the command-line selection block in
  `docs/configuration.md`.
- [ ] 6.2 In the reasoning section of `docs/configuration.md`, state where the
  flag sits in precedence, that it does not disable reasoning, and that it
  shadows rather than replaces a session's persisted override on resume.
- [ ] 6.3 Document `SMITH_REASONING_EFFORT` alongside it, since it already
  resolves and is currently unmentioned.

## 7. Verification

- [ ] 7.1 `cargo test -p smith-cli -p smith-config -p smith-runtime`.
- [ ] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo fmt --all --check`.
- [ ] 7.3 Exercise the flag end to end against a real controllable binding:
  `smith -p '...' --effort low --output-format json` and confirm the result
  envelope's `reasoning` block reports the flag's effort and a command-line
  selection source.
- [ ] 7.4 Re-run `spec_toolkit validate add-cli-reasoning-effort-flag-2026-08-08`.
