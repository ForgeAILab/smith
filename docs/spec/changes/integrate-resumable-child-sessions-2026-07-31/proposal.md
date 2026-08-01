---
created_at: 2026-08-01T00:56:16Z
updated_at: 2026-08-01T01:08:11Z
---

## Why

Smith can already send several follow-up tasks to the same child while one
process owns its `DelegationCoordinator`. That child remembers its prior turns
and keeps one stable child ID. On restart, however, Smith reconstructs only the
root session, records every unresolved child as ephemeral interrupted work,
and wires an empty coordinator. Even a child that completed successfully is no
longer addressable for another follow-up.

This makes delegation feel like dispatching disposable workers instead of
collaborating with a persistent specialist. Smith should let the root and user
return to the same child session, with the same history and bounded authority,
after Smith restarts. Recovery must remain explicit: a task that was in flight
may be resumed from its checkpoint, but it must never begin running merely
because the TUI reopened.

## What Changes

- Adopt Agent Runtime's durable child-session catalog, recovery states,
  execution leases, and explicit `resume` operation through the one Smith
  factory and delegation coordinator.
- Make children durable by default when the parent session has Smith's normal
  persistent snapshot and protected checkpoint stores. Keep no-persistence
  runs explicitly labelled ephemeral.
- Preserve each child's stable child/session IDs, canonical history, manifests,
  usage, cumulative limits, model/provider selection, tool scope, workspace
  posture, profile/activation revisions, latest outcome, and artifact lineage.
- Restore child records when the parent resumes, but lazily construct runtimes.
  Idle children accept `follow_up`; interrupted children require an explicit
  `resume`; neither path may silently call `spawn`.
- Extend the `agent` tool with `resume` and durable/resumable status. Teach the
  root policy to prefer `follow_up` for an existing suitable child and to ask
  for explicit confirmation before resuming interrupted provider/tool work.
- Extend `/agent`, `@` completion, timeline, replay, and headless projections
  so a user can identify the same child, see whether it is idle/interrupted/
  resumable/ephemeral, and direct a continuation without losing root-composer
  state.
- Store child-control metadata only in owner-scoped Smith state. Reuse the
  already configured inline/environment protected-checkpoint key and never
  query Keychain or another credential service when that no-prompt source is
  selected.
- Migrate legacy journal-only children honestly as non-resumable ephemeral
  records; do not invent history or bind an old ID to a new child.
- Add deterministic and live Z.AI Coding Plan scenarios that complete a child
  review, restart Smith, follow up the same child, and explicitly resume an
  interrupted child without repeated side effects.

## Impact

- Affected specs: `child-agents`, `session-recovery`, `client-interaction`,
  `runtime-integration`
- Affected code: `smith-runtime` delegation/factory/host/persistence,
  `smith-cli` machine surfaces, `smith-tui` reducer/navigation/completion, and
  product evaluation fixtures
- Public compatibility: additive `agent` action and status fields, versioned
  child lifecycle events, recovery records, and machine-output fixtures
- Security: same-parent ownership, exact protected checkpoints, policy
  revalidation, provider-spend confirmation, and no cross-project adoption
- Persistence: owner-only user state only; no child metadata enters the project
  checkout and no plaintext checkpoint fallback is introduced
- Runtime dependency: coordinated
  `../agent-runtime/docs/spec/changes/add-resumable-child-sessions-2026-07-31/`

## Active Change Coordination

- `add-smith-agent-harness-2026-07-23` owns the current one-level child tool and
  factory. Its completed process-ephemeral behavior must be archived or
  explicitly rebased before this change replaces that clause.
- `integrate-stable-session-harness-2026-07-31` remains authoritative for
  protected exact-state recovery, no-repeat side effects, one composition path,
  and root-only child interaction. This proposal reuses those mechanisms.
- `add-agent-first-workflow-ux-2026-07-31` remains authoritative for
  transcript-first presentation, child timeline navigation, no-prompt
  checkpoint keys, and project cleanliness. Durable child controls extend
  those surfaces without adding a permanent pane.

## Delivery Slices

1. Land the coordinated runtime contracts and deterministic consumer fixtures.
2. Add Smith's protected child catalog storage and parent-resume wiring.
3. Add exact `follow_up`/`resume` tool behavior, policy, and migration handling.
4. Add TUI/headless status and explicit continuation UX.
5. Run the full workspace gates and a disposable live restart benchmark on the
   configured Z.AI Coding Plan model.

## Approval Boundary

Approval authorizes Stage 2 implementation in Smith only after the coordinated
Agent Runtime proposal is also approved. It does not authorize publishing,
nested agents, automatic background restart, new credential-service access,
or project-local control metadata.
