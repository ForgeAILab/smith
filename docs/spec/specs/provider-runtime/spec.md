# provider-runtime Specification

## Purpose
TBD - created by archiving change add-smith-agent-harness. Update Purpose after archive.
## Requirements
### Requirement: Shared capability-driven provider contract

Smith SHALL select providers through Agent Runtime's capability-driven
`Provider` contract and shared registry identities. Smith MUST NOT define a
closed provider enum, a parallel provider trait, or a second normalized event
vocabulary.

#### Scenario: Register a compatible provider

- **GIVEN** a trusted built-in or extension supplies a valid shared provider
  implementation with a unique identity
- **WHEN** Smith resolves the selected profile
- **THEN** the provider becomes selectable through Smith configuration
- **AND** request validation uses its shared model-dependent capabilities

#### Scenario: Capability varies by model

- **GIVEN** one provider exposes two models with different tool capabilities
- **WHEN** the user selects either model
- **THEN** the shared resolved model profile governs request validation
- **AND** Smith does not apply a provider-wide capability guess

### Requirement: Staged shared adapter availability

Smith's first production provider SHALL use Agent Runtime's OpenAI-compatible
Chat-Completions adapter, and deterministic tests SHALL use its fake provider.
OpenAI Responses, Anthropic Messages, and additional adapters MUST be
implemented and conformance-tested in Agent Runtime before Smith exposes them.

#### Scenario: Run without provider spend

- **GIVEN** a deterministic fake script containing text, tool, usage, cache,
  and error events plus an explicit fake model profile
- **WHEN** a Smith contract test runs a turn
- **THEN** the shared runtime reproduces those events without network access
- **AND** the same Smith runtime factory is exercised

#### Scenario: Requested adapter is not in the pinned runtime

- **GIVEN** configuration requests OpenAI Responses or Anthropic before the
  pinned Agent Runtime contains that adapter
- **WHEN** Smith validates the selected profile
- **THEN** startup reports that the adapter is unavailable
- **AND** Smith does not use a consumer-local compatibility shim

### Requirement: Smith-owned production transport

Smith SHALL supply the concrete streaming HTTP transport injected into shared
provider adapters. The transport MUST enforce cancellation, deadlines, bounded
request/response handling, TLS validation, status/error classification, and
redaction of headers, bodies, credentials, and prompt content from debug and
diagnostic surfaces.

#### Scenario: Provider stream is cancelled

- **GIVEN** the shared provider adapter is waiting on Smith's HTTP byte stream
- **WHEN** the session cancellation token fires
- **THEN** the transport request and byte stream are dropped promptly
- **AND** the shared runtime emits a structured cancelled attempt

#### Scenario: Error body contains a credential

- **GIVEN** a provider error body reflects an authorization value
- **WHEN** Smith maps the transport failure
- **THEN** the value is absent from visible and persisted diagnostics
- **AND** only bounded redaction-safe metadata reaches the shared provider error

### Requirement: Host-bound credential resolution

Provider configuration MUST contain either a credential reference or an inline
`api_key` accepted only from owner-only user configuration. Smith SHALL
construct the shared provider with a redaction-safe `Secret` only after
endpoint and profile validation succeeds.

#### Scenario: Configure an OpenAI-compatible endpoint with a reference

- **GIVEN** a profile supplies a base URL, model, credential reference, and
  enforceable model limits
- **WHEN** Smith constructs the shared OpenAI-compatible provider
- **THEN** authorization is attached only at the transport boundary
- **AND** the raw secret is absent from Smith runtime events, snapshots, tool
  arguments, and logs

#### Scenario: Configure an inline user key

- **GIVEN** owner-only user config supplies a provider `api_key`
- **WHEN** Smith constructs the provider
- **THEN** it wraps and registers the value with the persistence redactor
  before provider construction
- **AND** it performs no Keychain, Secret Service, or environment lookup
- **AND** every runtime and persistence surface remains secret-free

### Requirement: Shared normalized loss-aware streaming

Smith SHALL consume Agent Runtime's versioned text, reasoning, tool-call,
finish, error, usage, cache-observation, and planning events. Provider-only
metadata MAY be retained only through the shared bounded redaction-safe
extension mechanism.

#### Scenario: Stream a fragmented tool call

- **GIVEN** an adapter receives a tool name and arguments across several wire
  chunks
