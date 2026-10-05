## ADDED Requirements

### Requirement: Forked sessions copy history and start their own state

Smith SHALL be able to fork an idle session into a new session whose canonical
history and history-derived extension state are copies of the parent's. The
fork MUST start with its own session id, event journal, usage ledger,
checkpoint, background task spool, and change attribution journal, and MUST
NOT inherit approval grants scoped to the parent. Smith SHALL record the
parent id and fork point beside the fork's snapshot. The parent MUST be left
unchanged.

#### Scenario: Fork leaves the original intact

- **GIVEN** an idle session with five turns
- **WHEN** it is forked
- **THEN** a new session exists with the same five turns of history
- **AND** its usage ledger is empty and its change journal does not exist
- **AND** the original session's files are unchanged and it is still resumable

#### Scenario: Undo stays in its own session

- **GIVEN** a fork of a session that edited `a.rs`
- **WHEN** the user runs `/undo` in the fork
- **THEN** nothing is reverted, because the fork has made no edits

#### Scenario: Fork refused while work is running

- **GIVEN** a session with a running child agent
- **WHEN** the user forks it
- **THEN** Smith refuses and names the running child

### Requirement: Forks read their ancestors' artifacts

An artifact read SHALL succeed when the requesting session owns the artifact or
when the owner is an ancestor in the requesting session's recorded fork
lineage. Any other session MUST be refused. Forking MUST NOT rewrite artifact
references in the copied history.

#### Scenario: Fork reads parent tool output

- **GIVEN** a parent whose history references offloaded shell output
- **WHEN** the fork reads that artifact
- **THEN** the read succeeds

#### Scenario: Sibling fork is refused

- **GIVEN** two forks of the same parent
- **WHEN** one fork reads an artifact the other created
- **THEN** the read is refused
