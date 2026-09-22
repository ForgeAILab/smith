## ADDED Requirements

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
