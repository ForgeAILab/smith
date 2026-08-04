---
created_at: 2026-08-01T00:56:16Z
updated_at: 2026-08-01T03:22:51Z
completed_at: 2026-08-01T03:22:51Z
---

## 0. Coordination and Baseline

- [x] 0.1 Approve this proposal and
  `../agent-runtime:add-resumable-child-sessions-2026-07-31` before
  implementation.
- [x] 0.2 Archive or explicitly rebase the process-ephemeral clauses in the
  completed Smith harness/session changes before applying this delta.
- [x] 0.3 Capture current `agent` schema, child lifecycle journal/reducer,
  headless output, and in-process follow-up behavior as compatibility fixtures.

## 1. Durable Smith Composition

- [x] 1.1 Compose Agent Runtime's parent-owned delegation catalog through
  Smith's owner-only persistence path, with atomic commits, checkpoint
  watermarks, lifecycle leases, schema/version gates, and bounded retention.
- [x] 1.2 Compose child session/checkpoint/artifact/catalog stores through the
  one Smith factory; remove unconditional child persistence clearing only for
  durable mode.
- [x] 1.3 Preserve the resolved inline/environment checkpoint-key path for
  children and prove it performs no Keychain/credential-service query or
  plaintext fallback.
- [x] 1.4 Restore and validate child records before wiring the resumed parent's
  coordinator, without constructing providers or spending tokens.

## 2. Exact Child Operations and Policy

- [x] 2.1 Add `resume` to the `agent` schema, preparation display,
  authorization, structured result, and documentation.
- [x] 2.2 Extend `list`/`result` with stable child-session, durability,
  resumability, cumulative limits, and bounded incompatibility fields while
  preserving old fields.
- [x] 2.3 Rebuild the original narrowed child composition lazily and fail closed
  on parent, project/trust, workspace, provider/model, profile/activation,
  tool-scope, lease, or checkpoint incompatibility.
- [x] 2.4 Update Smith's prompt policy to reuse a relevant idle child with
  `follow_up`, use explicit `resume` for interrupted work, and never silently
  replace a missing child with `spawn`.
- [x] 2.5 Keep child questionnaires routed through the root and preserve exact
  pending interaction state across child recovery.

## 3. Recovery and Migration

- [x] 3.1 Reconcile orphaned running records as interrupted without automatic
  execution and persist/replay one recovery transition.
- [x] 3.2 Resume exact child checkpoints without repeating committed provider,
  tool, approval, interaction, artifact, usage, or turn-count effects.
- [x] 3.3 Follow up recovered idle/needs-input children with prior canonical
  history and cumulative limits under the same IDs.
- [x] 3.4 Label journal-only historical children `legacy_ephemeral`, retain
  timeline evidence, and refuse fabricated resume/follow-up.
- [x] 3.5 Make stop, expiry, bounded-retention rejection, and parent deletion
  terminal/non-executable without touching project-owned files; physical
  session-file cleanup remains the configured store's retention concern.

## 4. TUI and Headless UX

- [x] 4.1 Render durable/ephemeral, idle/interrupted/resumable/blocked states in
  `/agent`, `/timeline`, live work, and replay-equivalent child inspection.
- [x] 4.2 Add existing-child `@child-id` completion distinct from spawn presets,
  preserve root draft/focus, and show exact follow-up spend/scope confirmation.
- [x] 4.3 Add explicit `/agent resume <child-id>` confirmation and progress,
  including a clear distinction between exact checkpoint continuation and a
  new follow-up turn.
- [x] 4.4 Version additive headless JSON/stream fields and operation results,
  with compatibility, redaction, and machine-consumer fixtures.
- [x] 4.5 Add narrow/normal/wide, colorless, keyboard, snapshot, and journal
  replay tests for idle, interrupted, incompatible, expired, and legacy child
  states.

## 5. Product Validation and Documentation

- [x] 5.1 Run fmt, warning-denied Clippy, workspace/all-feature tests, MSRV,
  strict spec validation, privacy/corruption/adversarial tests, and diff
  hygiene in both repositories.
- [x] 5.2 Run a deterministic process-restart scenario proving a follow-up uses
  the same child/session IDs and prior provider history.
- [x] 5.3 Run deterministic crash fixtures proving explicit resume does not
  repeat committed provider/tool work and startup performs no execution.
- [x] 5.4 Run a disposable live Z.AI Coding Plan coding/review scenario across
  Smith restart with same-child follow-up and explicit interrupted-task resume;
  record exact model, commands, events, results, and project cleanliness.
- [x] 5.5 Update `DESIGN.md`, persistence/recovery docs, threat model, command
  help, machine schema, setup guidance, and migration notes; reinstall only a
  fully verified local binary.
