# refactor-client-structure — fixture baseline, 2026-10-02

Task 1.2. The recorder was written by Codex (one dispatch, three fix
passes); builds, recordings, and comparisons were run by the orchestrator.
Nothing is committed.

## Baseline

- Commit `58bc098`: release v0.2.15 (`ed253a8`) plus docs-only commits.
- Worktree `../tui-refactor-client`, branch `refactor/client-structure`.
  The main checkout's uncommitted `fix-interaction-defects`, retry, and
  idle-compaction work is not in it.

## What is recorded

425 files under `crates/smith-cli/tests/fixtures/`:

- `local-commands/` — 112 cases, each as the raw result (title, state, body)
  and as the rendered transcript at 100 and 44 columns (336 files).
- `headless/` — 89 files: eight flow scenarios and the output-format cases,
  as text, JSON, and stream JSON where the scenario produces them.

Not covered: connected or failed MCP transports, interrupted or resumable
child checkpoints, and successful connect or reconfigure actions (they
return no local result).

| Action | Command |
| --- | --- |
| Record | `SMITH_UPDATE_FIXTURES=1 cargo test -p smith-cli --locked --bin smith fixtures_` |
| Compare | `cargo test -p smith-cli --locked --bin smith fixtures_` |

A missing fixture fails in compare mode. Recording does not delete fixtures
whose case was removed.

## Verification

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |
| Workspace tests in the worktree, internal `TMPDIR` | 1,704 passed, 0 failed, 6 ignored |
| Three recordings, diffed | byte-identical |
| Local-command fixtures recorded with `TMPDIR` on the data volume and on the internal disk | byte-identical |
| Compare mode, internal `TMPDIR`, 30 runs | 30 of 30, about 6 s each |
| Compare mode, default `TMPDIR`, 3 runs | 3 of 3, 18 to 33 s each |

All 22 recorder tests run on the default test-thread stack.

## What the normalizer masks

Token counts, costs, percentages, labels, key order, and event sequence
numbers on `runtime_event` lines are verbatim. These are replaced:

| Placeholder | Replaces |
| --- | --- |
| `<PROJECT>`, `<HOME>`, `<PROJECT_ID>` | Temporary paths and the identity derived from them |
| `<SESSION_ID>`, `<CHILD_SESSION_ID:n>` | Random session ids |
| `<PROFILE_REVISION>` | Path-dependent profile revision |
| `<FINGERPRINT:n>`, `<ARTIFACT_ID:n>` | Path-dependent plan, registry, cache, and preparation fingerprints; artifact ids |
| `<INTERACTION_ID:n>`, `<APPROVAL_FINGERPRINT:n>` | Random per-run request ids (headless flows) |
| `<DURATION>`, `<TIMESTAMP>` | Elapsed and latency values; local-time renderings of the fixed clock |
| `<SHUTDOWN_SEQ>`, `<SHUTDOWN_INTERVAL_ID>`, `<SHUTDOWN_RECENT_TURNS>` | The shutdown race below (headless flows only) |

Limit: a fingerprint that the refactor changed would still compare equal,
because fingerprints are numbered by first appearance, not by value.

## Findings outside this change

Neither is fixed here; the fixtures record current behaviour.

1. **The headless result is not deterministic.** For identical input the
   terminal result and the `cache_controller` stream line come out in one of
   two variants. `resume_capsule.retained_recent_turns` is empty or holds the
   turn; the idle-compaction interval id is absent or present;
   `last_event_sequence`, `exact_state.watermark`, and
   `last_persisted_watermark` differ by one. Headless starts shutdown at
   `crates/smith-cli/src/headless/run_flow.rs:321`; the cache worker's
   `select!` at `crates/smith-runtime/src/cache_controller.rs:630` can take
   cancellation before reducing the turn's last event; the snapshots are read
   at `run_flow.rs:349`. Observed in every flow that reaches shutdown, about
   5 runs in 12 for one scenario. Located by Codex; not independently traced.
2. **`/timeline` fails after a goal turn.** In the scripted scenario
   (`timeline-turn`) it returns `timeline unavailable: Serialization: journal
   … line 8 is not a readable record: invalid type: string "[redacted]",
   expected u64`. Not checked outside the fixture scenario.