- **WHEN** Agent Runtime normalizes the stream
- **THEN** it assembles one tool call in the original order
- **AND** Smith executes nothing until shared validation completes

### Requirement: Explicit unsupported behavior

Smith SHALL configure the shared downgrade policy explicitly. An unsupported
requested capability MUST fail before network I/O unless that named downgrade
is enabled, and every applied downgrade MUST remain visible in shared events.

#### Scenario: Unsupported tools

- **GIVEN** the request advertises tools and the selected model lacks tool
  support
- **WHEN** no tools downgrade is configured
- **THEN** the shared runtime rejects the request before provider I/O

#### Scenario: Approved downgrade

- **GIVEN** the resolved profile enables a supported downgrade
- **WHEN** the selected model lacks that capability
- **THEN** Agent Runtime applies the configured downgrade
- **AND** Smith displays or emits the shared downgrade event

### Requirement: Safe provider and model switching

Smith SHALL change provider or model only between turns. It MUST retain the
canonical session, create a new cache identity, warn that the old remote cache
does not transfer, and reconstruct an immutable shared runtime when in-place
reconfiguration is unavailable.

#### Scenario: Switch provider during a session

- **GIVEN** a completed session turn used provider A
- **WHEN** the user confirms a switch to provider B
- **THEN** Smith saves and resumes the same session through a runtime configured
  for provider B
- **AND** cache state becomes unknown or unsupported for the new provider
- **AND** context counts remain labelled by their actual provenance

### Requirement: Supported Codex subscription spike

Smith MUST isolate Codex subscription work behind an experimental feature and
MUST use only publicly supported authentication and integration surfaces. The
spike SHALL classify the result as a direct provider adapter, an external Codex
agent backend, or unsupported.

#### Scenario: Only app-server is supported

- **GIVEN** supported documentation permits subscription use through Codex
  app-server but not a generic direct model API
- **WHEN** the spike is evaluated
- **THEN** Smith labels it an external agent backend
- **AND** the shared direct API-provider loop remains independent

#### Scenario: No supported path succeeds

- **GIVEN** no publicly supported integration satisfies the spike
- **WHEN** the spike concludes
- **THEN** the feature reports unsupported with evidence
- **AND** experimental credential or private-endpoint code is removed

### Requirement: Validated provider catalog cache

The Smith host SHALL load provider model metadata from a generated embedded
seed or a schema-validated last-good user cache and MAY refresh that cache only
from Smith's exact public Models.dev HTTPS source. Catalog loading and refresh
MUST be bounded, credential-free, atomically published, and non-blocking for an
otherwise usable embedded or cached snapshot.

#### Scenario: Valid last-good cache exists

- **GIVEN** a schema-valid last-good cache exists
- **WHEN** Smith prepares the host catalog
- **THEN** it uses that cache without requiring a network response
- **AND** schedules refresh only according to the cache freshness policy

#### Scenario: Cache is absent or corrupt

- **GIVEN** the user cache is missing, truncated, malformed, or fails schema
  validation
- **WHEN** Smith prepares the host catalog
- **THEN** it uses the generated embedded seed
- **AND** the invalid cache contributes no model metadata

#### Scenario: Refresh succeeds

- **GIVEN** the current snapshot is stale
- **WHEN** the bounded Models.dev refresh returns a complete valid response
  from the allowed origin
- **THEN** Smith writes a temporary file in the cache directory and atomically
  publishes it as the new last-good cache
- **AND** exposes the new snapshot only to a later host rebuild

#### Scenario: Refresh fails safely

- **GIVEN** refresh times out, exceeds the byte limit, redirects outside the
  allowed origin, returns a bad status, or fails schema validation
- **WHEN** the refresh attempt ends
- **THEN** Smith retains the current last-good or embedded snapshot unchanged
- **AND** startup, the current session, and local picker interaction remain
  usable

#### Scenario: Refresh sends no provider secret

- **GIVEN** configured providers reference environment, keychain, or inline
  credentials
- **WHEN** Smith requests public Models.dev metadata
- **THEN** the request contains no provider credential or provider-specific
  authorization/header value
- **AND** Smith does not open the credential backend for catalog refresh

### Requirement: Frozen catalog-backed runtime profile

