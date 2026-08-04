## ADDED Requirements

### Requirement: No-prompt protected checkpoint initialization

When an explicit inline or environment checkpoint key resolves, Smith SHALL
initialize the existing authenticated-encrypted checkpoint store without
opening or querying an operating-system credential service. The key source
changes protection only; checkpoint schema, exact-state boundaries, journal
watermarks, atomicity, and no-plaintext-fallback rules remain unchanged.

#### Scenario: Resume with an inline key
- **GIVEN** a session checkpoint was encrypted under the configured inline key
- **WHEN** Smith restarts and resumes it
- **THEN** it decrypts and validates the exact compatible state without an OS
  credential prompt
- **AND** does not repeat committed provider or tool work

#### Scenario: Configured key is wrong
- **GIVEN** a checkpoint envelope cannot authenticate under the resolved key
- **WHEN** Smith loads it
- **THEN** recovery fails closed with a redacted key-mismatch/integrity result
- **AND** does not delete, replace, or treat the checkpoint as empty

### Requirement: Atomic checkpoint-key rotation

Changing an established checkpoint key SHALL either re-encrypt every selected
compatible checkpoint atomically under an exclusive lease or refuse before
modification. Smith MUST retain a recoverable prior state until the complete
rotation commits.

#### Scenario: Rotation succeeds
- **GIVEN** every selected checkpoint validates under the old key
- **WHEN** the user confirms rotation to a new source
- **THEN** Smith publishes all re-encrypted envelopes and config as one
  recoverable transaction
- **AND** zeroizes old and new working key material after commit

#### Scenario: One checkpoint is corrupt
- **GIVEN** any selected envelope fails validation
- **WHEN** rotation preflight runs
- **THEN** no checkpoint or config value changes
- **AND** Smith reports the affected session without secret material

### Requirement: User-state-only agent metadata

Smith SHALL keep agent modes, child navigation, live-work reconstruction,
timeline records, and prepared composer recovery in process memory, canonical journals,
protected checkpoints, or owner-only user state. Smith MUST NOT create
continuation or agent-control metadata in the project checkout.

#### Scenario: Complete an agent-heavy coding turn
- **GIVEN** Smith uses todos, file references, shell, and a child reviewer
- **WHEN** the turn and session complete
- **THEN** `git status` shows only task-requested project changes
- **AND** no `.smith`, `.omo`, timeline, session, or continuation metadata is
  present in the checkout
