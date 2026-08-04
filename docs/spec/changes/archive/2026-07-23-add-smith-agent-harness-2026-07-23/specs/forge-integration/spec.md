## ADDED Requirements

### Requirement: Embeddable in-process runtime

Smith SHALL expose an in-process composition API that returns and controls
Agent Runtime's `Runtime`/`SessionHandle` facade without launching the TUI or a
daemon. Hosts MUST be able to supply resolved Smith configuration and inject
shared provider, tool, approval, workspace, session, credential, observer, and
clock implementations through documented Rust contracts.

#### Scenario: Forge-like host embeds Smith

- **GIVEN** a host supplies its own workspace and approval adapters
- **WHEN** it creates a Smith runtime and starts a session
- **THEN** Agent Runtime's canonical loop uses those adapters
- **AND** the host receives the same shared typed events as the CLI/TUI

### Requirement: One-way dependency boundary

Reusable Smith crates and Agent Runtime MUST NOT depend on Forge crates, Forge
task types, or Forge business rules. A Forge integration SHALL map Forge-owned
concepts onto shared runtime contracts and Smith policy in a Forge-owned adapter
layer.

#### Scenario: Build Smith without Forge

- **GIVEN** no Forge source or dependency is available
- **WHEN** the Smith workspace builds and tests
- **THEN** all standalone runtime, CLI, and TUI capabilities remain available

### Requirement: Shared runtime semantics across hosts

The TUI, non-interactive CLI, embedding tests, and future Forge adapter SHALL
use the same Smith runtime factory over Agent Runtime. Smith-level monitor,
cache-lifetime, and child orchestration MUST wrap that composition rather than
reimplement shared agent, provider, context, tool, event, or usage mechanism.

#### Scenario: Compare CLI and embedded turn

- **GIVEN** identical fake-provider input and resolved configuration
- **WHEN** a turn runs through `smith -p` and an embedded host
- **THEN** their canonical session events and usage records are equivalent
  apart from host presentation metadata

### Requirement: No MVP daemon dependency

Smith MUST NOT require a local daemon or IPC service for standalone or embedded
operation in this change. Active work SHALL belong to the constructing process.
A future daemon requires a separate approved proposal.

#### Scenario: Run with no background service

- **GIVEN** no Smith process is already running
- **WHEN** a user launches the TUI or a host embeds the runtime
- **THEN** the new process can perform every MVP capability
- **AND** all active monitors and children stop with that process

### Requirement: Separate Forge rollout authorization

This Smith change SHALL include only host contracts, examples, and tests inside
the Smith repository. Modifying Open Forge or replacing a Forge executor MUST
require a separate Forge proposal and opt-in rollout.

#### Scenario: Smith reaches embedding readiness

- **GIVEN** Smith's embedding contract tests pass
- **WHEN** the Smith change is completed
- **THEN** Open Forge remains unchanged
- **AND** the integration evidence is available for a later Forge change