Smith SHALL pass the same immutable catalog source used for model enumeration
into runtime construction. Model limits and capabilities MUST be resolved and
frozen before provider I/O, and a later catalog refresh MUST NOT mutate an
active runtime or session.

#### Scenario: Select a catalog-only model

- **GIVEN** `/model` offers a selectable catalog model with no explicit local
  model record
- **WHEN** the user selects that provider/model pair
- **THEN** the host rebuild injects the same frozen catalog source into
  `RuntimeRequest.catalog_sources`
- **AND** runtime preflight resolves complete limits before any provider request

#### Scenario: Catalog changes during a session

- **GIVEN** a host was constructed from catalog revision A
- **WHEN** background refresh publishes catalog revision B
- **THEN** the active runtime continues using revision A
- **AND** revision B can affect selection or preflight only after a later host
  rebuild

#### Scenario: Catalog and picker cannot diverge

- **GIVEN** a catalog-backed entry is marked selectable in `/model`
- **WHEN** that exact entry is applied without intervening host rebuild
- **THEN** runtime preflight uses the snapshot that established its
  selectability
- **AND** does not refetch or substitute metadata by insertion order

#### Scenario: Catalog metadata conflicts at equal precedence

- **GIVEN** two cached-remote sources disagree about the same winning model
  field
- **WHEN** runtime profile resolution runs
- **THEN** existing same-layer conflict handling fails before provider I/O
- **AND** Smith does not choose whichever source was registered first

### Requirement: Exact reasoning request dialects

Smith SHALL translate a typed reasoning selection only through the exact
request dialect trusted for the resolved provider/model binding. OpenAI effort,
OpenRouter reasoning-object, and Z.AI thinking-object fields MUST remain
distinct, and unknown OpenAI-compatible endpoints MUST NOT inherit a dialect
from their name alone.

#### Scenario: OpenAI effort request

- **GIVEN** the resolved binding advertises an OpenAI-style `high` effort
- **WHEN** Smith prepares the provider request
- **THEN** it emits `reasoning_effort = "high"`
- **AND** it emits no OpenRouter `reasoning` or Z.AI `thinking` object

#### Scenario: OpenRouter unified reasoning request

- **GIVEN** the resolved OpenRouter model advertises optional `low` effort
- **WHEN** Smith prepares the provider request
- **THEN** it emits the typed OpenRouter `reasoning` object
- **AND** the object cannot override model, messages, tools, or other normalized
  fields

#### Scenario: Z.AI thinking toggle request

- **GIVEN** the resolved Z.AI model supports turn-level thinking
- **WHEN** Smith prepares a turn with thinking disabled
- **THEN** it emits `thinking.type = "disabled"`
- **AND** it does not emit `reasoning_effort`

#### Scenario: Unknown endpoint exposes no inferred control

- **GIVEN** an OpenAI-compatible endpoint has no trusted control metadata
- **WHEN** configuration or a session asks to control reasoning
- **THEN** Smith refuses before provider I/O
- **AND** it sends no guessed vendor extension or downgrade

### Requirement: Immutable per-turn reasoning selection

Smith SHALL snapshot the effective reasoning selection at turn acceptance and
apply it unchanged to every attempt and tool-call continuation in that turn.
An idle-session update MAY affect a later turn but MUST NOT race the active
selection.

#### Scenario: Tool continuation retains thinking settings

- **GIVEN** a turn starts with thinking enabled at a supported effort
- **WHEN** the model calls a tool and Smith sends a continuation
- **THEN** the initial request and continuation use the same typed selection
- **AND** preserved reasoning content follows the existing continuation policy

#### Scenario: Retry retains thinking settings

- **GIVEN** a provider attempt fails retryably
- **WHEN** Smith sends another attempt for the same turn
- **THEN** the retry uses the identical reasoning selection
- **AND** capability validation is not silently downgraded

#### Scenario: New child inherits resolved selection

- **GIVEN** the parent has a valid effective reasoning selection
- **WHEN** Smith creates a new child session
- **THEN** the child inherits that resolved selection
- **AND** a later parent override does not mutate the already-running child

### Requirement: Host-injected renewable provider credentials

Agent Runtime SHALL accept a host-injected provider credential source that can
acquire a redaction-safe authorization lease with optional expiry and opaque
revision identity. Acquisition, proactive refresh, and invalidation MUST be
bounded by cancellation and the provider-call deadline, while static secrets
remain supported through the same semantics.

