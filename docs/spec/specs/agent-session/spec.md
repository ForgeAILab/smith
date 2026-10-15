# agent-session Specification

## Purpose
The canonical direct agent loop and append-only sessions Smith runs on Agent Runtime, including the safe-boundary inbox and reconciliation of ephemeral work.
## Requirements
### Requirement: Shared canonical direct agent loop

Smith SHALL execute direct API turns through Agent Runtime's canonical
streaming model/tool loop. Smith MUST supply product instructions, selected
provider/model policy, tools, approval, workspace, stores, and observers without
reimplementing request planning, stream handling, tool validation/execution, or
turn continuation.

#### Scenario: Complete a tool-assisted turn

- **GIVEN** the model requests a registered tool with valid arguments
- **WHEN** the action is allowed and the tool succeeds
- **THEN** Agent Runtime appends the canonical tool call and result
- **AND** continues the same shared turn with the selected provider
- **AND** streams the final assistant response to the host

#### Scenario: Reject malformed tool arguments

- **GIVEN** the model completes a tool call whose arguments fail its schema
- **WHEN** the shared loop validates the call
- **THEN** the runtime does not execute the tool
- **AND** Smith receives a structured validation failure through shared events

### Requirement: Append-only canonical sessions

Smith MUST persist Agent Runtime's versioned canonical events through an
injected `EventObserver` in append-only JSON Lines logs under user state and
persist compatible `SessionSnapshot`s through `SessionStore`. Smith-owned
orchestration events MAY share the journal only when they are explicitly
versioned and cannot be mistaken for shared runtime events.

#### Scenario: Resume a prior session

- **GIVEN** a session ended after complete records were flushed
- **WHEN** the user resumes its session ID
- **THEN** Smith loads a compatible shared snapshot and reconstructs the
  transcript, selection, usage, manifests, and compaction state
- **AND** does not replay synthetic cache ping/pong as conversation messages

#### Scenario: Recover an incomplete tail

- **GIVEN** a process crash left one partial final JSONL record
- **WHEN** Smith opens the session
- **THEN** it preserves all prior complete records
- **AND** quarantines or truncates only the incomplete final record
- **AND** emits a recovery diagnostic

#### Scenario: Snapshot lags the event journal

- **GIVEN** a crash occurred after complete runtime events were journaled but
  before the latest snapshot was saved
- **WHEN** Smith resumes the session
- **THEN** it reconstructs a compatible snapshot from the complete journal or
  reports that the tail is not recoverable
- **AND** it does not silently claim the stale snapshot is complete

### Requirement: Ephemeral work reconciliation

Smith SHALL persist the fact that a monitor, child, or background shell task
was active, but MUST NOT claim that the work survives process exit. On resume
after a crash, previously active work MUST be marked
`interrupted_by_process_exit` and MUST NOT restart automatically.

#### Scenario: Resume after a process crash

- **GIVEN** a saved session contains a child-started event with no terminal
  event
- **WHEN** a new Smith process resumes the session
- **THEN** it appends an interrupted terminal state
- **AND** does not recreate the child

#### Scenario: Resume never fabricates a running background task

- **GIVEN** a saved session contains a background-task start marker with no
  terminal marker
- **WHEN** a new Smith process resumes the session
- **THEN** the task is reported as `interrupted_by_process_exit`
- **AND** no process is spawned for it

### Requirement: Safe-boundary session inbox

Smith SHALL expose a bounded session inbox for monitor notifications, child
progress, and host steering. The host MAY display an event immediately, but the
agent MUST consume queued items only before a provider request or after the
current tool boundary, never by mutating an in-flight provider stream.

#### Scenario: Notification arrives during model streaming

- **GIVEN** an assistant response is currently streaming
- **WHEN** a monitor emits an error line
- **THEN** the TUI receives the notification immediately
- **AND** the current response remains unchanged
- **AND** the agent receives the queued notification at the next safe boundary

#### Scenario: Inbox reaches capacity

- **GIVEN** non-terminal progress events exceed the configured inbox capacity
- **WHEN** Smith coalesces or drops superseded progress
- **THEN** it emits an overflow/coalescing marker
- **AND** retains terminal monitor and child-result events

### Requirement: Graceful process-owned shutdown

Smith MUST return an active-work error before a non-interactive exit when
monitors, children, or background shell tasks remain and the caller chose no
exit policy that covers them. In the TUI, `/quit` SHALL exit without asking,
whatever work is active. Once exit begins, Smith SHALL stop accepting new
work, cancel the running turn and children, terminate owned process groups,
append terminal events, flush canonical records, and exit. Exit begins when
the user quits the TUI, when a non-interactive run starts its shutdown, or
when Smith receives its first SIGINT, SIGTERM, or SIGHUP. Smith MUST exit
within 10 seconds of that moment; work that has not stopped by then SHALL be
killed, and the exit SHALL report that the deadline was reached.

#### Scenario: User quits the TUI with active work

- **GIVEN** a turn, one child, and one background shell task are running
- **WHEN** the user types `/quit`
- **THEN** Smith exits without a confirmation
- **AND** stops the turn, the child, and the task and records each terminal
  reason
- **AND** the exit output names the work it stopped

#### Scenario: Background task is stopped at exit

- **GIVEN** only a background shell task is running
- **WHEN** the user types `/quit`
- **THEN** Smith terminates the task's owned process group and records its
  terminal reason
- **AND** the exit output names the task

#### Scenario: Shutdown cannot finish in time

- **GIVEN** some part of shutdown has not finished 10 seconds after exit
  began
- **WHEN** the deadline passes
- **THEN** Smith kills its remaining owned processes and exits
- **AND** the TUI restores the terminal and a non-interactive run still writes
  its result, which reports the deadline

#### Scenario: Ordinary exit finishes well inside the deadline

- **GIVEN** a turn whose provider honors cancellation
- **WHEN** the user types `/quit` during it
- **THEN** Smith saves the session and exits without waiting for the
  deadline
