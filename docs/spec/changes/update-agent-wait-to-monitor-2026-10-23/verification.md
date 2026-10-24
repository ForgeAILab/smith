# Verification

The wait implementation and its client fixtures are complete. Task 3.1 remains
open because the shared checkout contains other changes that fail the full gate.
No runtime dependency revision or package version was changed for this proposal.

## Passing checks

- Delegation suite: 44 passed, including eight controlled-clock timer fixtures,
  terminal-result acknowledgement, and completion delivery.
- TUI suite: 728 passed, including monitor deadlines at 44, 80, and 140 columns.
- CLI contracts: 140 passed. Both harness dialects return immediately and admit
  exactly one timeout turn with status and no partial child output.
- CLI unit tests: 416 passed, one ignored, including headless timer attribution
  and the existing terminal-result acknowledgement fixture.
- PTY suite: 20 passed. The new fixture observes the monitor deadline, parent
  parking, automatic timeout wake, child completion, and restored terminal.
- Headless fixtures were regenerated for the revised delegation prompt and
  passed all 11 fixture checks.
- Workspace Clippy passed on stable and Rust 1.88 with warnings denied before
  the concurrent model-fallback runtime patch was enabled.
- The pinned runtime's testkit/conformance suites passed 418 tests.
- The `--no-default-features` build passed; its CLI suite passed 403 tests with
  one ignored. Minimal smoke checks passed against the exact executable from
  Cargo's artifact directory, confirming the version and absent optional modules.
  Another build had replaced the shared `target/debug/smith`, so the smoke
  check used the artifact whose recorded feature set was empty.
- Strict spec validation and `git diff --check` passed.

## Full workspace run

Command:

```sh
TMPDIR=/tmp RUST_MIN_STACK=16777216 cargo test --workspace --locked --no-fail-fast
```

Result: 3,258 passed, three failed, 14 ignored, plus a configuration doctest
compile failure. The larger test-thread stack avoids a stack overflow in the
existing interactive-rebind test while the other composition work is present.

Failures outside this proposal:

- `smith_runtime_public_modules_match_the_allow_list`: the separate image
  change exposes `image_sizing`, which is absent from the public-module list.
- `every_rust_file_is_within_the_line_budget`: `runtime_host.rs` and
  `smith-config/src/model.rs` exceed 1,500 lines in the other changes.
- `no_artifact_store_does_not_claim_recoverable_offloading`: the compaction
  changes reject semantic summaries without a protected artifact store.
- The configuration doctest ran while the model-fallback work referenced a
  `resolve/fallback.rs` file that had not yet been created.

The checkout continued changing during verification. A later workspace Clippy
run picked up the separate runtime model-fallback patch and failed on missing
`fallback` fields and missing `ModelFallback` projection test cases. A formatting
check also found the newly created fallback resolver unformatted. These files
and runtime changes are outside the wait proposal.
A later minimal-build retry encountered another temporarily missing file,
`smith-runtime/src/factory/fallback.rs`, in that same work. The successful
minimal build, test run, and artifact-specific smoke check are recorded above.

## Review follow-ups (2026-10-23)

- A zero-timeout status check still arms nothing, but now reports a timer an
  earlier wait left armed, with its remaining duration; the result note no
  longer promises a deadline wake when none is armed.
- The configuration delta now also modifies "Layered adaptive cache lifecycle
  configuration", whose truth text still said an over-maximum wait is rejected
  and that a wait returns `timed_out = true`.
- Full gate on the combined checkout: formatting, stable and Rust 1.88 Clippy,
  fixtures, the `--no-default-features` build and tests, minimal smoke, and
  cargo-deny pass; 3,282 tests pass. Two failures are not from this change:
  `compact_settings_preflight_before_publication_and_rebind_the_same_session`
  fails only because the local gate exports `SMITH_AGENT_RUNTIME_REVISION`,
  which `scripts/ci.sh` un-exports, and
  `metadata_adoption_waits_for_open_picker_and_active_turn_then_keeps_session_and_draft`
  is a known full-gate flake that passed 3 of 3 alone and in three whole-binary
  runs.
