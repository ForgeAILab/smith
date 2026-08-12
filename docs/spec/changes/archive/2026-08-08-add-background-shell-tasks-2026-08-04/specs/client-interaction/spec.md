## ADDED Requirements

### Requirement: Manual backgrounding of a running foreground command

While a foreground `shell` call is running, the TUI SHALL offer a dedicated
backgrounding action distinct from interrupt. Invoking it MUST NOT kill the
process: Smith adopts the running process group as a background task, and the
pending tool call resolves promptly as a non-error outcome carrying the output
captured so far, the new task ID, and an explicit statement that the user
moved the command to the background so the model does not read the outcome as
completion. Subsequent lifecycle MUST be identical to a task started with
`run_in_background: true`. The interrupt action keeps its existing
cancel-and-kill semantics, and headless mode has no backgrounding affordance.

#### Scenario: User backgrounds a slow foreground command

- **GIVEN** a foreground shell command has been running in the TUI
- **WHEN** the user invokes the backgrounding action
- **THEN** the tool call resolves with the output so far and a task ID
- **AND** the outcome states the user moved the command to the background
- **AND** the process keeps running as a background task

#### Scenario: Interrupt still kills rather than backgrounds

- **GIVEN** a foreground shell command is running
- **WHEN** the user invokes the interrupt action
- **THEN** the owned process group is terminated
- **AND** no background task is created
