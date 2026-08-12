## MODIFIED Requirements

### Requirement: Operational status in the TUI

The TUI SHALL display current provider/model, token and provenance status,
cache state, active monitors, direct children, running background shell tasks,
and queued notifications. An estimated or unknown value MUST be visually
distinguishable from a provider-reported value.

#### Scenario: Provider switch leaves estimated context

- **GIVEN** the user switches to a provider that has not reported usage
- **WHEN** the status line updates
- **THEN** it labels context tokens estimated
- **AND** does not reuse the prior provider's verified cache indicator

#### Scenario: Running background task is visible

- **GIVEN** a background shell task is running
- **WHEN** the user views operational status
- **THEN** the task is listed as active work with its task ID
- **AND** it disappears from active work after its terminal notification

### Requirement: Explicit active-work exit policy

The TUI MUST request confirmation before exiting with active monitors,
children, or background shell tasks. Non-interactive mode SHALL support
`error`, `wait`, and `stop` background-exit policies and MUST default to
`error`.

#### Scenario: Headless turn finishes with a persistent monitor

- **GIVEN** the final answer is ready while a persistent monitor remains
- **WHEN** the caller did not choose an exit policy
- **THEN** Smith emits an active-work error describing the monitor
- **AND** does not silently orphan it

#### Scenario: Headless turn finishes with a running background task

- **GIVEN** the final answer is ready while a background shell task remains
- **WHEN** the caller chose the `wait` background-exit policy
- **THEN** Smith waits for the task's terminal state before exiting
- **AND** reports its terminal state in machine output
