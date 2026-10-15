## MODIFIED Requirements

### Requirement: Explicit active-work exit policy

The TUI's `/quit` SHALL exit immediately, without confirmation, and stop any
active turn, pending prompt, monitor, child, or background shell task; the
exit output MUST name the work it stopped. Non-interactive mode SHALL support
`error`, `wait`, and `stop` background-exit policies and MUST default to
`error`.

#### Scenario: Quit during a turn

- **GIVEN** a turn is streaming and an approval is pending
- **WHEN** the user types `/quit`
- **THEN** the TUI exits without opening a confirmation
- **AND** the pending approval is cancelled, not granted

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
