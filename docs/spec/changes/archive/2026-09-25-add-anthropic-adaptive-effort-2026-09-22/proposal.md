---
created_at: 2026-09-22T14:21:25-04:00
updated_at: 2026-09-22T14:21:25-04:00
---

# Proposal: Add Anthropic adaptive effort controls

## Why

Smith can construct the Anthropic Messages adapter, and Agent Runtime already
maps its neutral reasoning selection to Anthropic's native adaptive-thinking
request fields. Smith cannot currently select that path, however, because its
trusted reasoning metadata has no Anthropic request dialect. An explicitly
configured Fable 5.1 model is consequently treated as reasoning-present but
uncontrollable, so `--effort`, `/effort`, and mandatory-thinking validation
fail locally before the provider is constructed.

The missing local capability is independent of dddai's current Fable 5.1
`503 Service Unavailable` response. Adding it will make Smith send a valid
effort selection once the route is healthy, but it cannot repair or hide an
upstream routing failure.

## What Changes

- Add a trusted `anthropic-effort` reasoning dialect for models served through
  the Anthropic Messages adapter.
- Preserve Smith's neutral typed reasoning selection through the dialect
  wrapper so Agent Runtime emits `thinking.type = "adaptive"` and
  `output_config.effort = <selected level>` on the native Anthropic request.
- Allow exact model metadata to declare Anthropic's mandatory-on adaptive
  thinking and ordered `low`, `medium`, `high`, `xhigh`, and `max` effort
  ladder, including a provider-default `high` value.
- Keep `/think off` unavailable for mandatory models while making `/effort`
  and `--effort` use the existing capability-driven validation and provenance
  surfaces.
- Document a complete dddai Fable 5.1 model stanza and add deterministic tests
  that perform no paid provider inference.

Out of scope: inferring controls from a model name, exposing raw chain of
thought, changing Anthropic retry behavior, changing provider routing, adding
an automatic model fallback, or claiming to resolve dddai's current Fable 5.1
503.

## Impact

- Affected specs: `configuration`, `provider-runtime`, `client-surfaces`
- Affected code: `smith-config` reasoning dialect parsing and validation,
  `smith-runtime` reasoning policy/dialect adaptation, focused CLI/runtime/TUI
  capability tests, and configuration documentation
- Agent Runtime remains pinned and unchanged because its Anthropic adapter
  already owns the native wire mapping and has adapter-level coverage
- Existing configurations and sessions remain compatible; the new enum value
  is additive

## Approval Boundary

Approval authorizes the additive dialect, exact explicit model metadata,
capability-driven UI/CLI behavior, deterministic tests, documentation, and an
update to the user's dddai Fable 5.1 model entry after the code passes. It does
not authorize paid Fable 5.1 probes, provider/model fallback, route changes,
retry-policy changes, raw reasoning display, or unrelated TUI restyling.
