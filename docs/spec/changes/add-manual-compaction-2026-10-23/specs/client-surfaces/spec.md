## ADDED Requirements

### Requirement: Explicit conversation compaction command

Smith SHALL expose `/compact` through the shared
command registry. Running it without arguments SHALL request one
protected semantic-compaction attempt at an idle session boundary. It SHALL
reuse the existing summary policy, persistence, usage attribution, and
once-per-idle-interval admission, without submitting a conversational turn.

#### Scenario: A user compacts eligible history

- **WHEN** an idle user runs `/compact` with eligible completed history
- **THEN** Smith attempts semantic compaction and reports its outcome
- **AND** recent canonical turns remain available under the existing retention policy
- **AND** the command is discoverable in help, completion, and the command palette

#### Scenario: Compaction cannot run

- **WHEN** the session is busy, shutting down, already attempted for the interval, or lacks eligible history
- **THEN** Smith reports the condition without creating a conversation turn or automatically retrying

#### Scenario: A summary attempt fails

- **WHEN** manual semantic compaction fails
- **THEN** Smith reports the failure and preserves original canonical history and the previously committed summary
