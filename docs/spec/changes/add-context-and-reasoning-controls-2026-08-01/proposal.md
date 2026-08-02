---
created_at: 2026-08-01T21:37:58Z
updated_at: 2026-08-01T21:37:58Z
---

## Why

`/context` currently omits system-instruction and tool-schema rows when no
latest plan exists or when a plan has no total for that exact segment label.
Those are stable parts of Smith's model-facing request shape, so their absence
makes the view look incomplete. Smith also has a typed reasoning request in the
shared runtime but exposes no resolved default or session control, leaving
provider-supported thinking and effort settings inaccessible.

## What Changes

- Make `/context` always name `system instructions` and `tool schemas` in a
  stable order. Before the first plan they render `? · not counted yet`; after
  a plan they render the plan's estimated/exact total or an honest zero.
- Aggregate system-, developer-, and ability-instruction segments into the
  user-facing system row while retaining canonical segment totals in status
  and telemetry.
- Add source-explainable reasoning-control metadata that distinguishes
  unsupported, fixed/mandatory, toggleable, effort-controllable, and
  token-budget-controllable behavior. A boolean `reasoning = true` alone MUST
  continue to mean fixed, not controllable.
- Add profile defaults plus session-local `/think [on|off|default]` and
  `/effort [level|default]` controls. Omitted arguments open a bounded local
  selector; direct arguments use the same validation path.
- Show the effective thinking state, effort, control provenance, and any
  fixed/unavailable reason in `/status` and `/context`.
- Serialize only controls supported by the exact provider/model binding:
  OpenAI-style `reasoning_effort`, OpenRouter's unified `reasoning` object, or
  Z.AI's `thinking.type`. Unknown OpenAI-compatible endpoints remain fixed
  unless trusted metadata or explicit owner configuration declares controls.
- Apply a changed setting only while idle and atomically to the next complete
  turn, including its tool-call continuations. Persist the session override,
  revalidate it on provider/model changes and resume, and inherit the resolved
  setting into newly created children.

## Impact

- Affected specs: `client-surfaces`, `configuration`, `provider-runtime`
- Affected code: Smith configuration/model inventory and catalog projection,
  runtime composition and provider request adaptation, session persistence,
  slash-command dispatch, status/context rendering, help, and tests
- Compatibility: additive configuration and persisted-session fields with
  defaults; no raw reasoning content or model-facing context content is added
  to local status records
- Network behavior: no probe request is made when a user opens a reasoning
  selector; availability comes from the frozen resolved capability snapshot

## Active Change Coordination

- This change tightens the completed `add-smith-slash-commands-2026-07-26`
  context-view requirement without changing local-command/no-provider-spend
  behavior.
- This change extends `add-provider-model-catalogs-2026-07-28`; model reasoning
  presence remains distinct from adjustable reasoning controls.
- Existing uncommitted `Ctrl+C` footer work is unrelated and MUST be preserved.

## Approval Boundary

Approval authorizes stable system/tool context rows, typed reasoning metadata,
profile and session reasoning controls, exact supported provider dialects,
status visibility, and persistence/revalidation. It does not authorize guessing
controls from a reasoning boolean, sending arbitrary vendor JSON, changing a
busy turn, exposing raw reasoning text, probing a provider during UI use, or
silently downgrading a rejected control.
