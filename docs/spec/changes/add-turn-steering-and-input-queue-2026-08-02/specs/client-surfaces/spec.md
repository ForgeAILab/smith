## ADDED Requirements

### Requirement: Pending user input is visibly distinguished

The interactive TUI SHALL render bounded, text-labelled previews for pending
steers, rejected-steer follow-ups, and explicit future turns in the existing
anchored composer region. It MUST distinguish process-local pending state from
canonical transcript history and MUST remain understandable without color.

#### Scenario: Steer waits for a safe boundary

- **GIVEN** an accepted steer has not yet committed
- **WHEN** the TUI renders the busy surface
- **THEN** it labels the input as pending for the active turn
- **AND** shows the interrupt-for-steer hint without adding a canonical user row

#### Scenario: Several future turns are queued

- **GIVEN** queued previews exceed the per-section line budget
- **WHEN** the TUI renders at a supported terminal size
- **THEN** it shows the bounded leading previews and an overflow count
- **AND** does not displace the composer or create an unbounded pane

#### Scenario: Todo and pending input coexist

- **GIVEN** public todo state and pending user input both exist
- **WHEN** no modal or picker owns the anchored area
- **THEN** the renderer allocates bounded rows to both within the existing
  anchored budget
- **AND** cursor placement remains attached to the composer

### Requirement: Busy key guidance matches behavior

The TUI and `/help` SHALL describe the conditional `Enter`, `Tab`, `Alt+Up`,
and `Esc` behavior while work is serving. Idle profile cycling and overlay
selection hints MUST remain accurate in their respective states.

#### Scenario: Ordinary prompt is ready during work

- **GIVEN** eligible work is serving and an ordinary draft is non-empty
- **WHEN** Smith renders composer guidance
- **THEN** the guidance identifies `Enter` as steer and `Tab` as queue
- **AND** identifies the configured queued-input edit action when a future turn
  exists
