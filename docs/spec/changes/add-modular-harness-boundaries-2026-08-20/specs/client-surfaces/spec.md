## ADDED Requirements

### Requirement: Versioned Smith client protocol

TUI, headless, GPUI, Forge, and other presentation clients SHALL drive sessions
through Smith-owned versioned session commands, receipts, IDs, and event
projections. Clients MUST NOT depend on the concrete Agent Runtime session
handle or event enum, while canonical persistence and execution MUST remain on
Agent Runtime behind the Smith adapter.

#### Scenario: Agent Runtime adds an event variant

- **GIVEN** a compatible Agent Runtime revision adds a canonical event
- **WHEN** Smith updates its adapter
- **THEN** Smith explicitly maps, bounds, or intentionally omits that event
- **AND** unchanged clients continue to consume their supported Smith protocol
  version

#### Scenario: TUI and GPUI observe one session

- **GIVEN** two Smith clients subscribe to the same composed session
- **WHEN** a turn streams text, prepares a tool, requests approval, and finishes
- **THEN** both receive causally ordered Smith events with stable Smith IDs
- **AND** neither client receives a direct runtime handle or mutable runtime
  internals

#### Scenario: Session is resumed from canonical state

- **GIVEN** Smith resumes Agent Runtime canonical events and snapshots
- **WHEN** a client subscribes after reconstruction
- **THEN** Smith rebuilds the same bounded client projection
- **AND** no Smith client event is treated as an independent canonical journal
