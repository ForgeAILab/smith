## ADDED Requirements

### Requirement: Explicit background shell execution

The `shell` tool SHALL accept an optional `run_in_background` boolean. When
true, Smith MUST validate and authorize the call exactly as a foreground call,
spawn the command in a Smith-owned process group, and return immediately with
a session-scoped task ID and a bounded output-spool reference instead of
waiting for completion. A background task SHALL run until it exits, is stopped
by `task_stop`, or the session shuts down; `timeout_ms` MAY still be supplied
to bound it. Smith MUST NOT convert a foreground call to background
automatically under any condition.

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

### Requirement: Background task lifecycle and notification

A background task's stdout and stderr SHALL spool to a bounded per-task
output file; exceeding the byte cap MUST truncate with an explicit marker, not
grow unbounded. When the task reaches a terminal state — exit, stop, deadline
kill, or session shutdown — Smith MUST emit exactly one terminal notification
through the session inbox carrying the task ID, terminal state, exit code when
available, and a bounded output tail. The journal SHALL record metadata-only
lifecycle markers for task start and terminal state, never output bodies.

#### Scenario: Task exits while the model is streaming

- **GIVEN** a background task exits during an in-flight provider response
- **WHEN** the terminal notification is published
- **THEN** the current response stream is not mutated
- **AND** the agent receives the notification at the next safe boundary

#### Scenario: Journal stays metadata-only

- **GIVEN** a background task produced output and exited
- **WHEN** the session journal is written
- **THEN** it contains the task's lifecycle markers and metadata
- **AND** it contains no spooled output body

### Requirement: Background task inspection and control

Smith SHALL provide a `task_output` tool returning a background task's status,
exit code when terminal, and a bounded incremental slice of its spooled output
addressed by offset, and a `task_stop` tool that terminates a running task's
owned process group by task ID. Both MUST fail with a stable error for an
unknown task ID. `task_stop` on an already-terminal task MUST be idempotent
and report the existing terminal state.

#### Scenario: Poll a running task's output

- **GIVEN** a running background task that has produced output
- **WHEN** the agent calls `task_output` with its task ID and a prior offset
- **THEN** it receives only the output after that offset
- **AND** the reported status is running with no exit code

#### Scenario: Stop a running task

- **GIVEN** a running background task
- **WHEN** the agent calls `task_stop` with its task ID
- **THEN** Smith terminates the owned process group within the cleanup grace
  period
- **AND** exactly one terminal notification reports the stopped state

### Requirement: Actionable foreground timeout outcome

A foreground shell command that reaches its timeout MUST still be killed with
its process group and returned as an error containing the output captured so
far. The outcome text SHALL state the elapsed limit and name the concrete
options: raising `timeout_ms` toward the documented maximum, narrowing the
command, or rerunning with `run_in_background: true`.

#### Scenario: Timed-out grep teaches the model its options

- **GIVEN** a foreground shell command killed at its 120000 ms default timeout
- **WHEN** the tool outcome is returned
- **THEN** it includes the partial output and names the timeout that elapsed
- **AND** it mentions `timeout_ms`, narrowing the command, and
  `run_in_background` as next steps
