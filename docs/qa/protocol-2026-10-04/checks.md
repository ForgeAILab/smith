# Client protocol and boundaries — checks

`refactor-client-protocol`, branch `refactor/client-protocol`, 2026-10-04.

- Gate with the macOS runner's temp path: fmt, strict Clippy (with
  `unreachable_pub` denied for smith-runtime), 2,011 workspace tests,
  `cargo deny --locked check all`. Local-command and headless fixtures
  unchanged.
- The projection test matches all 52 variants of the pinned runtime's
  `RuntimeEvent` exhaustively. Two were missing before this change
  (`BudgetFailure`, `RateLimitObservation`) and projected to `Unknown`.
- `../grammar-2026-10-03/final_checks.py`: 67/67.

## Cache, 0.3.3 against 0.3.4

`../grammar-2026-10-03/cache_ab.py` ([results](cache-ab.json)). Gemini matched
turn for turn. Z.AI's 0.3.3 run read the file more in turn 1. xAI turn 2 was
served 192 cached tokens on 0.3.3 and 2,496 on 0.3.4 — the provider variance
recorded in the 0.3.0 and 0.3.3 runs, this time on the older build. No miss
or re-billed tokens on any turn.
