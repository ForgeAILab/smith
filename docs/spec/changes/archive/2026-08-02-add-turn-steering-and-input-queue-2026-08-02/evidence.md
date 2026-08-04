# Verification Evidence

Verified on 2026-08-02 against the coordinated local Agent Runtime working
tree.

## Smith gates

- `cargo fmt --all -- --check` — passed after final formatting.
- `cargo test --workspace --all-targets` — passed; the configured live-provider test remained intentionally ignored because it spends external quota.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo test -p smith-tui --lib` — 236 passed.
- `cargo test -p smith-tui --test end_to_end` — 4 passed.
- `cargo test -p smith-cli --bin smith` — 62 passed.
- `cargo test -p smith-runtime --test persistence` — 17 passed, including the updated steer identity floor fixture.
- Coordinated Agent Runtime workspace, v11 schema, active-turn steering,
  testkit consumer, strict Clippy, and Rust 1.86 gates passed.

Coverage includes FIFO steer/queue routing, stale/no-active fallback logic,
commit-driven exactly-once transcript rows, interrupt discard/resend behavior,
lossless paste restoration, dispatch-time workspace-bounded file reads, bounded
pending rendering with todo coexistence, and user-priority goal admission.

## Runtime pin

The committed Smith manifests name the verified steering implementation at
runtime revision `b24cc1bec22ffca106591feee9eb4f5bb2a9a9d3`. The repository's
ignored sibling-path patch can continue to support coordinated local
development without weakening that immutable production pin.
