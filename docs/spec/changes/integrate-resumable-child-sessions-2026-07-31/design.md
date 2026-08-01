---
created_at: 2026-08-01T00:56:16Z
updated_at: 2026-08-01T00:56:16Z
---

## Context

The current Smith `agent` tool exposes `spawn`, `list`, `wait`, `result`,
`follow_up`, and `stop`. `follow_up` uses the existing in-memory child handle,
so memory continuity is already correct within one process. The child factory
is intentionally composed without persistence, and host recovery treats any
unresolved child lifecycle as `EphemeralWorkInterruption`. The missing product
work is therefore persistence/rebinding, not another delegation loop.

## Goals

- Let the root agent and user continue with the same child after Smith restart.
- Keep idle-child follow-up distinct from interrupted-task resume.
- Make startup and browsing side-effect free; provider spend happens only on
  an explicit continuation.
- Preserve existing depth-one, approval, trust, workspace, redaction, and
  no-project-metadata boundaries.
- Provide a clear OpenCode-like sense of addressable collaborators while
  retaining Smith's transcript-first TUI.

## Non-Goals

- A daemon that keeps children running after Smith exits.
- Automatic recovery execution, nested delegation, or remote workers.
- Direct child access to root interaction/approval UI.
- Reconstructing exact child state from the redacted event journal.

## Decision 1: Durable by Default Only With the Full Store Set

When a persistent root session has the Smith session store and authenticated
checkpoint store, `spawn` creates a durable child. Agent Runtime stores its
versioned delegation catalog in the parent snapshot's redaction-safe extension
state rather than introducing another storage backend. Child
snapshots/checkpoints use the same owner-only project/session namespace and
protection policy as the root, keyed by the stable child session ID. The parent
catalog contains only bounded identity/authority/status metadata and a
checkpoint watermark.

If any required store is intentionally disabled, Smith composes the existing
ephemeral path and surfaces `durability: ephemeral`. It never implies that
restart follow-up will work.

The existing inline/environment checkpoint-key selection remains sufficient.
When selected, neither child startup nor recovery may query Keychain, Secret
Service, or another interactive credential source.

## Decision 2: Restore Records, Not Running Work

Parent startup loads and validates child records before wiring the coordinator.
It does not instantiate providers for idle children. A child previously marked
running but without a valid live lease appears as `interrupted` with either
`resumable: true` or a bounded incompatibility reason.

```text
parent resume
  → load/validate child catalog
  → acquire parent lifecycle lease
  → render children (no provider spend)
  → follow_up(id, task) for idle child
     or resume(id) for interrupted task
  → lazily rebuild the exact narrowed child runtime
```

`follow_up` always creates a new child turn. `resume` always continues the
checkpointed child turn. Neither operation may substitute a spawn.

Smith's existing exclusive parent-session lifecycle file lease is also the
cross-process child-catalog lease. Agent Runtime rejects a competing
coordinator for the same live parent handle and serializes child binding and
catalog publication within the owner; no second Smith-only lease/store is
needed.

## Decision 3: Smith Rebuilds the Original Narrowed Composition

The persisted record retains fingerprints/revisions for provider/model,
workspace, trust, tool scope, child preset/profile, activation epoch, limits,
and parent ownership. `SmithChildFactory` resolves current availability and
asks Agent Runtime to validate compatibility. It may narrow authority; any
widening, changed canonical workspace, unavailable provider/model, or stale
untrusted profile produces a structured blocked state.

The child still receives no `agent` management tool. Needs-input outcomes
continue to route through the root; a recovered pending child interaction does
not directly open a TUI prompt.

## Decision 4: Tool and Prompt Semantics

The `agent` schema adds `resume { child_id }`. `list` and `result` add bounded
fields for child session identity, durability, lifecycle state, resumability,
task usage, and incompatibility reason. Existing fields remain compatible.

Smith's delegation prompt states:

- use `follow_up` when a suitable idle child already has relevant context;
- use `resume` only for the same interrupted task after explicit user approval
  of renewed provider/tool execution;
- use `spawn` for a genuinely new specialist context;
- never treat an unknown/stale child ID as permission to spawn a replacement.

## Decision 5: Transcript-First Continuation UX

`/agent` and `/timeline` display stable child ID, short role/task label,
durability, model, state, turn usage, and latest bounded result. Existing
previous/next/parent inspection stays read-only and keeps the root composer
focused.

Durable existing children join typed completion as `@child-id` entries,
distinct from spawn presets. Selecting one associates the draft with an exact
follow-up target; interrupted entries offer an explicit `/agent resume
<child-id>` action instead. Confirmation shows model/provider spend,
workspace/tool posture, and whether the operation is a new turn or exact
checkpoint continuation.

Live events and journal replay reduce to the same child state. Headless JSON
adds versioned optional fields/action results and remains redaction-safe.

## Decision 6: Legacy and Retention Behavior

Historical journals may contain child IDs but not protected child snapshots or
catalog records. Smith keeps their existing interrupted timeline evidence and
labels them `legacy_ephemeral`; it does not offer follow-up/resume.

Configurable per-parent retained-child count/age bounds prevent unbounded user
state. Explicit stop or parent session deletion makes a child terminal and
non-resumable. Cleanup is user-state-only and must not remove project files or
child-produced task artifacts outside the session-private artifact store.

## Validation Strategy

- Deterministic fake-provider tests inspect the actual child provider request
  after restart and confirm prior child history and identity.
- Crash fixtures cover every checkpoint boundary and prove no repeated model,
  approval, interaction, or tool work.
- TUI snapshot/replay tests cover normal and narrow terminals, colorless mode,
  idle/interrupted/legacy states, and draft preservation.
- A disposable live Z.AI Coding Plan scenario completes a review child, exits,
  resumes the root, sends a follow-up to that same child, and verifies a second
  scenario's explicit interrupted-task resume.
