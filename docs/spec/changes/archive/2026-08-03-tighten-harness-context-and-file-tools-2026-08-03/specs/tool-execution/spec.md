## ADDED Requirements

### Requirement: Explicit edit operations

The `edit` tool SHALL accept an explicit `operation` of `replace`, `create`,
`overwrite`, or `delete`, defaulting to `replace` when absent. Each operation
MUST request only the permissions it needs: `replace` and `overwrite` request
filesystem read and write, `create` requests filesystem create and write, and
`delete` requests filesystem read and delete. An empty `old_string` MUST
continue to mean `create` so existing transcripts replay unchanged.

#### Scenario: Overwrite replaces a file without echoing its contents
- **GIVEN** an existing project file the session has read in full
- **WHEN** the model calls `edit` with `operation` `overwrite` and a new body
- **THEN** the file contains exactly the new body
- **AND** the call did not require the previous contents as an argument

#### Scenario: Create still refuses an existing target
- **GIVEN** an existing project file
- **WHEN** the model calls `edit` with `operation` `create`
- **THEN** the call fails
- **AND** the existing file is unchanged

#### Scenario: Delete requests the narrow permission
- **GIVEN** a prepared `edit` call with `operation` `delete`
- **WHEN** the prepared action is presented for authorization
- **THEN** it requests filesystem read and delete only
- **AND** it requests neither process spawn nor network

#### Scenario: A legacy empty old_string still creates
- **GIVEN** a recorded call passing an empty `old_string` and no `operation`
- **WHEN** it is replayed
- **THEN** the file is created exactly as before this change

### Requirement: Destructive operations require a current full read

`overwrite` and `delete` SHALL be refused unless the session has already read
the exact target path in full during this session, and the file's modification
time is not newer than that read. A partial read using an offset or a limit MUST
NOT satisfy the precondition. The refusal MUST name which condition failed.

#### Scenario: Overwrite without a prior read is refused
- **GIVEN** an existing file the session has not read
- **WHEN** the model calls `edit` with `operation` `overwrite`
- **THEN** the call fails with a message saying the file must be read first
- **AND** the file is unchanged

#### Scenario: A partial read does not authorize an overwrite
- **GIVEN** a file the session read with an `offset` and a `limit`
- **WHEN** the model calls `edit` with `operation` `overwrite`
- **THEN** the call fails
- **AND** the message distinguishes a partial view from an unread file

#### Scenario: An external modification invalidates the read
- **GIVEN** a file the session read in full
- **AND** the file was subsequently modified outside Smith
- **WHEN** the model calls `edit` with `operation` `overwrite`
- **THEN** the call fails with a message saying the file changed since it was
  read
- **AND** the external modification is preserved

#### Scenario: Exact replacement keeps its existing contract
- **GIVEN** a file the session has never read
- **WHEN** the model calls `edit` with `operation` `replace` and an
  `old_string` that matches exactly once
- **THEN** the edit applies
- **AND** no read precondition is imposed

### Requirement: Deletion is attributed like any other mutation

A completed `delete` SHALL be recorded in the turn's change set with the same
attribution as an exact edit, retaining the pre-image needed for conflict
checked undo and recording only hashes and path metadata in the persisted
journal.

#### Scenario: A deleted file can be undone
- **GIVEN** a file deleted by an `edit` call in the current session
- **WHEN** the user undoes that turn's changes
- **THEN** the file is restored with its exact previous contents

#### Scenario: The journal records no file contents
- **GIVEN** a completed `delete`
- **WHEN** the session journal is written
- **THEN** it contains the path metadata and content hashes
- **AND** it contains no file body
