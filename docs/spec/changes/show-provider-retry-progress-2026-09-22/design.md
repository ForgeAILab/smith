---
created_at: 2026-09-22T00:50:37-04:00
updated_at: 2026-09-22T00:50:37-04:00
---

# Design: Show provider retry progress

## Context

The provider loop emits `ProviderAttemptStarted { index, ... }` and later
`ProviderAttemptFinished { retryable, error, ... }`. `retryable` is error
classification, not the loop's retry decision. The loop decides afterward
whether the configured attempt budget and turn deadline allow another attempt,
then computes backoff (default 200 ms, doubling to the cap, with provider
`Retry-After` taking precedence) and sleeps before starting it.

Smith v0.2.10 now preserves the attempt error, but the missing decision data
forces the TUI to say `retrying` for an exhausted final attempt and gives it no
honest source for an attempt denominator or backoff.

## Goals / Non-Goals

- Goals:
  - Make an admitted retry unmistakable while it is waiting and running.
  - Render an exact one-based attempt position and configured total.
  - Render the runtime-selected backoff without duplicating retry policy.
  - Preserve the final redaction-safe provider cause after exhaustion.
  - Keep old event/journal payloads readable.
- Non-Goals:
  - Change retry classification, delays, limits, or cancellation behavior.
  - Add provider failover or health polling.
  - Reclassify a third-party proxy failure as a first-party service failure.

## Decisions

### Runtime owns the retry decision

Agent Runtime will add serde-defaulted optional metadata to the existing
`ProviderAttemptFinished` payload:

- `index`: zero-based index of the finished attempt;
- `max_attempts`: the configured total including the first attempt; and
- `retry_delay_ms`: the effective wait before the next attempt, present only
  when a next attempt has actually been admitted. `Some(0)` means an immediate
  retry and is distinct from `None`.

The loop computes this metadata after error classification and policy/deadline
admission, before publishing the finish event and beginning the cancellable
wait. A final retryable error with an exhausted attempt budget has
`retry_delay_ms: None`; clients therefore never infer a retry from
`retryable: true`.

Adding optional fields to the existing event is preferred over a new event
variant: tolerant serialized consumers can ignore the fields, older journals
deserialize them as absent, and the finish/error remain one causally atomic
observation. Smith's client projection carries the same option semantics.

### The TUI keeps bounded live retry state

On a finish event with a scheduled retry, the root reducer records:

- next one-based attempt number (`index + 2`), bounded by `max_attempts`;
- total attempts;
- selected delay and a local monotonic receipt instant; and
- the redaction-safe provider cause for the notice/final fallback only.

While the delay remains, the live row is:

```text
Retrying 2/3... · 1m 03s · backoff <1s
```

When the matching next `ProviderAttemptStarted` arrives, the same identity is
retained and the phase becomes the existing direction/timing grammar:

```text
Retrying 2/3... · 1m 03s · ↑ 19s
```

The backoff remainder is display-only, rounded up so a positive wait never
renders as `0s`. Event transport lag can only shorten the real wait; the next
attempt-start event is authoritative and clears the backoff immediately.

The initial attempt remains `Working...`. Successful completion, cancellation,
a new turn, shutdown, or any terminal turn event clears retry state.

### Transient and terminal wording are distinct

A scheduled retry appends one informational notice led by the action:

```text
provider · retrying 2/3 in 200ms: Server: ...
```

It is not rendered with the terminal error marker. If a retryable attempt has
no schedule because the attempt budget or deadline is exhausted, the final
provider cause is rendered once with terminal wording:

```text
provider · failed after 3/3 attempts: Server: ...
```

Non-retryable failures continue through the existing runtime error path so the
TUI does not duplicate them. If optional position metadata is absent (old
journal/runtime), Smith keeps the current bounded generic notice rather than
fabricating `x/x`.

## Risks / Trade-offs

- This is a coordinated Agent Runtime and Smith change. Smith cannot land the
  exact UI against a runtime revision that does not expose the schedule.
- A local countdown starts when the client receives the event, so a severely
  lagged subscriber can briefly overstate remaining backoff. The authoritative
  next-start/terminal event always clears it; no execution decision depends on
  the countdown.
- The default 200/400 ms backoffs may be visible for only a frame or two. The
  retained `Retrying 2/3` label during the next in-flight attempt provides the
  durable evidence the user needs.

## Migration Plan

1. Land the additive fields and provider-loop conformance tests in Agent
   Runtime.
2. Update Smith's exact Agent Runtime revision and project the optional fields.
3. Add the TUI reducer/render behavior and compatibility fixtures for payloads
   without the new fields.
4. Run runtime consumer conformance plus Smith formatting, Clippy, focused
   tests, and workspace tests.

## Open Questions

None. The requested `x/x` presentation requires the configured attempt total,
and the shared-runtime ownership rule determines where the schedule originates.
