# tool-execution Specification

## Purpose
TBD - created by archiving change add-smith-agent-harness. Update Purpose after archive.
## Requirements
### Requirement: Minimal built-in coding tools

Smith SHALL provide built-in file read, path/text search, patch application,
and shell command tools implementing Agent Runtime's neutral `Tool` contract.
Every tool MUST publish a stable name, description, effects, input schema, and
bounded structured outcome.

#### Scenario: Apply a project patch

- **GIVEN** the agent submits a valid patch inside the allowed project root
- **WHEN** approval policy permits the write
- **THEN** Smith applies the patch atomically where practical
- **AND** records a structured result and affected paths

### Requirement: Scoped execution context

Every tool call MUST use the shared invocation context carrying workspace,
deadline, cancellation token, output limit, approval decision, and request
identity. A Smith tool MUST NOT infer broader filesystem authority from the
process working directory.

#### Scenario: Tool targets outside its scope

- **GIVEN** a filesystem tool is restricted to one project root
- **WHEN** its resolved target escapes that root
- **THEN** Smith rejects the call before mutation
- **AND** records a permission failure

### Requirement: Approval before material side effects

Smith SHALL inject a configurable approval policy into Agent Runtime. The
shared executor MUST evaluate it before shell execution or filesystem mutation;
an action requiring confirmation remains pending until approved and fails
closed when no allowing policy is available.

#### Scenario: Interactive mutation requires confirmation

- **GIVEN** a patch is classified as requiring approval
- **WHEN** the agent requests it in the TUI
- **THEN** Smith displays the exact tool, scope, and material arguments
- **AND** executes only after user approval

#### Scenario: Headless approval cannot be collected

- **GIVEN** `smith -p` has no TTY and no policy authorizing a shell command
- **WHEN** the agent requests that command
- **THEN** Smith returns an approval-required result and stable non-success
  outcome
- **AND** never hangs waiting for input

### Requirement: Bounded cancellable processes

Shell and monitor commands MUST run in Smith-owned process groups on macOS and
Linux. Smith SHALL enforce deadlines and output limits and MUST terminate the
owned group on cancellation or confirmed shutdown.

#### Scenario: Shell command spawns a child process

- **GIVEN** a shell command starts a subprocess
- **WHEN** the tool call is cancelled
- **THEN** Smith terminates the owned process group within the cleanup grace
  period
- **AND** records whether forced termination was required

### Requirement: Side-effect-aware tool scheduling

Smith SHALL configure Agent Runtime's side-effect-aware scheduler. Independent
read-only tools MAY run concurrently, but shared execution MUST serialize or
reject calls whose declared write scopes overlap unless an explicit conflict
policy allows another deterministic outcome.

#### Scenario: Two patches overlap

- **GIVEN** one model turn requests two patches to the same file
- **WHEN** Smith schedules the calls
- **THEN** it does not apply them concurrently
- **AND** preserves deterministic result order

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
