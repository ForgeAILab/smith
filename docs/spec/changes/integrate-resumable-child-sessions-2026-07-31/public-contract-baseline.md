# Public Contract Baseline

Captured at Smith commit `2aa1fa8` before durable-child implementation.

- The `agent` tool schema exposes `spawn`, `list`, `wait`, `result`,
  `follow_up`, and `stop`; there is no `resume` action.
- `SmithChildFactory` reconstructs the narrowed provider/model/tool/workspace
  composition but carries no session or checkpoint stores.
- `wire_delegation` synchronously creates a fresh empty coordinator after the
  root session starts.
- In-process `follow_up` reaches the same `ChildId` and session handle.
- Host recovery scans journal lifecycle events and records unresolved children
  as `EphemeralWorkInterruption`; completed/needs-input child entries are also
  lost because coordinator state is process-owned.
- `/agent` and `/timeline` project event-derived child history, while the
  machine-facing tool result has no durability, child-session, resumability,
  or incompatibility fields.

The approved change rebases the earlier Smith harness and stable-session
changes: one factory, depth-one delegation, protected checkpoints,
transcript-first presentation, no-prompt configured checkpoint keys, and no
project-local control metadata remain mandatory.
