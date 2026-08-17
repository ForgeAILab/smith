---
created_at: 2026-08-17T20:54:25Z
updated_at: 2026-08-17T21:08:00Z
---

## Why

`agent.wait` can keep the parent model turn occupied while a direct child is
still working. A wait timeout must be a foreground convenience boundary, not
a child lifetime limit: after the boundary the child should remain active and
its terminal result should still be delivered through the existing protected
completion path. The current five-second default and thirty-second ceiling are
shorter than the cache-retention window this host wants to use for a foreground
wait, while an unbounded wait would prevent the parent from parking.

## What Changes

- Make the default and maximum model-facing foreground wait five minutes
  (`300_000` milliseconds).
- Keep the existing `agent.wait.timeout_ms` override and immediate zero-time
  status check, with the same source-explainable child-agent configuration.
- Return a successful running result with an explicit timeout marker when the
  foreground wait expires; never cancel, stop, expire, or otherwise limit the
  child because the wait ended.
- Split a long Smith wait into bounded Agent Runtime wait slices so the
  Smith-side five-minute policy remains compatible with the pinned runtime's
  per-call wait cap.
- Preserve automatic terminal child delivery and parent parking after the
  foreground tool call reaches a safe boundary.
- Update child-agent/configuration specifications, reference documentation, and
  focused tests.

## Impact

The affected behavior is the root-only model-facing `agent.wait` operation and
its `profiles.<name>.child_agents.wait_*` defaults/ranges. Child execution,
child turn/token limits, stop behavior, persistence, and terminal outcome
admission are unchanged. A five-minute wait is a soft foreground boundary:
completion before the boundary is returned normally; otherwise the parent is
released while the child continues in the background.

The pinned Agent Runtime remains unchanged. Smith invokes its existing bounded
wait API in slices no longer than the upstream cap and does not claim that the
child itself has expired.

## Approval Boundary

Approval authorizes implementation of this Smith-side foreground wait policy
and its documentation/tests only. It does not authorize changing child
lifetime limits, cancellation semantics, provider cache guarantees, or the
pinned Agent Runtime dependency.
