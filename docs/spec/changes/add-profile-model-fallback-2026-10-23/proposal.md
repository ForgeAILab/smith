---
created_at: 2026-10-23T09:29:57Z
updated_at: 2026-10-23T13:32:37Z
---

## Why

A profile currently selects one provider/model pair. Retries stay on that
model, and credential pools only rotate accounts within its provider. A
profile cannot keep working when its model is unavailable or takes too long
to start responding.

## What Changes

- Keep `provider` and `model` as the primary selection, and add an optional
  `[profiles.<name>.fallback]` table with ordered, provider-qualified `models`.
  Different providers are supported, as requested.
- Add an optional `first_response_timeout_ms`. If a candidate has not started
  a meaningful response within that budget, cancel its attempt and advance
  directly to the next candidate. Connection time, retries, and backoff all
  consume the budget; HTTP headers and heartbeats do not satisfy it.
- Stop the startup timer when text, reasoning, or tool activity starts.
  Do not switch automatically because an answer stalls after it starts.
- Also advance on typed quota exhaustion and exhausted transient failures,
  within the original turn deadline. Configuration errors, authentication
  rejection, cancellation, and generic bad requests remain errors.
- Preserve the selected profile's instructions, posture, permissions, and
  tool history. Resolve and replan against each candidate's actual model
  limits, reasoning controls, modalities, tools, and cache partition.
- Show each switch and its reason in the TUI and headless events, retaining
  timing, usage, and cost attribution for every attempted provider/model.
- Implement the request-boundary mechanism in Agent Runtime first, then
  consume an immutable compatible revision in Smith. Do not add a Smith-owned
  replacement provider/tool loop.

## Configuration

Provider names refer to providers already declared in the user's configuration.

```toml
[profiles.code]
provider = "zai"
model = "glm-5.3"

[profiles.code.fallback]
first_response_timeout_ms = 30000
models = [
  { provider = "chatgpt", model = "gpt-6.1-sol" },
  { provider = "anthropic", model = "claude-sonnet-4-6" },
]
```

The example waits up to 30 seconds for each candidate to begin responding.
Omitting the timeout enables failure fallback without adding a latency
cutoff. Candidate entries may override the timeout, reasoning selection,
context window, and output request when their model needs different settings.

## Scope

Support ordinary provider-backed main and child turns through the shared
factory. Each new user turn starts with the configured primary; a successful
fallback handles the rest of that turn unless it subsequently fails before
another request starts responding. Installed `cli/...` harnesses, advisor and
vision subrequests, synthetic cache work, mid-answer switching, speculative
parallel requests, and persistent health routing are outside this change.

## Impact

- Affected specs: `configuration`, `provider-runtime`, `client-surfaces`.
- Existing contracts retained: shared runtime ownership, per-binding usage
  accounting, profile authority, and backward-compatible legacy sessions.
- Affected code: `smith-config` model/resolution/inventory, `smith-runtime`
  factory/rotation/session/event projection, `smith-client`, `smith-cli`,
  `smith-tui`, configuration guides, and Agent Runtime's provider driver,
  planner, event contracts, and consumer-conformance tests.
- Active Google-provider, project-memory, and guide changes touch some of the
  same capabilities but introduce no competing fallback requirements.
- Status: approved for implementation by the user on 2026-10-23. Implemented
  with shared runtime revision `a5f1b1788e982c39e7c3a394c715a7d92cc7852b`;
  the development override has been removed. See `evidence.md` for validation.
