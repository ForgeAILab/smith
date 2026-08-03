## Context

Google recommends the Interactions API for new Gemini integrations. It streams
typed steps for model output, thoughts, function calls, and usage, and supports
stateless operation by sending the complete step history with `store=false`.
Stateless tool continuation must preserve model thought blocks and signatures
exactly.

Smith currently composes providers only through Agent Runtime's shared
`Provider` contract. That contract already carries signed reasoning content,
canonical tool calls/results, vendor extensions, streaming events, and model
capabilities, but the new adapter still requires upstream conformance work to
prove exact Gemini step replay. Smith must consume the released adapter rather
than create a consumer-local provider trait.

Smith's catalog work already separates provider activation from model
metadata. Models.dev publishes a direct `google` provider with current model
IDs, limits, modalities, tool/structured-output flags, reasoning support, and
effort options. Today, however, Smith's generated snapshot excludes it and
user-written model overrides share `config.toml` with unrelated runtime policy.

## Goals / Non-Goals

### Goals

- Use Google's native Interactions protocol for Gemini.
- Require users to choose only authentication and model identity in the normal
  flow.
- Resolve model safety/capability metadata automatically from a frozen
  Models.dev snapshot.
- Keep custom model metadata and overrides in a focused optional file.
- Preserve Smith's canonical tool loop, approvals, persistence, cancellation,
  usage, retries, and surface parity.
- Keep provider continuation stateless, resumable, bounded, and
  redaction-safe.

### Non-Goals

- Use the OpenAI-compatible Gemini endpoint.
- Add the legacy `generateContent` API as a second Gemini adapter.
- Add Vertex AI, Application Default Credentials, service-account OAuth, or
  Google Cloud project/region configuration.
- Enable Google Search, code execution, computer use, URL context, or other
  provider-hosted tools.
- Treat Models.dev as authority for endpoints, credentials, package names, or
  account entitlement.
- Require `models.toml` for catalog-backed built-in providers.

## Decisions

### Add the native adapter upstream

Agent Runtime receives a `gemini-interactions` adapter over its injected
streaming HTTP transport and renewable credential-source contract. The adapter
targets Smith's reviewed Interactions endpoint/schema revision, sends the API
key only as `x-goog-api-key`, sets `stream=true` and `store=false`, and maps:

- canonical system/user/assistant/tool content to ordered Interactions steps;
- images and supported multimodal tool results without lossy text coercion;
- function declarations, choice policy, calls, argument deltas, and results;
- `thinking_level` to Agent Runtime's named reasoning effort;
- structured output to the native response schema;
- model output, thought summaries/signatures, finish states, usage categories,
  and cache observations to shared events; and
- HTTP/SSE failures to existing classified, redaction-safe provider errors.

Smith adds only the adapter kind, trusted endpoint policy, factory composition,
and product-facing descriptor after the upstream adapter is released. A local
fork or parallel Smith provider contract is rejected.

### Use stateless interactions and exact provider continuation

Every request sets `store=false`; Smith does not use
`previous_interaction_id`. The canonical history remains the local source of
truth and can survive provider switching, replay, and session resume without a
Google retention dependency.

The adapter must preserve every required thought block, signature, function
call, call ID, model-output step, and function result in order. Opaque
signatures are never displayed as reasoning text or general metadata. They are
bounded, serialized only with the content needed for exact provider replay,
protected by Smith's existing persistence/redaction policy, and omitted from
diagnostics. Missing continuation data fails before provider I/O rather than
sending a degraded or invalid tool continuation.

Alternative considered: `store=true` plus `previous_interaction_id`. Rejected
because it makes local resume depend on provider retention, complicates model
switching, and stores conversation state remotely by default.

### Resolve built-in model metadata from Models.dev

The existing bounded catalog generator and runtime validator add provider
identity `google`. A built-in `gemini-interactions` provider binds directly to
that catalog identity; no user endpoint match is needed because the adapter's
endpoint is fixed and cannot be overridden.

For catalog-backed Google models, Models.dev supplies limits, input/output
modalities, tool calling, structured output, reasoning presence, and effort
names. The same immutable snapshot drives `/model` selectability and runtime
preflight. A last-good cache wins over the embedded seed; refresh remains
credential-free and affects only a later host rebuild.

Smith retains one reviewed product choice: the quick-start model ID is
`gemini-3.6-flash`. It is selectable only when the frozen catalog contains a
valid, tool-capable record for that exact ID. Smith does not duplicate its
limits in source code or write them to user configuration.

