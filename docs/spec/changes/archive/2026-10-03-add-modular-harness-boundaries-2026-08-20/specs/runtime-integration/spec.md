## MODIFIED Requirements

### Requirement: Shared runtime is the canonical mechanism

Smith SHALL use the versioned `agent-runtime` facade as the sole owner of model
and context planning, provider normalization, the provider/tool loop, tool
execution, cancellation, canonical runtime events, usage accounting, and
registry mechanism. Smith MUST NOT maintain a parallel implementation of those
behaviors. Smith SHALL expose a versioned product-level session and event
projection to presentation clients, but that projection MUST adapt the shared
mechanism and MUST NOT become a second execution loop, journal, or source of
canonical state.

#### Scenario: Smith executes a configured turn

- **GIVEN** Smith has resolved product configuration and host policy
- **WHEN** it starts a session and sends user input
- **THEN** the turn executes through `agent-runtime::Runtime`
- **AND** canonical persistence consumes shared runtime events
- **AND** presentation clients consume the versioned Smith projection

#### Scenario: Shared mechanism needs a new capability

- **GIVEN** Smith needs a provider or execution mechanism absent from the
  pinned runtime
- **WHEN** the ownership boundary is evaluated
- **THEN** the mechanism is proposed and implemented in Agent Runtime first
- **AND** Smith consumes it only after a compatible version is available

## ADDED Requirements

### Requirement: Immutable resolved harness composition

Smith SHALL resolve declarative harness input into one immutable,
provenance-bearing `ResolvedHarness` before runtime construction. The record
MUST include identity, provider/model, modules, trust, contributions, requested
and granted capabilities, approval, persistence, context, and delegation
policy; the public composition root MUST accept the resolved record rather than
re-resolving raw files, environment values, or mutable host options.

#### Scenario: Two hosts resolve identical input

- **GIVEN** TUI and headless hosts receive equivalent declarations and host
  services
- **WHEN** each resolves its harness
- **THEN** their `ResolvedHarness` policy records compare equivalent
- **AND** both pass through the same public factory

#### Scenario: Module contribution exceeds its grant

- **GIVEN** a module declares a tool that requires network authority
- **AND** the host grants the module no network capability
- **WHEN** Smith resolves the harness
- **THEN** resolution refuses or disables that contribution before runtime
  construction
- **AND** the declaration itself grants no authority
