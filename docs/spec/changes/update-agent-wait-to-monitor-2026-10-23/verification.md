# Verification

The commit was isolated from the other uncommitted proposals on top of
`7d5f2c6b`. It changes no runtime dependency revision, package version, or
configuration range.

## Passing checks

- `cargo fmt --all -- --check`, strict spec validation, and `git diff --check`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- Delegation suite: 44 passed, including controlled-clock timer expiry,
  replacement, batching, completion priority, shutdown, and acknowledgement.
- CLI contracts: 140 passed, including immediate waits and timeout wakes on
  both installed-agent harness dialects with no partial child output.
- PTY suite: 20 passed. The new fixture observes the monitor deadline, parent
  parking, automatic timeout wake, child completion, and terminal restoration.
- TUI suite: 726 passed, including monitor deadlines at 44, 80, and 140 columns.
- Regenerated headless fixtures: all 11 checks passed against the isolated tree.
- `cargo build --locked -p smith-cli --no-default-features` and
  `bash scripts/smoke-minimal.sh` passed.
- The unchanged pinned runtime's testkit/conformance suites passed 418 tests
  during implementation.

## Remaining full-gate failure

```sh
TMPDIR=/tmp RUST_MIN_STACK=16777216 cargo test --workspace --locked --no-fail-fast
```

The isolated workspace run completed with 3,237 passed, one failed, and 15
ignored. The remaining failure is the unchanged CLI fixture
`metadata_adoption_waits_for_open_picker_and_active_turn_then_keeps_session_and_draft`:
its assertion expects `InteractiveExit::CatalogChanged` at
`crates/smith-cli/src/main_tests/interactive_rebind.rs:1350`.

That fixture also failed when run alone. The minimal CLI suite completed with
401 passed, the same one failed, and one ignored. All wait/monitor fixtures
passed. Task 3.1 remains open for the catalog-adoption failure; the separate
proposals and their earlier shared-checkout failures are excluded from this
commit.
