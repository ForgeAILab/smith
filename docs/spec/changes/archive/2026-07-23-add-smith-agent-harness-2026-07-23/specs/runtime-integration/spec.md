## ADDED Requirements

### Requirement: Shared runtime is the canonical mechanism

Smith SHALL use the versioned `agent-runtime` facade as the sole owner of model
and context planning, provider normalization, the provider/tool loop, tool
execution, cancellation, runtime events, usage accounting, and registry
mechanism. Smith MUST NOT maintain a parallel implementation or public contract
for those behaviors.

#### Scenario: Smith executes a configured turn

- **GIVEN** Smith has resolved product configuration and host policy
- **WHEN** it starts a session and sends user input
- **THEN** the turn executes through `agent-runtime::Runtime`
- **AND** the TUI and persistence layers consume shared messages and events

#### Scenario: Shared mechanism needs a new capability

- **GIVEN** Smith needs a provider or execution mechanism absent from the
  pinned runtime
- **WHEN** the ownership boundary is evaluated
- **THEN** the mechanism is proposed and implemented in Agent Runtime first
- **AND** Smith consumes it only after a compatible version is available

### Requirement: Versioned dependency with local override

A releasable Smith manifest MUST depend on Agent Runtime through an exact
semantic version or exact Git revision. A sibling path MAY be used only through
an uncommitted, git-ignored Cargo patch that can be removed without source
changes.

#### Scenario: Build Smith without sibling checkouts

- **GIVEN** only the Smith repository and its declared package sources are
  available
- **WHEN** a normal release build resolves dependencies
- **THEN** it obtains the pinned Agent Runtime source
- **AND** it does not require `../agent-runtime`

#### Scenario: Develop Smith and Agent Runtime together

- **GIVEN** both repositories are sibling checkouts
- **WHEN** a developer enables the documented local Cargo patch
- **THEN** Smith resolves the runtime from the local path
- **AND** removing the patch restores the pinned source without Rust or
  manifest dependency-line edits

### Requirement: Complete preflight runtime composition

Before terminal entry or provider network I/O, Smith SHALL resolve and validate
the selected provider, credential reference, model, model profile or catalog,
context policy, product prompt, loop limits, tools, approval policy, workspace,
stores, observers, and shutdown policy. Smith MUST fail closed when any required
input is missing or inconsistent.

#### Scenario: Model limits are missing

- **GIVEN** the selected model has no explicit profile and no catalog source
  provides enforceable limits
- **WHEN** Smith prepares the runtime
- **THEN** startup fails with a model-profile diagnostic
- **AND** no provider request is sent
- **AND** the terminal is not left in raw or alternate-screen mode

#### Scenario: Resolved composition is valid

- **GIVEN** every required provider, model, context, and host-policy value
  resolves
- **WHEN** Smith builds the runtime
- **THEN** it maps those values through `RuntimeBuilder`
- **AND** the emitted model-profile and planning events identify the resolved
  provider, model, revisions, limits, and provenance

### Requirement: One Smith composition path

Smith SHALL use one runtime factory for the interactive TUI, non-interactive
CLI, deterministic tests, direct child sessions, and future Forge adapter.
Host-specific presentation MAY differ, but runtime behavior MUST NOT be
reimplemented at an entry point.

#### Scenario: Compare TUI and headless execution

- **GIVEN** identical resolved configuration, host adapters, and fake-provider
  input
- **WHEN** a turn runs through the TUI and `smith -p`
- **THEN** both construct equivalent shared runtime policy
- **AND** their canonical shared events and usage differ only in declared
  presentation metadata

### Requirement: Coordinated runtime compatibility gate

Smith SHALL maintain integration tests for its resolved builder composition and
participate in Agent Runtime's Smith consumer conformance gate. A compatible
runtime update MUST NOT be accepted while either gate fails.

#### Scenario: Runtime adds a required model-profile field

- **GIVEN** a candidate runtime revision changes construction or event behavior
- **WHEN** the Smith integration and shared consumer suites run
- **THEN** any missing Smith mapping fails before the dependency is updated
- **AND** the migration is documented rather than hidden by permissive defaults
