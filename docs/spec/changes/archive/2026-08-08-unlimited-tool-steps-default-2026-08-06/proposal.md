---
created_at: 2026-08-06T00:00:00Z
updated_at: 2026-08-08T22:39:12Z
---

## Why

The built-in default `limits.max_tool_steps = 64` stops every turn after
sixty-four tool calls. Real agent turns — inspect, edit, run, verify, and the
delegated work they coordinate — routinely need more, and the turn ends with
`limit · ToolSteps reached` while the model was still working the task. The
wall-clock ceiling already became opt-in for exactly this reason; the tool-loop
ceiling is the other half of the same default and cuts the same work short.

## What Changes

- Change the built-in default for `limits.max_tool_steps` from `64` to `0`.
- Treat a configured value of `0` as "no ceiling": pass `None` to the shared
  loop's optional tool-step limit so the loop is bounded by the model rather
  than by a count.
- A positive configured value remains the enforced ceiling, unchanged.
- The setting keeps typed layered resolution and source provenance like every
  other limit.

## Impact

- Affected spec: `configuration`
- Affected code: `smith-config` built-in defaults and model docstring,
  `smith-runtime` loop config translation and test fixture
- Affected docs: configuration reference example and defaults table
- Security: removes the last default runaway-loop guard, since the wall-clock
  deadline is already opt-in. A turn is still bounded by interrupt, provider
  usage limits, session and child budgets, and approval gates on every
  write-capable tool. Users who want the guard back set a positive value in
  user config or via `SMITH_LIMITS_MAX_TOOL_STEPS`.

## Approval Boundary

This change alters the built-in default only. It does not add configuration
authority, remove validation, change precedence, or alter any other limit. A
positive configured value is still honored exactly.

Approved by the user for implementation on 2026-08-06.
