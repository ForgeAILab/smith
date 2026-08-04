## ADDED Requirements

### Requirement: Coordinated stable runtime pipeline

Every Smith surface SHALL use the one Smith runtime factory over a compatible
Agent Runtime release implementing session-scoped planning, prepared
invocations, attempt-scoped output, structured turn control, checkpoints, and
activation epochs. Smith MUST NOT retain fallback copies of those mechanisms.

#### Scenario: TUI and headless run the same fixture
- **GIVEN** identical resolved Smith policy and fake-provider input
- **WHEN** the TUI and headless hosts execute the fixture
- **THEN** their canonical runtime semantics and committed event sequence are
  equivalent
- **AND** only presentation projections differ

### Requirement: Smith built-ins use ability activation

Smith SHALL register its built-in coding tools and standard harness components
through Agent Runtime abilities with accurate affordances, typed permission
upper bounds, risk, context cost, readiness, provenance, and revision. The
provider tool surface MUST be materialized from a frozen activation epoch.

#### Scenario: Read-only repository question
- **GIVEN** read and mutation abilities are installed
- **WHEN** deterministic retrieval classifies the request as read-only
- **THEN** the active epoch contains the dependency-complete read subset
- **AND** edit and shell are not advertised merely because they are installed

### Requirement: One product composition path

Smith SHALL map prompt sections, ability sources, approval, interaction,
workspace, stores, artifacts, memory, tools, provider, model, clock, and
observers through `smith-runtime::factory` for terminal, headless, child, test,
and embedded surfaces.

#### Scenario: Child runtime is constructed
- **GIVEN** a root agent delegates a read-only child
- **WHEN** Smith constructs the child runtime
- **THEN** it uses the same factory and shared harness mechanism
- **AND** product policy narrows delegation and mutation abilities without
  creating a second execution loop
