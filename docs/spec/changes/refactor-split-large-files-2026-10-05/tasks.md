---
created_at: 2026-10-05T08:42:01Z
updated_at: 2026-10-05T08:42:01Z
completed_at:
---

Approved 2026-10-05 ("lets do 1 and clear all large files. you may use
codex"; limit: over 1,500 lines). Code written by Codex, one slice per
worktree branched from this change's first commit; slices touch disjoint
files and change no public path, so they merge without conflicts.

## 1. Split by slice

- [ ] 1.1 `smith-config`, `smith-tools`, `smith-client` (7 files)
- [ ] 1.2 `smith-runtime` source (9 files)
- [ ] 1.3 `smith-runtime` integration tests (3 files, as directory targets)
- [ ] 1.4 `smith-tui` (8 files)
- [ ] 1.5 `smith-cli` (3 files)

Each slice: every touched file is 1,000 lines or less; `cargo fmt --check`,
the slice crates' tests, and `cargo clippy -p <crates> --all-targets --locked
-- -D warnings` pass; no fixture file changes; test counts per crate match
the baseline.

Baseline test attributes (`#[test]` and `#[tokio::test]`) at `e03e8e5`:
smith-cli 295, smith-client 96, smith-config 277, smith-host 45,
smith-runtime 651, smith-tools 177, smith-tui 647.

## 2. Guard and integration

- [ ] 2.1 Merge the five slices.
- [ ] 2.2 Add the line-budget test to `smith-runtime/tests/architecture.rs`:
  every `.rs` file under `crates/` is 1,500 lines or less.
- [ ] 2.3 Full gate: `cargo fmt --check`;
  `cargo clippy --workspace --all-targets --locked -- -D warnings`;
  `cargo +1.88.0 clippy` (MSRV); `cargo test --workspace --locked
  --no-fail-fast` with the per-crate test counts equal to the baseline plus
  the guard test; `cargo deny --locked check all`.
- [ ] 2.4 Smoke the built binary: `smith --version`, a headless run, and the
  TUI start screen.