Models.dev advertisement does not create a provider, endpoint, credential,
profile, entitlement, or auth method. Explicit user model fields keep existing
field-level precedence.

### Split explicit model metadata into `models.toml`

Smith discovers these model-catalog files alongside each configuration scope:

- `~/.smith/models.toml` at user scope;
- `<project>/.smith/models.toml` at project scope; and
- `<project>/.smith/models.local.toml` at project-local scope.

They accept only `[models."<provider>/<model>"]` tables and reject profiles,
providers, policy, credentials, or unknown fields. Their values join the
corresponding scope's provenance and retain the file path. Project and local
model overrides remain subject to project trust and cannot grant an adapter,
endpoint, credential, tool, or larger workspace.

New setup and model-editing flows write explicit model metadata only to the
appropriate `models*.toml` file. Provider declarations, credentials, profile
selection, and runtime policy remain in `config*.toml`. Catalog-backed models
write no explicit record at all.

Existing `[models]` tables in `config*.toml` remain accepted with a deprecation
diagnostic for one transition release. Defining the same model field in both
legacy and dedicated files at the same scope is an ambiguity error; Smith does
not silently choose by discovery order. A reviewed migration command moves
only model tables, preserves comments where possible, validates both candidate
files, publishes them as one rollback-capable transaction, and retains exact
prior bytes until full preflight succeeds.

### Keep normal Gemini configuration small

`/connect google` creates or updates only the trusted built-in provider's
credential reference and safe provider identity. Guided setup or `/model`
selects `gemini-3.6-flash` (or another valid catalog model) in a profile. It
does not ask for or write an endpoint, input limit, output limit, modality,
tool flag, reasoning flag, or effort list.

A representative resulting main config is intentionally small:

```toml
default_profile = "gemini"

[profiles.gemini]
provider = "google"
model = "gemini-3.6-flash"

[providers.google]
kind = "gemini-interactions"
credential = "env:GEMINI_API_KEY"
```

The provider table remains explicit so Smith does not activate credentials or
network access merely because an ambient variable exists. Protected storage
or an owner-only reviewed inline key can replace the environment reference
through the existing connection flow.

## Risks / Trade-offs

- The native Interactions schema is newer than `generateContent`. Pinning a
  reviewed schema/API revision and maintaining conformance fixtures reduces
  drift but requires active upkeep.
- Stateless requests resend history and may cost more than provider-side
  continuation. They preserve Smith's local canonical-history guarantees and
  avoid hidden remote state.
- Thought signatures are opaque provider continuation data. Incorrect
  ordering or persistence breaks tool continuation; exact multi-step and
  resume fixtures are release gates.
- Models.dev can lag Google. Embedded/last-good fallback improves availability,
  while exact catalog provenance and opt-in live tests expose staleness without
  claiming entitlement.
- Adding parallel model files expands discovery and transactions. Strict file
  schemas, same-scope ambiguity errors, and exact rollback prevent silent
  precedence changes.

## Migration Plan

- Existing OpenAI-compatible Gemini configurations remain valid and are not
  silently rewritten. Users explicitly reconnect or migrate to the native
  `google` provider.
- Existing `[models]` blocks in `config*.toml` remain readable for one
  transition release and can be moved by the reviewed migration command.
- New catalog-backed Google setup writes no model record. Custom model records
  and explicit overrides go to `models*.toml`.
- No provider, model, profile, credential, or default is selected merely from
  an environment variable or Models.dev entry.

## Open Questions

- The coordinated Agent Runtime proposal must confirm whether the current
  signed-reasoning content shape can replay every Interactions thought step or
  needs a bounded optional provider-continuation field on canonical content.
- The implementation review must pin the exact generally available
  Interactions endpoint/schema revision supported by deterministic fixtures.

## References

- Gemini Interactions API overview:
  https://ai.google.dev/gemini-api/docs/interactions-overview
- Gemini streaming events:
  https://ai.google.dev/gemini-api/docs/streaming
- Gemini stateless function calling:
  https://ai.google.dev/gemini-api/docs/function-calling
- Gemini thought signatures:
  https://ai.google.dev/gemini-api/docs/thought-signatures
- Gemini 3.6 Flash model record:
  https://ai.google.dev/gemini-api/docs/models/gemini-3.6-flash
- Models.dev Google metadata:
  https://github.com/anomalyco/models.dev/tree/dev/providers/google
