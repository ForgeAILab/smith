## ADDED Requirements

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

Provider configuration MUST contain credential references rather than plaintext
keys. Smith SHALL resolve a reference through its user-scope secret backend and
construct the shared provider with a redaction-safe `Secret` only after endpoint
and profile validation succeeds.

#### Scenario: Configure an OpenAI-compatible endpoint

- **GIVEN** a profile supplies a base URL, model, credential reference, and
  enforceable model limits
- **WHEN** Smith constructs the shared OpenAI-compatible provider
- **THEN** authorization is attached only at the transport boundary
- **AND** the raw secret is absent from Smith configuration, runtime events,
  snapshots, tool arguments, and logs

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
