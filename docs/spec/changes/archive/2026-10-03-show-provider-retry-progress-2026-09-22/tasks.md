---
created_at: 2026-09-22T00:50:37-04:00
updated_at: 2026-09-22T01:28:56-04:00
completed_at: 2026-09-22T01:28:56-04:00
---

# Tasks: Show provider retry progress

## 1. Agent Runtime contract

- [x] 1.1 Add serde-compatible optional attempt index, maximum-attempt count,
      and admitted retry delay to `ProviderAttemptFinished`.
- [x] 1.2 Compute the retry metadata from the provider loop's actual policy,
      `Retry-After`, and remaining turn deadline before emitting the finish
      event; preserve cancellable wait behavior.
- [x] 1.3 Add conformance tests for exponential backoff, provider-directed
      delay, immediate credential recovery, exhausted attempts, deadline
      refusal, and cancellation during backoff.

## 2. Smith client projection

- [x] 2.1 Update Smith's exact Agent Runtime revision after the upstream
      contract and consumer gate pass.
- [x] 2.2 Project the optional retry metadata through `SmithEventKind` and add
      serialization tests for both current and legacy field-absent payloads.
- [x] 2.3 Confirm stream JSON remains version-compatible and headless terminal
      error selection still uses the last provider cause.

## 3. TUI state and transcript

- [x] 3.1 Add bounded root-only retry presentation state and fold scheduled,
      started, completed, cancelled, new-turn, and shutdown transitions.
- [x] 3.2 Replace classification-based `attempt failed, retrying` notices with
      schedule-based `retrying x/x in <delay>` notices.
- [x] 3.3 Render one terminal attributed `failed after x/x attempts` provider
      cause when a retryable error has no admitted next attempt, without
      duplicating non-retryable runtime errors.
- [x] 3.4 Preserve generic compatibility wording when old events lack exact
      attempt metadata; never fabricate a denominator or delay.

## 4. Live progress rendering

- [x] 4.1 Render `Retrying x/x...` during backoff with a positive rounded-up
      remaining wait, then retain the label with the existing provider phase
      while the next attempt runs.
- [x] 4.2 Add reducer and render snapshots for first attempt, backoff, active
      retry, successful retry, exhausted retry, cancellation, reduced motion,
      narrow terminals, and legacy events.
- [x] 4.3 Update `DESIGN.md` retry-state grammar before the TUI implementation
      uses it.

## 5. Validation

- [x] 5.1 Run Agent Runtime provider-loop and Smith consumer conformance tests.
- [x] 5.2 Run `cargo fmt --check`, workspace Clippy with warnings denied,
      focused `smith-runtime`/`smith-tui`/`smith-cli` tests, and workspace tests.
- [x] 5.3 Reproduce a deterministic 503 sequence with a fake provider and
      capture TUI evidence for backoff, retry attempt, success, and final
      exhaustion; do not spend against a live provider.
