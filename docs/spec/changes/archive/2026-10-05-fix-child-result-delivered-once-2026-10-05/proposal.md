---
created_at: 2026-10-05T05:00:00Z
updated_at: 2026-10-05T07:20:00Z
---

## Why

A main agent that spawned a child, waited for it with the agent tool, and
answered from the result got the same result again from the automatic
`delegation.child-completion` turn, and answered twice. The session event log
showed the model noticing ("I already replied 'pong'"). Every task that waits
for a child paid for an extra provider call.

## What Changes

- The agent tool acknowledges the child outcome it returns from `wait` (a
  completed result whose text the status carries) and `result`. Agent Runtime
  (`fc92efb`, change `update-tool-delivered-child-outcomes`) withdraws it from
  automatic delivery once that tool result commits.
- Headless runs stop treating every child completion as a pending delivery.
  They track the child-outcome cursor revision, which advances once per
  admitted delivery turn, and read ready and running state from the live
  coordinator. Without this, a headless run whose model read the result
  waited forever for a delivery turn that no longer comes.
- The agent tool description says a result already returned by `wait` or
  `result` is not delivered again.

## Impact

- Affected specs: child-agents.
- Affected code: `smith-runtime/src/delegation.rs`,
  `smith-cli/src/headless/{fold,run_flow}.rs`, tests in `smith-runtime` and
  `smith-cli`. Shipped in v0.3.8.
