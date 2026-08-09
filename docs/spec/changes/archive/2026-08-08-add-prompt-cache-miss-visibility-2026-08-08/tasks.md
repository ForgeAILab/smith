---
created_at: 2026-08-08T22:32:53Z
updated_at: 2026-08-09T01:20:01Z
completed_at: 2026-08-09T01:20:01Z
---

## 1. Runtime dependency and adapter compatibility

- [x] 1.1 Update Smith's Agent Runtime dependency to the approved
  `add-prompt-cache-miss-evidence-2026-08-08` revision and migrate every
  exhaustive `RuntimeEvent`/`ProviderStreamEvent` match.

  _Pinned to published git revision
  `0a07231649d81ccb40f2395a9924f8bd6027baf9`; the complete workspace and
  Clippy gates pass with the repository-local path patch excluded._
- [x] 1.2 In `crates/smith-runtime/src/chatgpt.rs`, preserve independent
  presence of `cached_tokens` and `cache_write_tokens`, including explicit
  zero, in the shared presence-aware observation.
- [x] 1.3 Add ChatGPT adapter tests for explicit zero, omitted fields, positive
  read, positive write, and disjoint uncached/cached/write accounting.

## 2. Canonical Smith cache projection

- [x] 2.1 Add a focused cache projection module/type keyed by turn, request,
  attempt, and cache-plan fingerprint rather than extending ad-hoc positive
  cache counters.
- [x] 2.2 Fold `ProviderAttemptStarted`, `Usage`,
  `CacheStateChanged`, `ProviderAttemptFinished`, and `TurnCompleted` into
  per-attempt, per-turn, and session summaries.
- [x] 2.3 Compute `CH` from all billed root attempts in the completed turn and
  distinguish explicit `0%` from unknown `?`.
- [x] 2.4 Sum canonical missed tokens and miss count separately from
  `CounterKind` totals, retaining expectation confidence and failure
  attribution.
- [x] 2.5 Derive optional incremental cost from the serving attempt's versioned
  price and paid non-read categories; keep it unknown when any required input
  is unavailable.
- [x] 2.6 Add reducer tests for first request, full hit, explicit-zero miss,
  omitted evidence, partial miss, write observation, model switch, tool
  continuation, failed retry, and read-greater-than-expectation.

## 3. Interactive surfaces

- [x] 3.1 Replace cumulative positive-read footer semantics with the latest
  completed root turn's `CH` percentage/zero/unknown projection.
- [x] 3.2 Extend `/status` and the exit/session usage summary with canonical
  cache state, expected/observed/missed values, cumulative miss count, and
  re-billed tokens without adding them to total token usage.
- [x] 3.3 Add one local transcript notice per significant completed root turn,
  aggregating retries/tool-loop requests and respecting the 20,000-token or
  $0.10-derived-cost threshold.
- [x] 3.4 Derive factual logical-request idle context from event-envelope
  timestamps; render rounded minutes only at one minute or more and never
  infer expiry.
- [x] 3.5 Ensure cache notices remain local blocks, are absent from canonical
  model history, and reconstruct once without duplication during replay.
- [x] 3.6 Preserve the existing provider/model switch notice, clear the old
  `CH` projection, and prove a new cache identity does not add re-billed
  tokens.

## 4. Configuration

- [x] 4.1 Add layered Boolean `cache.miss_notices` to `smith-config` with a
  default of `false` and ordinary source provenance/explain output.
- [x] 4.2 Thread the resolved setting to both interactive and headless
  presentation without changing Agent Runtime mechanism or provider requests.
- [x] 4.3 Add precedence/default tests and document that the fixed
  significance thresholds are not configuration keys in this change.

## 5. Headless and machine output

- [x] 5.1 Extend final JSON `cache` output additively with final-turn state,
  expected/observed/missed tokens, `CH`, confidence, and optional derived cost
  while retaining existing last-plan fields.
- [x] 5.2 Confirm `stream-json` serializes attributed
  `cache_observation` and `cache_state_changed` events in causal order.
- [x] 5.3 In text mode, keep stdout answer-only and emit an enabled significant
  notice on stderr.
- [x] 5.4 Add a deterministic fixture proving TUI, final JSON, stream JSON, and
  text diagnostics derive equivalent state and missed-token totals.

## 6. Persistence and compatibility

- [x] 6.1 Teach live and replay reducers to produce byte-equivalent cache
  projections from the same canonical event sequence.
- [x] 6.2 Add optional/defaulted cache miss count and re-billed-token fields to
  the bounded usage-log/session-summary record and bump its schema version.
- [x] 6.3 Add fixtures proving older usage records and legacy unattributed
  cache observations load without fabricating zero evidence or a miss.
- [x] 6.4 Add resume coverage proving cumulative cache diagnostics survive
  restart while local notices do not duplicate.

## 7. Documentation and verification

- [x] 7.1 Document `cache.miss_notices`, `CH`, `/status` cache fields,
  machine-output fields, significance thresholds, and evidence wording.
- [x] 7.2 Document explicitly that `after Nm idle` is correlation and that
  Smith has no generic pre-request cache-alive probe.
- [x] 7.3 `cargo fmt --all --check`.
- [x] 7.4 `cargo clippy --workspace --all-targets -- -D warnings`.
- [x] 7.5 `cargo test --workspace`.
- [x] 7.6 Run one opt-in OpenAI/Anthropic live check when credentials are
  available, asserting only field/state consistency and never a TTL.

  _A bounded ChatGPT/OpenAI tool-continuation check passed on 2026-08-09.
  The first request reported an explicit zero and `eligible`; the continuation
  reported 6,656 cached tokens against an 8,158-token expectation, and both
  the canonical event and final JSON reported a 1,502-token observed miss.
  The check made no TTL or expiry assertion._
- [x] 7.7 Re-run strict spec validation for
  `add-prompt-cache-miss-visibility-2026-08-08`.
