## MODIFIED Requirements

### Requirement: Shared capability-driven provider contract

Smith SHALL select providers through Agent Runtime's capability-driven
`Provider` contract and shared registry identities. Smith MUST NOT define a
closed provider enum, a parallel provider trait, or a second normalized event
vocabulary.

Every resolved model profile SHALL carry the shared provider cache contract
needed to distinguish unsupported, implicit-prefix, explicit-breakpoint, and
explicit-resource behavior; an opaque exact cache identity; observable
read/write/expiry evidence; supported retention controls; cache key and
breakpoint constraints; explicit resource operations; and conformance-gated
keepalive or handoff safety. Capability MUST remain model-dependent when one
provider serves models with different cache behavior. Omitted cache evidence
is `unknown`; expiry or resource deletion is valid only from a typed upstream
cache/resource event or typed provider error correlated to that exact identity.
Elapsed time or a missing observation MUST NOT be normalized as expiry.

Smith-owned endpoint identity and host policy MAY narrow the shared contract
but MUST NOT invent provider capabilities, TTLs, observations, or maintenance
safety absent from the adapter declaration. Agent Runtime owns the canonical
cache/admission event vocabulary and payloads; Smith consumes and projects
those events and MUST NOT define a second vocabulary or local provider event
names.

#### Scenario: Register a compatible provider

- **GIVEN** a trusted built-in or extension supplies a valid shared provider
  implementation with a unique identity
- **WHEN** Smith resolves the selected profile
- **THEN** the provider becomes selectable through Smith configuration
- **AND** request validation uses its shared model-dependent capabilities

#### Scenario: Capability varies by model

- **GIVEN** one provider exposes two models with different tool or cache
  capabilities
- **WHEN** the user selects either model
- **THEN** the shared resolved model profile governs request and maintenance
  validation
- **AND** Smith does not apply a provider-wide capability guess

#### Scenario: Compatible endpoint has no verified retention contract

- **GIVEN** a custom OpenAI-compatible endpoint can serve ordinary requests
- **AND** its selected model has no conformance-backed maintenance declaration
- **WHEN** Smith constructs the runtime in adaptive mode
- **THEN** ordinary provider use remains available
- **AND** synthetic cache maintenance is narrowed to observation-only

## ADDED Requirements

### Requirement: Adapter conformance before synthetic maintenance

An adapter SHALL NOT advertise keepalive or handoff-checkpoint safety until
conformance proves:

- the exact stable prefix is preserved on the wire;
- cache keys and breakpoints remain stable;
- the synthetic suffix is excluded from later canonical requests;
- opaque cache identity, key/breakpoint/resource fields, and typed
  resource/expiry evidence are preserved on the upstream plan or observation;
- usage and presence-aware cache observations normalize correctly;
- a miss remains distinguishable when the provider exposes sufficient
  evidence;
- tools and tool choice are disabled;
- bounded output, deadline, and cancellation work;
- retries cannot create duplicate maintenance calls; and
- request/response/error diagnostics remain redaction-safe.

Conformance SHALL be scoped to the adapter, endpoint contract revision, model
capability, and synthetic action shape it actually tested. Ordinary provider
cache support MUST NOT automatically grant synthetic-action safety.

#### Scenario: Adapter has cache support but no maintenance conformance

- **GIVEN** an adapter supports ordinary prompt caching
- **BUT** has not passed synthetic-maintenance conformance
- **WHEN** Smith resolves `maintenance = "adaptive"`
- **THEN** it uses ordinary caching and canonical observations
- **AND** behaves as `observe` for synthetic maintenance

#### Scenario: Only handoff checkpoint is conformance-approved

- **GIVEN** an adapter has passed the handoff-checkpoint fixture
- **AND** has not passed the minimal keepalive fixture
- **WHEN** maintenance is evaluated
- **THEN** Smith may select a bounded handoff when otherwise eligible
- **AND** MUST NOT fall back to an unapproved keepalive shape

#### Scenario: Adapter revision changes

- **GIVEN** maintenance conformance applies to adapter revision A
- **WHEN** the resolved provider uses revision B
- **THEN** revision A's safety declaration is inapplicable
- **AND** revision B remains observation-only until independently conformed

#### Scenario: Runtime event is projected without a second vocabulary

- **GIVEN** Agent Runtime emits a canonical cache or admission event
- **WHEN** Smith renders status, TUI, or machine output
- **THEN** it carries the upstream event identity, revision, and redaction-safe
  fields as a projection
- **AND** it does not create a competing canonical event type or provider
  event name

### Requirement: Explicit cache-resource operations are lifecycle-bound

Agent Runtime SHALL represent explicit cache create, extend, inspect, and
delete operations through typed provider
capabilities and redaction-safe outcomes. Smith SHALL invoke them only for the
exact cache identity, within resolved authority and budget, and while the
session lifecycle lease remains active. Resource handles or cache contents
MUST NOT become canonical conversation content or portable warmth claims.

#### Scenario: Explicit resource is inspected after resume

- **GIVEN** a compatible resumed session references an explicit cache resource
- **WHEN** the adapter inspects it under current authority
- **THEN** the typed result may update current expiry or existence evidence
- **AND** no raw cache content enters Smith state or diagnostics

#### Scenario: Shutdown releases an explicit resource

- **GIVEN** adapter policy requires deletion of a session-owned explicit cache
  resource
- **WHEN** bounded shutdown begins
- **THEN** Smith attempts the typed delete within the shutdown deadline
- **AND** records a redaction-safe outcome without delaying lifecycle release
  indefinitely

### Requirement: Synthetic provider responses cannot execute tools

Keepalive and handoff-checkpoint requests SHALL preserve tool definitions only
when they are already identity-bound stable-prefix material, force tool choice
to none, and expose no tool execution, mutation, interaction, delegation, or
host side-effect capability. If a provider nevertheless returns a tool call or
side-effect-shaped output, Agent Runtime and Smith SHALL fail the synthetic
attempt without executing or approving it.

#### Scenario: Provider emits a tool call during handoff

- **GIVEN** a handoff request was constructed with tools disabled
- **WHEN** the provider emits a tool call
- **THEN** the attempt fails with a provider contract violation
- **AND** no tool preparation, approval, or execution occurs

#### Scenario: Ordinary parent tools remain unchanged

- **GIVEN** a later real parent continuation advertises its ordinary scoped
  tool view
- **WHEN** Agent Runtime constructs that request
- **THEN** the prior synthetic no-execution shape does not mutate the parent's
  canonical ability epoch
- **AND** normal tool validation remains governed by the parent runtime
