---
created_at: 2026-08-02T04:44:53Z
updated_at: 2026-08-02T07:57:20Z
completed_at:
---

## 0. External Runtime Gate

- [ ] 0.1 Record the separately approved Agent Runtime proposal and exact
  released revision providing conditional internal turns, goal component
  state/tools/context, trustworthy usage views, and typed goal events.
  - The coordinated proposal is approved and implemented in Agent Runtime
    commit `0c043e3bbb8f`; publishing that immutable revision remains pending.
- [x] 0.2 Pass Agent Runtime's goal/internal-turn conformance suite and Smith
  consumer suite before changing Smith's pinned revision.

## 1. Runtime Compatibility and Composition

- [ ] 1.1 Pin the approved compatible Agent Runtime revision and refresh all
  dependency/release evidence required by Smith's stable-runtime workflow.
  - Smith passes against the compatible sibling checkout. The committed git
    pin intentionally remains unchanged until that runtime is immutable.
- [x] 1.2 Register goal tools and the reusable goal component through
  `smith-runtime::factory` for persistent root sessions only; keep child,
  review, and explicitly ephemeral sessions excluded.
- [x] 1.3 Attach exactly one goal controller to each eligible hosted session
  and restore its projection before automatic continuation is admitted.
- [x] 1.4 Preserve the ordinary-turn composition/event/history baseline when no
  goal exists or goal abilities are inactive.

## 2. Goal Policy and Lifecycle

- [x] 2.1 Implement Smith's explicit-creation, bounded-objective, positive
  budget, one-goal, stale-identity, replacement, and transition validation.
- [x] 2.2 Map provider-reported uncached-input/output usage and active elapsed
  time into the goal component with exact-once final accounting.
- [x] 2.3 Implement complete, blocked, paused, usage-limited, budget-limited,
  accounting-unavailable, and terminal-error behavior without automatic
  restart from a stopped state.
- [x] 2.4 Verify conditional continuation loses to real user input, never
  creates a user-role history message, and deduplicates replayed/resumed
  terminal observations.

## 3. Persistence and Recovery

- [x] 3.1 Persist goal mutations and typed projections through the canonical
  extension-state/checkpoint/snapshot path with no parallel database or
  project metadata.
- [x] 3.2 Restore active, stopped, and complete goals with exact identity,
  usage provenance, elapsed time, and no process-downtime accounting.
- [x] 3.3 Reconcile crash and shutdown boundaries so completed work is not
  repeated and active goals continue only after a later host attaches.

## 4. Interactive Surface

- [x] 4.1 Add `/goal`, create, edit, budget, pause, resume, and clear commands
  through the existing local command registry and typed host-action path.
- [x] 4.2 Enforce idle-only mutations except busy goal pause, which interrupts
  the serving goal turn and commits final accounting/status exactly once.
- [x] 4.3 Add compact provenance-aware goal status and `/status` detail without
  adding a second todo pane or focusable region.
- [x] 4.4 Add live/replay reducer and narrow/normal/wide snapshot coverage for
  every goal status, unknown usage, budget overshoot, and local validation
  error.

## 5. Headless Surface

- [x] 5.1 Extend the headless loop to follow attributed goal continuations until
  a stopped boundary while retaining one-turn behavior for ordinary prompts.
- [x] 5.2 Add optional text/JSON/JSONL goal projections, continuation counts,
  usage provenance, and structured interaction-required/limit outcomes
  without reinterpreting existing fields.
- [x] 5.3 Add deterministic headless fixtures for completion, blocker,
  unavailable accounting, budget overshoot, provider usage limit,
  interruption/shutdown, and resume.

## 6. Verification and Documentation

- [x] 6.1 Add state-machine, race, accounting, persistence, replay, TUI, and
  surface-equivalence tests using provider-reported and missing-usage fixtures.
- [x] 6.2 Update `DESIGN.md`, security/context documentation, command help, and
  release notes with process-scoped goal semantics and explicit non-goals.
- [x] 6.3 Run formatting, warning-denied Clippy, targeted and workspace tests,
  Agent Runtime consumer conformance, strict change validation, and diff
  hygiene before requesting implementation review.
