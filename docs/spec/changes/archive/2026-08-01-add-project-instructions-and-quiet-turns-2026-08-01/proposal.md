---
created_at: 2026-08-01T17:26:57Z
updated_at: 2026-08-01T19:45:18Z
---

## Why

Smith currently defines a `project_context` prompt slot, but the standard
runtime factory populates only the active agent mode. Neither the interactive
nor headless host discovers `AGENTS.md`, so repository guidance is absent from
the model context unless the user explicitly asks the agent to read it.

The TUI also appends a local `turn` transcript notice for every successful
terminal event. A sub-second tool- or reasoning-only turn therefore produces
`completed in 0s without a visible answer (reasoning only)`. The canonical
runtime event is already journaled, and visible answers and tool results
already communicate the outcome, so the extra notice is noisy and its rounded
duration is easy to misread.

## What Changes

- Discover exactly `<canonical-project-root>/AGENTS.md` during standard host
  preflight and capture one bounded immutable snapshot for the lifetime of the
  constructed runtime.
- Activate that snapshot as a dedicated project-instruction fragment for both
  the TUI and `smith -p`, and let direct children inherit the parent's exact
  snapshot rather than reading mutable workspace state again.
- Give the project-instruction fragment a content-derived revision separate
  from Smith's stable product-policy fragments. A changed file affects a newly
  constructed runtime's exact prompt/cache identity without pretending the
  built-in Smith policy changed.
- Do not watch `AGENTS.md` or mutate an active runtime when the file changes.
  A user may explicitly ask the agent to read it again through the ordinary
  workspace tool path; the dedicated fragment refreshes on the next runtime
  construction.
- Stop appending transcript notices for successful turn completion, including
  completions with no visible assistant text. Continue to close active UI
  state and retain the canonical start/completion events, timestamps, usage,
  and timeline evidence.
- Keep interrupted, limited, needs-input, and failed terminals visible because
  they require explanation or user action.

## Impact

- Affected specs: `harness-policy`, `prompt-cache`, `client-interaction`
- Affected code: `smith-runtime` host/factory/prompt composition,
  `smith-cli` standard host startup, `smith-tui` event reduction and tests,
  plus `DESIGN.md` and security/context documentation
- Compatibility: no runtime event or headless output schema changes; only the
  interactive transcript projection becomes quieter
- Security: repository instructions guide behavior but cannot grant tool
  authority, widen the canonical workspace, change approval policy, activate
  executable project content, or override higher-priority host policy
- Cache behavior: the active runtime stays immutable. A later runtime built
  from changed instructions receives a new exact cache identity, while the
  independently revisioned Smith prefix remains identifiable as unchanged.

## Active Change Coordination

- `add-smith-agent-harness-2026-07-23` remains authoritative for one runtime
  composition path and exact cache identity. This change supplies one missing
  host-owned prompt input and does not create a second context planner.
- `integrate-stable-session-harness-2026-07-31` remains authoritative for
  independently versioned prompt fragments and project-text trust boundaries.
  This change makes default root `AGENTS.md` discovery one explicit host
  activation policy.
- `add-agent-first-workflow-ux-2026-07-31` remains authoritative for the
  transcript-first, quiet working presentation. This change supersedes only
  its documented successful `turn · completed ...` terminal decoration;
  actionable non-success terminals remain visible.

## Delivery Slices

1. Add bounded root instruction discovery and an immutable snapshot type at
   the standard host preflight boundary.
2. Compose the snapshot as one provenance- and revision-bearing prompt
   fragment shared by root, headless, resumed, and child runtimes.
3. Remove successful completion notices from the TUI reducer while preserving
   terminal state cleanup and journal/timeline evidence.
4. Add deterministic prompt, cache-identity, child-inheritance, reducer,
   replay, and narrow-terminal tests; update product/security documentation.

## Approval Boundary

Approval authorizes implementation of the root-only, read-once behavior and
quiet successful terminals described here. It does not authorize file
watching, automatic mid-session prompt mutation, nested `AGENTS.md` discovery,
`AGENTS.override.md`, a new reload command, a new trust/permission grant, or a
runtime/event schema change.
