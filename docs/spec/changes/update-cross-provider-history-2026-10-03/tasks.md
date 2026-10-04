---
created_at: 2026-10-03T00:00:00Z
updated_at: 2026-10-04T00:57:10Z
completed_at: 2026-10-04T00:57:10Z
---

Approved 2026-10-03 ("continue with the road map ... good ui ux. and strong
backend"); the cross-provider failure was listed as the v0.3.2 known issue.

## 1. Runtime

- [x] 1.1 Agent Runtime `update-reasoning-history-provenance` lands on
  `fix/smith-cross-provider-reasoning` with its tests passing, and the
  branch is pushed to `ForgeAILab/agent-runtime`.
- [x] 1.2 Bump the six `rev` pins in the root `Cargo.toml` to that revision
  and update `Cargo.lock` (`--locked` builds must pass with no `[patch]`).

## 2. Smith

- [x] 2.1 Supply the new optional reasoning field wherever Smith constructs
  `ContentPart::Reasoning`; Smith's own decoders leave it unset.
- [x] 2.2 A Smith test drives a host through a turn on one fake provider
  that returns signed reasoning, switches provider, and asserts the next
  request carries none of that reasoning (copy the persistent host fixture
  in `crates/smith-cli/src/main_tests/local_shell.rs` `Fixture::new`).

## 3. Verification

- [x] 3.1 `cargo fmt --all -- --check`, strict Clippy, workspace tests, and
  `cargo deny --locked check all`.
- [x] 3.2 Live matrix (Gemini, Z.AI, xAI, Anthropic; one turn then a resumed
  turn on each other provider): every pair that failed on 0.3.2 succeeds.
- [x] 3.3 Cache comparison against 0.3.2 matches turn for turn after turn 1
  on the same model (acceptance gate, not a formality).
- [x] 3.4 Record the matrix in `docs/qa/cross-provider-2026-10-03/`; the
  next release notes list the fix instead of the known issue.
