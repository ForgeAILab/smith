# Tasks

- [x] Add layered inline-output setting with default, range and provenance tests.
- [x] Reuse exact artifact storage with bounded readable text previews.
- [x] Preserve reported outcome status and typed/multimodal content.
- [x] Clamp valid oversized artifact reads before prepared invocation.
- [x] Use the same resolved policy in root and child composition.
- [x] Explain output policy alongside existing request-category diagnostics.
- [x] Add many-exchange, restart, ownership and source-fidelity regression tests.
- [x] Complete full workspace tests, format check and strict Clippy.

## Verified source and checks

Tested source: `5b0c3f437781087868288442840332c81c0f08c6`.
Environment: Ubuntu 24.04, Rust 1.88.0.

- `cargo test --locked --workspace --no-fail-fast`: 1602 passed, 0 failed,
  6 marked ignored.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed.

Evidence: https://github.com/ForgeAILab/smith/actions/runs/35026543815
The artifacts record the actual tested commit separately from the trigger commit;
the isolated workflow applies and commits its integration before testing. Its
source-transfer script and workflow are absent from the final source tree.

The production regression exercises eighteen medium-sized shell outcomes in one
user task, exact source reconstruction, protected-session reopening, dedicated
`artifact-read` capability discovery, and bounded artifact retrieval without
replaying the shell effects. This is a deterministic mechanical fixture, not a
live-provider token/cost or semantic-compaction benchmark. The regular PR CI
matrix is separate from the successful isolated Linux run above.

## Explicitly separate follow-on scope

- [ ] PR B: one durable semantic-history path with actual pressure triggers,
      bounded model inputs/accounting, and safe mid-task atomic cutovers.
- [ ] PR C: supported provider-native optimization and a shared `/compact` action.

Do not mark either follow-on complete on the strength of output-offloading tests.
