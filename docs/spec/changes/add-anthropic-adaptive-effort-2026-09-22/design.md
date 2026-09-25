---
created_at: 2026-09-22T14:21:25-04:00
updated_at: 2026-09-22T14:21:25-04:00
---

# Design: Add Anthropic adaptive effort controls

## Context

Smith resolves reasoning controls from trusted per-binding metadata into a
neutral `ReasoningConfig`. A Smith dialect wrapper translates vendor-specific
OpenAI-compatible extensions, while native adapters are allowed to translate
the neutral request themselves. Agent Runtime's Anthropic Messages adapter
already implements the latter mapping: an effort produces adaptive thinking
plus `output_config.effort`, and a token budget uses the legacy enabled-budget
shape.

Smith nevertheless requires a dialect to establish that controls are trusted.
Its dialect enum lacks an Anthropic variant, so exact explicit metadata cannot
describe Fable 5.1's mandatory adaptive thinking or effort ladder.

## Goals / Non-Goals

- Goals:
  - Represent native Anthropic adaptive-effort support explicitly.
  - Make CLI, TUI, status, persistence, retries, and continuations reuse the
    existing typed reasoning policy.
  - Refuse unsupported efforts and thinking-off locally.
  - Prove the native request mapping without spending against a live model.
- Non-Goals:
  - Infer support from `claude-*` or `fable-*` names.
  - Duplicate Anthropic JSON construction in Smith.
  - Change retries, route selection, provider availability, or output display.
  - Reveal model reasoning content.

## Decisions

### Add an explicit native Anthropic dialect

`ReasoningDialect` gains `AnthropicEffort`, serialized as
`anthropic-effort`. The value is accepted only as trusted explicit metadata for
an exact provider/model binding. Smith does not grant it from an arbitrary
endpoint or model-name pattern.

Like the existing native Gemini dialect, the request wrapper treats this
dialect as a deliberate no-op. The wrapper's purpose is capability trust and
selection validation; Agent Runtime's Anthropic adapter remains the single
owner of the Anthropic wire schema.

### Describe Fable 5.1 as mandatory adaptive thinking

The recommended exact model metadata is:

```toml
[models."dddai/claude-fable-5-1".reasoning]
mandatory = true
efforts = ["low", "medium", "high", "xhigh", "max"]
default_enabled = true
default_effort = "high"
dialect = "anthropic-effort"
```

The user may select any advertised effort, or provider default. Because the
capability is mandatory, `/think off` fails before credential lookup or
provider I/O. No model-name inference is needed, and a different proxy/model
can advertise a different exact contract.

### Preserve ownership of the wire request

Smith passes the neutral effort unchanged into the native provider. Agent
Runtime encodes the request as:

```json
{
  "thinking": { "type": "adaptive" },
  "output_config": { "effort": "low" }
}
```

Deterministic Smith tests verify that `AnthropicEffort` preserves the neutral
selection and resolves mandatory-on capability metadata. Agent Runtime's
existing adapter test remains the conformance proof for the exact JSON body;
Smith does not introduce a second serializer or live-provider dependency.

## Risks / Trade-offs

- A proxy may list a model while its upstream route is unavailable. Correct
  capability metadata does not imply entitlement or health; the real 503 must
  remain visible and retry under existing policy.
- Explicit configuration is more verbose than model-name detection, but keeps
  control claims source-explainable and avoids sending unsupported fields to
  unknown Anthropic-compatible proxies.
- The new enum value is additive in config, but older Smith binaries will
  reject it. The user config should therefore be updated only after the new
  binary is built and tested.

## Migration Plan

1. Add and validate the `anthropic-effort` dialect.
2. Resolve explicit mandatory-on Anthropic metadata through the shared policy.
3. Add deterministic config/runtime/client coverage and documentation.
4. Run focused and workspace gates without a paid Fable 5.1 request.
5. Add the reviewed reasoning block to the user's dddai Fable 5.1 model entry.

## Open Questions

None. dddai already accepted a native adaptive-thinking request on its healthy
Fable 5 route, and the separate Fable 5.1 503 is an upstream availability
issue rather than an ambiguity in Smith's request contract.
