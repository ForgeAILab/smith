---
created_at: 2026-08-05T00:00:00Z
updated_at: 2026-08-08T22:39:12Z
---

## Why

The built-in default `limits.turn_time_limit_ms = 600000` caps every turn at a
ten-minute wall-clock deadline. Long-running agent turns are routinely cut off
before the model finishes, and the deadline is the only thing ending otherwise
healthy work. A wall-clock ceiling should be an opt-in guard, not the default.

## What Changes

- Change the built-in default for `limits.turn_time_limit_ms` from `600000` to
  `0`.
- Treat a configured value of `0` as "no deadline": pass `None` to the shared
  loop's optional turn-time limit so a turn is unbounded by wall-clock time.
- A positive configured value remains the enforced deadline, unchanged.
- The setting keeps typed layered resolution and source provenance like every
  other limit.

## Impact

- Affected spec: `configuration`
- Affected code: `smith-config` built-in defaults, `smith-runtime` loop config
  translation and test fixture, config model docstring
- Affected docs: configuration reference example and defaults table
- Security: removes a default runaway-turn guard. A positive value still enforces
  a ceiling, and `max_tool_steps`, retries, interrupt, and session limits remain
  in effect. Users who want the guard back set a positive value in user config or
  via `SMITH_LIMITS_TURN_TIME_LIMIT_MS`.

## Approval Boundary

This change alters the built-in default only. It does not add configuration
authority, remove validation, change precedence, or alter any other limit. A
positive configured value is still honored exactly.

Approved by the user for implementation on 2026-08-05.
