---
created_at: 2026-08-02T15:43:22Z
updated_at: 2026-08-02T15:54:07Z
---

## Why
Smith already exposes a bounded, revisioned, sensitivity-aware memory
contributor, but the standard host installs no durable source and users must
repeat useful cross-session context. Smith needs a human-auditable project
memory that preserves the existing context, authority, and persistence
boundaries.

## What Changes
- Enable project-scoped memory by default for standard persistent hosts, rooted
  at ~/.smith/memory/<project-id>/ and isolated with the same canonical
  project identity used by session persistence.
- Store a host-managed MEMORY.md index plus bounded Markdown topic files with
  versioned typed frontmatter for user, feedback, project, and reference
  memories.
- Replace the standard host's empty memory slot with a file-backed
  MemorySource that snapshots the store at turn boundaries and contributes a
  bounded index plus deterministically ranked records through Agent Runtime's
  existing memory lane.
- Add dedicated memory.remember, memory.forget, and memory.search tools rather
  than granting ordinary workspace tools access to Smith's user-state root.
- Keep capture visible and explicit by default: the active agent may call the
  memory tools, while hidden post-turn model extraction remains disabled
  unless the user opts in.
- Add an opt-in, spend-bounded post-turn capture coordinator that may upsert
  memories after successful root turns but may never delete memory or change
  the outcome of the completed turn.
- Run default-on deterministic maintenance when the store opens and after
  mutations: validate files, recover incomplete writes, enforce bounds, and
  regenerate MEMORY.md without semantically rewriting user content.
- Use private directories and files, same-directory atomic publication,
  cross-process mutation locking, path and symlink containment, secret
  rejection, sensitivity-aware context, and content-free audit events.
- Defer vector search, model-assisted recall ranking, global or team memory,
  remote synchronization, and Dream-style semantic consolidation to separate
  follow-on changes.

## Impact
- Affected specs: new memory-storage, memory-recall, memory-controls, and
  memory-automation capabilities; this realizes the existing
  integrate-stable-session-harness memory-policy contract without changing
  Agent Runtime's generic MemorySource API.
- Affected code: smith-config resolution and user editing; smith-runtime
  memory, host, factory, prompt, event, and lifecycle composition; smith-tools
  ability registration; smith-cli and smith-tui status or tool projections;
  persistence, security, and integration tests; DESIGN.md and user
  documentation.
- Default behavior: memory mechanics and deterministic maintenance are enabled
  for an empty project store, while automatic model capture and its additional
  provider spend remain off until explicitly enabled.
- Security: repository configuration cannot relocate the store, enable
  automatic capture, lower sensitivity, grant authority, or bypass the normal
  ability and approval pipeline. Memory content never enters audit metadata.
- Persistence: memory is durable user state separate from session snapshots,
  checkpoints, journals, project instructions, and canonical conversation
  history.

## Existing Change Coordination

- integrate-stable-session-harness-2026-07-31 remains authoritative for the
  generic bounded memory contributor and for the rule that memory is not
  canonical history or tool authority. This change supplies the Smith-owned
  storage, retrieval, retention, and user controls that contract reserved.
- add-project-instructions-and-quiet-turns-2026-08-01 remains authoritative
  for root AGENTS.md as required developer instruction. Project memory stays
  optional evidence-bearing context and cannot override instructions.
- Existing session, checkpoint, semantic-summary, goal, child, and turn
  steering changes remain authoritative for their state machines. This change
  adds no second session transcript or resumability mechanism.

## Delivery Slices

1. Resolve default-on memory policy and build the private project store,
   versioned topic schema, generated index, locking, atomic publication, and
   deterministic repair.
2. Install the file-backed source and deterministic ranking at the standard
   host boundary with context provenance, sensitivity, and bounds intact.
3. Register remember, forget, and search through the ordinary ability,
   approval, event, and client projection paths.
4. Add the opt-in post-turn capture coordinator with separate usage
   attribution, bounded shutdown draining, and failure isolation.
5. Complete security, concurrency, corruption, cross-session, disablement,
   replay, and client-parity tests plus product documentation.

## Approval Boundary

Approval authorizes Stage 2 implementation in this repository only. It does
not authorize Agent Runtime API changes, a vector or embedding store,
model-assisted recall ranking, automatic semantic deletion, global or team
memory, remote sync, Dream-style consolidation, or importing Claude Code
source.
