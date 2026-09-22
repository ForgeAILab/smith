## MODIFIED Requirements

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