#### Scenario: Acquire a static API key

- **GIVEN** a direct provider uses a non-expiring API key source
- **WHEN** its adapter prepares a provider request
- **THEN** the credential source returns a static secret lease
- **AND** the adapter attaches it only at the wire authorization boundary

#### Scenario: Refresh an expiring credential

- **GIVEN** a renewable credential would expire before the configured safety
  margin
- **WHEN** a provider attempt acquires authorization
- **THEN** the credential source refreshes before returning a lease
- **AND** access and refresh values remain absent from runtime events, errors,
  snapshots, manifests, and debug output

#### Scenario: Credential refresh is cancelled

- **GIVEN** credential acquisition or refresh is waiting on external I/O
- **WHEN** cancellation fires or the provider-call deadline expires
- **THEN** the attempt stops with a classified credential error
- **AND** no provider request is started with an expired or partially refreshed
  credential

### Requirement: Bounded authentication-rejection replay

A direct provider adapter SHALL invalidate a renewable lease and replay at
most once only when the transport classifies an authentication rejection
before a response stream is accepted. The runtime MUST NOT refresh/replay a
partially accepted stream or enter an unbounded authentication loop.

#### Scenario: Expired access token is rejected before streaming

- **GIVEN** the provider rejects the current lease as unauthorized before any
  response stream is accepted
- **WHEN** the credential source successfully returns a newer revision
- **THEN** the adapter may replay the request once
- **AND** records only redaction-safe attempt and refresh classifications

#### Scenario: Replacement credential is also rejected

- **GIVEN** one authentication refresh replay has already occurred
- **WHEN** the provider rejects the replacement lease
- **THEN** the attempt fails with a non-retryable authentication error
- **AND** no third credential acquisition or provider replay occurs

### Requirement: OAuth protocol ownership boundary

Agent Runtime core SHALL define renewable authorization semantics without
embedding browser control, callback listeners, device-code polling, OAuth
issuers, client identities, scopes, token endpoints, or persistence. Concrete
OAuth ceremonies SHALL be implemented by trusted provider/backend integrations
and presented by the host.

#### Scenario: Register an OAuth-capable direct provider

- **GIVEN** a trusted provider integration implements its documented OAuth
  ceremony and renewable credential source
- **WHEN** a host connects and constructs that provider
- **THEN** Agent Runtime consumes only credential leases and provider
  authorization behavior
- **AND** the host/integration remains responsible for login UX and storage

#### Scenario: OAuth login does not authorize the selected API

- **GIVEN** an OAuth flow produces credentials that are not publicly supported
  by the selected direct model API
- **WHEN** the integration is evaluated for registration
- **THEN** it is rejected as a direct provider authentication method
- **AND** a successful browser login alone is not reported as a usable provider

### Requirement: Smith ChatGPT Responses adapter

Smith's experimental ChatGPT integration SHALL implement Agent Runtime's
neutral provider contract in `smith-runtime`. It SHALL keep the fixed OAuth,
account-header, endpoint, Responses wire, and token-refresh behavior out of
Agent Runtime core.

#### Scenario: Execute through the direct adapter

- **GIVEN** Smith has a current ChatGPT credential bundle in its owner-only
  auth file
- **WHEN** Agent Runtime sends a canonical provider request
- **THEN** the adapter maps messages, images, tools, reasoning continuity,
  streaming output, usage, and terminal reasons to the fixed Responses wire
- **AND** the normal Smith runtime owns tool execution, approvals, persistence,
  cancellation, recovery, and events

#### Scenario: Keep an unsupported output cap local

- **GIVEN** Smith's trusted model record and canonical request carry an output
  reserve and request budget
- **WHEN** the current experimental ChatGPT backend does not accept a
  per-request output-token parameter
- **THEN** the adapter uses the values for local context planning but omits the
  unsupported field from the Responses wire request
- **AND** an offline fixture and explicitly injected opt-in live smoke cover
  that compatibility boundary without querying an OS credential service

#### Scenario: Undocumented contract changes

- **GIVEN** the fixed ChatGPT endpoint rejects the reviewed wire contract
- **WHEN** the adapter cannot classify a valid response or stream
- **THEN** it fails with a redaction-safe provider error
- **AND** does not launch Codex, fall back to another client's cache, or route
  work through an external agent loop
