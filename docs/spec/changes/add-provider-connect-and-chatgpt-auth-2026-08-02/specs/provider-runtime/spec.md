## ADDED Requirements

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
