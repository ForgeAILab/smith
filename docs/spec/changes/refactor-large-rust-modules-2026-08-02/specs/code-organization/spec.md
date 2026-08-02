## ADDED Requirements

### Requirement: Large Rust modules have stable responsibility boundaries

Smith SHALL preserve its existing public module paths and runtime behavior
while decomposing large Rust source files into cohesive private modules. Each
facade MUST retain compatibility exports and orchestration only, and extracted
implementation MUST use the narrowest visibility required for sibling
coordination.

#### Scenario: Existing public imports continue to compile

- **GIVEN** downstream code imports public items from `smith_tui::app`,
  `smith_config::resolve`, or `smith_runtime::factory`
- **WHEN** the owning source file is decomposed into child modules
- **THEN** the same import paths and public signatures continue to compile
- **AND** no private item becomes public solely to support the extraction

#### Scenario: TUI behavior survives decomposition

- **GIVEN** the recorded reducer, input, prompt, rendering, and end-to-end
  fixtures at the implementation baseline
- **WHEN** application and renderer responsibilities move to private modules
- **THEN** live and replay reduction produce the same visible state
- **AND** keyboard ownership, queue ordering, prompt safety, scrolling,
  redaction, and rendered snapshots remain unchanged

#### Scenario: Configuration behavior survives decomposition

- **GIVEN** the recorded resolver and downstream configuration fixtures
- **WHEN** types, provenance, loading, agent, and provider resolution move to
  private modules
- **THEN** layer precedence, source attribution, validation, error text, and
  serialized representations remain unchanged

#### Scenario: CLI retains one runtime composition path

- **GIVEN** interactive, headless, setup, explain, list, and resume commands
- **WHEN** host, terminal-driver, local-command, submission, and resource code
  moves out of `main.rs`
- **THEN** every runtime still starts through `smith_runtime::host`
- **AND** command results, interruption, restart, and shutdown behavior remain
  equivalent to the baseline

#### Scenario: Factory build becomes staged without policy drift

- **GIVEN** the existing `RuntimeRequest` and factory test matrix
- **WHEN** `factory::build` is expressed as ordered typed preparation and
  assembly stages
- **THEN** validation order, credential handling, provider wrapping, tool and
  ability order, checkpoint durability, contributors, hooks, delegation, and
  final `RuntimePolicy` remain equivalent
- **AND** a physical module split occurs only when it does not widen internal
  visibility
