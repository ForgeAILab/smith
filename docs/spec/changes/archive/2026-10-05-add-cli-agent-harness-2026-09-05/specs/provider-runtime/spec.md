## ADDED Requirements

### Requirement: Harness adapters drive installed coding agents

Smith SHALL provide an external agent backend for each supported harness,
translating one turn into a bounded CLI invocation and normalizing that CLI's
machine output into the runtime's external agent events.

An adapter MUST report the CLI's own session identity so the next turn
continues that conversation instead of replaying history, MUST report usage
with its cache breakdown where the CLI provides one, and MUST treat a CLI's
self-reported error as a failed turn rather than inferring success from a zero
exit code.

#### Scenario: Turn continues the CLI conversation

- **GIVEN** a completed harness turn that reported a session identity
- **WHEN** the next turn runs
- **THEN** the adapter resumes that session
- **AND** sends only the new input rather than the whole history

#### Scenario: CLI reports failure with a zero exit code

- **GIVEN** a CLI that exits zero while reporting an error in its output
- **WHEN** the adapter normalizes that output
- **THEN** the turn fails with the reported reason

#### Scenario: CLI runs a tool itself

- **GIVEN** a harness permitted to run its own tools
- **WHEN** the CLI reports invoking one and its outcome
- **THEN** the adapter emits external tool observation events
- **AND** no Smith approval is requested
