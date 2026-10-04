---
created_at: 2026-08-17T00:00:00Z
updated_at: 2026-08-17T00:00:00Z
---

## Why

When a turn ended without a durable terminal boundary (a failed protected
write during terminal publication, a crash, or a checkpoint deliberately left
dormant), the pinned runtime revision rejected every later submission with
`Conflict: cannot accept a new turn over a non-terminal checkpoint`. The
session wedged in-process: the only recovery was restarting Smith, and the
restart path resumes the interrupted turn, re-running its provider/tool work.

## What Changes

- Smith consumes the runtime's new admission reconciliation (runtime change
  `fix(session): finalize interrupted turns on admission instead of wedging`):
  submitting a new turn over a stale non-terminal checkpoint finalizes the
  interrupted turn as an explicit `Failed` terminal without replaying it,
  then accepts the new turn over the continued checkpoint watermark.
- Smith's own checkpoint store already accepts the resulting chain (an
  ordinary same-turn successor to `Terminal`, then a new accepted turn), so
  no Smith code changes are required.
- The Smith-visible event stream gains one attributed runtime error plus
  `TurnCompleted { Failed }` for the interrupted turn, before the new turn
  starts.

## Impact

- Depends on a runtime revision containing
  `SessionInner::reconcile_interrupted_checkpoint` (agent-runtime commit
  `e3bad15` on `main`, cherry-picked to the pinned lineage as
  `smith-interrupted-turn-fix`). Until that revision is pinned in the root
  manifest, local development uses the git-ignored `.cargo/config.toml`
  patch table; the manifest bump lands together with the upstream runtime
  release.
- No changes to Smith's stores, journal reconciliation, or TUI: the
  finalized chain uses only pre-existing transitions and events.
