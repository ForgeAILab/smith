## ADDED Requirements

### Requirement: Truthful host-shell authority

Smith SHALL, until it supplies an enforceable operating-system sandbox, make
the `shell` tool declare an unrestricted same-user host-shell resource rather
than a workspace filesystem resource. Its approval presentation MUST state that the
command can access host files, inherited environment and credentials, child
processes, the network, and data egress outside the project.

#### Scenario: Shell cwd is inside the project

- **GIVEN** a shell call whose current directory resolves beneath the project
- **WHEN** Smith prepares the call without an operating-system sandbox
- **THEN** the security resource identifies host-shell execution
- **AND** it does not claim that the process is filesystem-contained by its cwd

#### Scenario: Headless shell lacks unrestricted policy

- **GIVEN** a headless run without explicit `allow-all` authority
- **WHEN** the model requests a shell command
- **THEN** the run returns a structured approval-required outcome
- **AND** no command process is spawned

### Requirement: Handle-relative workspace enforcement

Built-in filesystem tools SHALL enforce the project boundary through a
directory capability and handle-relative operations. Canonical pathname
validation MAY provide display and preliminary diagnostics but MUST NOT be the
final containment primitive, and a bounded read MUST inspect and consume one
opened object through a `max_bytes + 1` limiter.

#### Scenario: Symlink is swapped after validation

- **GIVEN** a repository path validated beneath the workspace
- **AND** an adversary replaces a traversed component with an escaping symlink
- **WHEN** the built-in tool opens or mutates the target
- **THEN** the handle-relative operation refuses the escape
- **AND** no object outside the workspace is read or changed

#### Scenario: Absolute path is outside the project

- **GIVEN** a built-in read, list, search, or edit call names an absolute path
  outside the workspace
- **WHEN** Smith prepares the ordinary filesystem tool
- **THEN** it refuses the call rather than routing it to approval
- **AND** host access remains available only through a separately labeled host
  capability

#### Scenario: File grows after metadata inspection

- **GIVEN** a file whose initial metadata is within the read limit
- **WHEN** the object grows while Smith reads it
- **THEN** Smith reads no more than `max_bytes + 1`
- **AND** returns the oversized-file outcome without retaining the full object

### Requirement: Version-checked metadata-preserving edits

A full read SHALL produce a version token from the same file handle as the
returned bytes. Overwrite, delete, and exact replacement MUST revalidate file
identity, size, modification time, and content hash immediately before commit;
a mismatch MUST leave the newer object unchanged. Replacing an existing file
MUST preserve the platform's required security-relevant metadata, including
permission and executable bits, and MUST durably order temporary-file and
parent-directory synchronization where supported.

#### Scenario: User edits during an approval prompt

- **GIVEN** Smith fully read a file and prepared an overwrite
- **AND** the user changes the file before approving the call
- **WHEN** invocation reaches the commit boundary
- **THEN** version revalidation refuses the overwrite as stale
- **AND** the user's newer bytes remain unchanged

#### Scenario: Changed bytes retain the same modification time

- **GIVEN** the target's bytes changed after Smith read it
- **AND** its reported modification time was restored to the prior value
- **WHEN** Smith revalidates the destructive edit
- **THEN** the content hash or file identity mismatch refuses the commit

#### Scenario: Replace an executable script

- **GIVEN** an existing executable script with supported security metadata
- **WHEN** an approved exact replacement or overwrite succeeds
- **THEN** its permission and executable bits are preserved
- **AND** required metadata is not silently discarded by the atomic rename
