## MODIFIED Requirements

### Requirement: Explicit background shell execution

The `shell` tool SHALL accept an optional `run_in_background` boolean. When
true, Smith MUST validate and authorize the call exactly as a foreground call,
spawn the command in a Smith-owned process group, and return immediately with
a session-scoped task ID and a bounded output-spool reference instead of
waiting for completion. A background task SHALL run until it exits, is stopped
by `task_stop`, or the session shuts down; `timeout_ms` MAY still be supplied
to bound it. Smith MUST NOT convert a foreground call to background
automatically except as defined by Automatic foreground handoff.

#### Scenario: Start a build in the background

- **GIVEN** the agent calls `shell` with a build command and
  `run_in_background: true`
- **WHEN** approval policy permits the command
- **THEN** the tool call returns promptly with a task ID and spool reference
- **AND** the build process keeps running after the tool call resolves

#### Scenario: Approval still gates a background command

- **GIVEN** a background shell request whose command requires confirmation
- **WHEN** the user has not yet approved it
- **THEN** no process is spawned
- **AND** the pending approval shows the same material arguments as a
  foreground call

#### Scenario: A bounded background task is killed at its deadline

- **GIVEN** a background task started with an explicit `timeout_ms`
- **WHEN** the deadline elapses before the process exits
- **THEN** Smith terminates the owned process group
- **AND** the terminal notification states the task was killed at its deadline

## ADDED Requirements

### Requirement: Automatic foreground handoff

A foreground `shell` call SHALL NOT hold a turn longer than 5 minutes, or for
longer than 2 seconds after its shell exits. A call still running 300000 ms
after it started MUST move to a session background task. The task keeps the
output captured so far and the remaining `timeout_ms` deadline, which is
measured from the start of the call. When the shell exits but a process it
started still holds the output pipe 2 seconds later, that process MUST move to
a session background task with no deadline. If no background service can take
the call or the process, Smith MUST stop it with its process group. Cancellation MUST reach
the process group until the call returns. A background task whose shell has
exited MUST stay stoppable by `task_stop`, its deadline, and session shutdown
for as long as its output pipe stays open.

#### Scenario: A long call moves to the background at 5 minutes

- **GIVEN** a foreground call with `timeout_ms` 600000
- **WHEN** the command is still running 300000 ms after the call started
- **THEN** the tool call returns with a task ID and the output captured so far
- **AND** the result says the command has not completed
- **AND** the task is killed with its process group 600000 ms after the call
  started if it is still running

#### Scenario: Without a background service a long call is stopped

- **GIVEN** a direct embedder supplies no background task service
- **WHEN** a foreground call with `timeout_ms` 600000 is still running
  300000 ms after it started
- **THEN** Smith stops it with its process group
- **AND** the result says it could not be moved to the background

#### Scenario: A timeout at or below 5 minutes still kills in the foreground

- **GIVEN** a foreground call with the default 120000 ms timeout
- **WHEN** the command is still running at 120000 ms
- **THEN** Smith kills it with its process group and returns the actionable
  timeout outcome

#### Scenario: A server left running with `&` does not hold the call

- **GIVEN** a foreground command that starts a process with `&` without
  redirecting that process's output, then exits
- **WHEN** the process still holds the output pipe 2 seconds after the shell
  exits
- **THEN** the tool call returns the shell's output and exit status
- **AND** the process continues as a background task whose ID the result names

#### Scenario: Without a background service the process is stopped

- **GIVEN** a direct embedder supplies no background task service
- **WHEN** a process left by a foreground command still holds the output pipe
  2 seconds after the shell exits
- **THEN** Smith stops it with its process group
- **AND** the result says it was stopped

#### Scenario: Cancellation after the shell exits

- **GIVEN** a foreground command whose shell exited while a process it started
  holds the output pipe
- **WHEN** the tool call is cancelled before the call returns
- **THEN** Smith terminates the owned process group
- **AND** the call reports cancellation

#### Scenario: A background task whose shell exited first is stopped

- **GIVEN** a background task whose shell exited while a process it started
  holds the output pipe
- **WHEN** the agent calls `task_stop` with its task ID
- **THEN** Smith terminates the owned process group within the cleanup grace
  period
- **AND** the task reports the stopped state
