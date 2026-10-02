---
created_at: 2026-09-22T00:50:37-04:00
updated_at: 2026-09-22T00:50:37-04:00
---

# Proposal: Show provider retry progress

## Why

Smith currently receives a failed provider attempt with only a `retryable`
classification. The TUI immediately appends `attempt failed, retrying`, clears
the provider phase, and returns the live row to `Working...`. This is
misleading in both directions: `retryable` does not prove the retry policy
admitted another attempt, while an admitted second or third attempt is not
identified as a retry. The runtime also does not expose the selected backoff
or total attempt budget, so the TUI cannot honestly render `2/3` or a backoff
wait.

The reported `503 Service Unavailable` is a real upstream/provider-server
failure, not a local validation error. Better progress must preserve that
cause for the final failure while making the live retry state primary.

## What Changes

- Extend Agent Runtime's existing provider-attempt finish event additively
  with the zero-based attempt index, configured attempt limit, and an optional
  runtime-decided retry delay. The delay is present only when policy and the
  remaining turn deadline admit a next attempt; it is the effective delay
  after exponential backoff and any provider `Retry-After` hint.
- Project that metadata through Smith's versioned client event without
  recomputing retry policy in the TUI. Older journals that omit the fields
  remain readable and fall back to the existing generic presentation.
- Track one bounded, presentation-only provider retry state in the root TUI.
  During backoff it renders `Retrying 2/3...` with the remaining backoff;
  after the next attempt starts it retains `Retrying 2/3...` and shows the
  ordinary sending/thinking/responding phase and elapsed time.
- Lead retry notices with the action, attempt position, and delay rather than
  presenting the transient failure as a terminal error. When no retry was
  scheduled and the attempt budget is exhausted, show one final attributed
  provider error such as `failed after 3/3 attempts`.
- Keep retry count, delay, and provider cause visible without color and clear
  the retry state on success, cancellation, turn completion, shutdown, or a
  new turn.

Out of scope: changing retry counts or backoff policy, automatic fallback to a
different provider/model, suppressing the final provider error, probing a
provider's health from the TUI, or inventing attempt totals for old events.

## Impact

- Affected specs: `provider-runtime`, `client-surfaces`
- Affected upstream: Agent Runtime provider-loop event contract and conformance
  tests, followed by a compatible Smith dependency-pin update
- Affected Smith code: `crates/smith-runtime/src/client.rs`,
  `crates/smith-tui/src/app/{state,reducer,conversation}.rs`, transcript
  rendering and focused reducer/render tests; headless compatibility tests
- No configuration, credential, authority, persistence-layout, or retry-policy
  changes

## Approval Boundary

Approval authorizes the additive runtime/client retry metadata, the bounded TUI
state and wording above, the coordinated Agent Runtime pin update, and focused
tests/documentation. It does not authorize changing retry timing, attempt
limits, provider routing, automatic fallback, paid live inference probes, or
unrelated TUI restyling.
