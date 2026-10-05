---
created_at: 2026-10-05T08:42:01Z
updated_at: 2026-10-05T09:02:21Z
completed_at: 2026-10-05T09:02:21Z
---

Approved 2026-10-05 ("lets do 1 and clear all large files. you may use
codex"; limit: over 1,500 lines). Code written by Codex, one slice per
worktree branched from this change's first commit; slices touch disjoint
files and change no public path, so they merge without conflicts.

## 1. Split by slice

- [x] 1.1 `smith-config`, `smith-tools`, `smith-client` (7 files)
- [x] 1.2 `smith-runtime` source (9 files, in two slices)
- [x] 1.3 `smith-runtime` integration tests (3 files, as directory targets)
- [x] 1.4 `smith-tui` (8 files, in two slices)
- [x] 1.5 `smith-cli` (3 files)

Audit of every slice against `41fb619`, after dropping `use` and `mod` lines
and narrowed visibility, compared as whitespace-free tokens: slice c is
identical; the others differ only by removed `mod tests { }` wrappers,
rustfmt trailing commas, `#[cfg(test)]` on test-only imports and helpers,
`impl` blocks split across children, and `crate::`/`super::` path
adjustments. No item became `pub` that was not public before.

Each slice: every touched file is 1,000 lines or less; `cargo fmt --check`,
the slice crates' tests, and `cargo clippy -p <crates> --all-targets --locked
-- -D warnings` pass; no fixture file changes; test counts per crate match
the baseline.

Baseline test attributes (`#[test]` and `#[tokio::test]`) at `e03e8e5`:
smith-cli 295, smith-client 96, smith-config 277, smith-host 45,
smith-runtime 651, smith-tools 177, smith-tui 647.

## 2. Guard and integration

- [x] 2.1 Merge the slices (seven, merged without conflicts).
- [x] 2.2 Add the line-budget test to `smith-runtime/tests/architecture.rs`:
  every `.rs` file under `crates/` is 1,500 lines or less. A probe file of
  1,501 lines made it fail and name the file; the largest file now is
  `smith-cli/src/headless/tests/fixtures.rs` at 1,486 lines.
- [x] 2.3 Full gate: `cargo fmt --check`;
  `cargo clippy --workspace --all-targets --locked -- -D warnings`;
  `cargo +1.88.0 clippy` (MSRV); `cargo test --workspace --locked
  --no-fail-fast` with the per-crate test counts equal to the baseline plus
  the guard test; `cargo deny --locked check all`.
  All exit 0; tests 2,179 passed, 0 failed, 7 ignored across 43 binaries
  (v0.3.8 gate: 2,178 passed, plus the guard test).
- [x] 2.4 Smoke the built binary: `smith --version`, a headless run, and the
  TUI start screen. `smith 0.3.8`; a headless run on `zai/glm-5.3` answered
  `ok`; the TUI drew its start screen and exited on `/exit`.
