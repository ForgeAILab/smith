## ADDED Requirements

### Requirement: Fixed file command directory layout

Smith SHALL read file commands from `commands/<name>.md` beneath the user
state root and beneath the project's `.smith/` directory, with the file stem
as the command's name. The layout MUST be fixed rather than configurable,
Smith MUST NOT create either directory, and frontmatter MUST NOT be able to
rename a command. A name MUST be 1 to 64 lowercase ASCII letters, digits, or
hyphens.

#### Scenario: Author adds a user command

- **GIVEN** the user creates `commands/audit.md` under the user state root
- **WHEN** Smith next starts in any project
- **THEN** `/audit` is available in that session

#### Scenario: Frontmatter cannot rename a command

- **GIVEN** `commands/audit.md` whose frontmatter declares a different `name`
- **WHEN** discovery runs
- **THEN** the command is not registered
- **AND** the mismatch is reported as a named discovery problem
- **AND** no command is registered under the frontmatter's name

#### Scenario: Nothing on disk

- **GIVEN** neither location contains a `commands` directory
- **WHEN** Smith starts
- **THEN** only built-in commands exist
- **AND** no directory is created

### Requirement: File command discovery is bounded and fails closed per file

A command file Smith cannot use SHALL be excluded and reported as a named
discovery problem. Discovery MUST be bounded in the number of commands per
layer, the size of a file, and the length of a description, MUST refuse a name
that collides with a built-in command, and MUST NOT fail startup because a
command file is malformed.

#### Scenario: One malformed file among several

- **GIVEN** three command files, one with unterminated frontmatter
- **WHEN** Smith starts
- **THEN** the session starts and the two well-formed commands are available
- **AND** the malformed one is reported by name with its reason

#### Scenario: Invalid or reserved name

- **GIVEN** files named `My Command.md` and `quit.md` in a commands directory
- **WHEN** discovery runs
- **THEN** neither is registered
- **AND** each is reported with the reason it was refused

#### Scenario: Non-command content beside the commands

- **GIVEN** a `commands` directory containing a subdirectory and a file that
  does not end in `.md`
- **WHEN** discovery runs
- **THEN** neither is registered
- **AND** neither is reported as a problem

#### Scenario: A bound is exceeded

- **GIVEN** a command file larger than the read bound, or more files in one
  layer than the count bound
- **WHEN** discovery runs
- **THEN** the excluded commands are not registered
- **AND** the exclusion is reported rather than silently truncated

### Requirement: Project file commands are hash-bound executable trust

Smith's project-trust model SHALL cover project-supplied command templates as
a distinct kind of executable authority, decided per file content and
persisted alongside the existing kinds. A project command MUST NOT run, and
MUST NOT shadow a user command of the same name, until the decision covering
its exact content is recorded. A project command file that resolves outside
the project root MUST be refused.

#### Scenario: Project ships an unreviewed command

- **GIVEN** a project supplies `.smith/commands/deploy.md` nobody has approved
- **WHEN** Smith starts
- **THEN** the command is listed as needing approval and does not run
- **AND** a user command named `deploy` still runs

#### Scenario: Trusted project command shadows a user command

- **GIVEN** a user command and a trusted project command with the same name
- **WHEN** the user invokes that name
- **THEN** the project command's body is used
- **AND** the listing identifies the user command as shadowed

#### Scenario: Decision binds path and content together

- **GIVEN** a decision recorded for a project command
- **WHEN** the same content appears at a different project path, or different
  content appears at the same path
- **THEN** the earlier decision does not authorize it

#### Scenario: Project command is a symlink out of the project

- **GIVEN** a project command file that canonicalizes outside the project root
- **WHEN** discovery runs
- **THEN** the command is not registered
- **AND** the escape is reported as a discovery problem

#### Scenario: Existing trust files remain readable

- **GIVEN** a persisted trust file written before commands were trustable
- **WHEN** Smith reads it
- **THEN** existing decisions load unchanged
- **AND** project commands are undecided rather than approved
