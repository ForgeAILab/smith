---
created_at: 2026-10-23T23:30:00Z
updated_at: 2026-10-23T23:30:00Z
completed_at:
---

## 1. Runtime

- [x] 1.1 Land runtime change `update-tool-names-and-discovery-2026-10-23` on
  the 0.7 line (`717c4d3` on `fix/capability-discovery-2026-10-23`) and move
  the six pins to it.

## 2. Smith

- [x] 2.1 Uncurated tools get a name-unique affordance.
- [x] 2.2 Activatable-tools fragment for root and child sessions.
- [x] 2.3 MCP and sandboxed tool collisions compare provider names; the CLI
  bridge uses the shared normalizer.
- [x] 2.4 The ChatGPT provider sends readable provider names without hashes.

## 3. Tests

- [x] 3.1 `web_fetch` and `web_search` with equal permissions both bind.
- [x] 3.2 The index omits pinned core tools and denied tools.
- [x] 3.3 ChatGPT names: readable alias, colliding or empty names rejected.
- [x] 3.4 Fixtures regenerated and reviewed (+45 prefix tokens for the new
  fragment; `empty-current-answer` and `goal-complete` no longer bind a
  reference skill on one shared word).

## 4. Verification

- [x] 4.1 `cargo fmt --all --check`
- [x] 4.2 `cargo clippy --workspace --all-targets --locked -- -D warnings`
  (stable and 1.88)
- [x] 4.3 `cargo test --workspace --locked --no-fail-fast` (3,267 passed)
