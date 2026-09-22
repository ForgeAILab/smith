# code-organization Specification

## Purpose
TBD - created by archiving change refactor-large-rust-modules. Update Purpose after archive.
## Requirements
### Requirement: Large Rust modules have stable responsibility boundaries

Smith SHALL preserve existing module paths and behavior while decomposing
large Rust source files into cohesive private modules. A child module SHALL
own a complete responsibility or stage, and implementation visibility SHALL
be limited to the narrowest scope needed for coordination.

#### Scenario: Cache lifecycle work is decomposed without changing scheduling

- **GIVEN** the cache controller's scheduler, idle-compaction lane,
  handoff-persistence path, and resume-capsule event projection
- **WHEN** idle compaction, handoff persistence, and capsule event projection
  move into private child modules
- **THEN** `smith_runtime::cache_controller` retains its existing public
  exports and controller orchestration
- **AND** admission boundaries, cancellation, Runtime event order, capsule
  persistence rollback, and synthetic-attempt accounting remain unchanged

#### Scenario: Headless output and event flow are decomposed without contract changes

- **GIVEN** a headless run using text, JSON, or stream JSON output and any
  configured background-exit policy
- **WHEN** output projection, background-exit handling, and event-stream flow
  move into private child modules
- **THEN** the existing `headless::run` entry point remains available to the
  CLI
- **AND** serialized fields, output ordering, diagnostics, background task
  states, and exit codes remain unchanged

#### Scenario: Extracted modules do not widen internal APIs

- **GIVEN** sibling orchestration calls an extracted stage
- **WHEN** private child modules are introduced
- **THEN** only the visibility required between the parent and those children
  is added
- **AND** no item becomes public solely to support the refactor

### Requirement: Factory and application state have physical ownership boundaries

Smith SHALL place the implementation of a factory stage in the private module
that names that stage, and SHALL keep TUI child-session presentation methods
in a cohesive private application module. The public factory and `App` paths
and signatures MUST remain stable. The extraction MUST preserve observable
runtime and TUI behavior.

#### Scenario: Factory stage implementation is owned by its module

- **GIVEN** provider, credential, and authority preparation is performed during
  `factory::build`
- **WHEN** factory source is decomposed
- **THEN** the corresponding stage modules contain the implementation rather
  than only forwarding to functions in `factory.rs`
- **AND** validation, credential, tool, and composition order is unchanged

#### Scenario: Child presentation moves without changing App behavior

- **GIVEN** the TUI tracks child conversations, counts, inspection, and
  dismissal in `App`
- **WHEN** those methods move to a private child module
- **THEN** existing `App` imports and method calls compile
- **AND** live and replayed events produce the same child presentation
