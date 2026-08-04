---
created_at: 2026-08-02T04:44:53Z
updated_at: 2026-08-02T07:57:20Z
---

## Why

Smith can execute and resume explicit user turns, but it has no durable way for
a user to declare a longer-lived objective that continues across turn
boundaries until completion, a real blocker, or a trustworthy limit. Treating
that behavior as queued user prompts or TUI-local state would pollute canonical
history, race with real user input, and duplicate Agent Runtime persistence.

## What Changes

- Add one persisted goal per eligible root session with a bounded objective,
  explicit lifecycle status, optional token budget, provider-reported token
  usage, active elapsed time, stable identity, and timestamps.
- Add `get_goal`, `create_goal`, and `update_goal` model tools. The model may
  create a goal only when the user or higher-priority instructions explicitly
  request one, may set a budget only when explicitly requested, and may update
  status only to `complete` or `blocked`.
- Add a reusable Agent Runtime goal component plus a conditional internal-turn
  primitive. An active goal starts another bounded, provenance-bearing turn
  only when the session remains idle; it creates no synthetic user message and
  loses atomically to real user input.
- Persist goal state through the existing versioned session extension-state
  contract, emit durability-aligned typed goal events, restore active goals on
  resume, and keep all automatic work inside the current Smith process.
- Account uncached input plus output tokens from provider-reported usage and
  active wall-clock time exactly once at safe boundaries. Stop a budgeted goal
  rather than claiming enforcement when required usage evidence is missing.
- Add local `/goal` summary/create/edit/budget/pause/resume/clear controls,
  compact goal status, replay-equivalent reduction, and headless execution that
  follows goal continuations until the goal stops.
- Keep ordinary non-goal TUI and `smith -p` behavior unchanged.

## Explicit Non-Goals

- Codex-style image or oversized-paste attachment materialization for goal
  objectives.
- Goal-specific analytics, product telemetry, or metrics.
- Thread-fork inheritance or continuation deferrals.
- App-server JSON-RPC methods, notifications, or SDK bindings.
- A daemon, restart-durable scheduler, remote worker, monitor executor, nested
  goals, multiple concurrent root goals, or child-session goals.

## Impact

- Affected specs: `goal-lifecycle`, `runtime-integration`,
  `session-recovery`, `client-interaction`, `client-surfaces`
- Affected code: coordinated Agent Runtime harness/session contracts,
  `smith-runtime` composition and host lifecycle, `smith-cli` interactive and
  headless loops, `smith-tui` reducer/status/rendering, tests, and product docs
- External dependency: a separately approved and released Agent Runtime change
  providing the reusable goal component, trusted usage view, typed goal events,
  and conditional internal-turn contract
- Persistence: goal state uses the existing versioned session snapshot and
  completed-turn durability path; Smith adds no parallel database or project
  metadata
- Compatibility: ordinary turns retain their current semantics. Goal-aware
  JSON/JSONL projections add explicit optional goal records without
  reinterpreting existing fields
- Spend and safety: automatic continuation is available only after explicit
  goal creation, remains subject to existing provider/request/tool limits,
  approvals, workspace confinement, interruption, and shutdown

## Active Change Coordination

- `integrate-stable-session-harness-2026-07-31` remains authoritative for the
  single runtime factory, session checkpoints, extension state, lifecycle
  events, and TUI/headless semantic equivalence. This change consumes new
  generic runtime contracts and adds no fallback execution or persistence loop.
- `add-smith-agent-harness-2026-07-23` remains the baseline for process-scoped
  background work and explicit user authority. Goals do not authorize daemons,
  project metadata, or work after Smith shuts down.
- `add-smith-slash-commands-2026-07-26` remains authoritative for local command
  interception and help. `/goal` maps to typed host actions and never becomes a
  provider prompt.
- `add-agent-first-workflow-ux-2026-07-31` remains authoritative for the
  transcript-first composer and anchored todo presentation. Goal status is
  compact session chrome, not a second todo pane or permanent focus region.
- `add-context-and-reasoning-controls-2026-08-01` remains authoritative for
  idle-only session controls and provenance-aware status. Goal controls follow
  the same local validation and safe-boundary conventions.

## Delivery Slices

1. Approve and release the coordinated Agent Runtime contracts and conformance
   tests for goal state, usage, typed events, and conditional internal turns.
2. Pin the compatible runtime and compose goal abilities/controller for
   persistent root sessions only.
3. Add resume, interruption, accounting, user-priority, and error/limit state
   transitions with deterministic integration tests.
4. Add `/goal`, compact TUI state, replay, and headless multi-turn projections.
5. Run both repositories' formatting, Clippy, tests, consumer conformance,
   strict spec validation, and terminal/headless product fixtures.

## Approval Boundary

Approval authorizes Stage 2 changes in this repository only after the
coordinated Agent Runtime proposal and release are separately approved. It does
not authorize edits to `../agent-runtime`, attachment materialization,
analytics, fork behavior, app-server APIs, a daemon, child goals, nested goals,
multiple concurrent goals, or background work after Smith shutdown.
