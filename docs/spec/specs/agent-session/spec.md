# agent-session Specification

## Purpose
TBD - created by archiving change add-smith-agent-harness. Update Purpose after archive.
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

Smith MUST warn or return an active-work error before normal exit when
monitors, children, or background shell tasks remain. Once exit is confirmed,
it SHALL stop accepting new work, cancel children, terminate owned process
groups, append terminal events, flush canonical records, and exit within a
bounded grace period.

#### Scenario: User exits the TUI with active work

- **GIVEN** one monitor and one child are running
- **WHEN** the user requests TUI exit
- **THEN** Smith shows both active work items and requests confirmation
- **AND** leaves them running if the user cancels the exit

#### Scenario: User confirms exit

- **GIVEN** active work exists and the user confirms exit
- **WHEN** shutdown begins
- **THEN** Smith stops the owned work and records its terminal reason
- **AND** flushes the session before the grace period ends

#### Scenario: Background task counts as active work at exit

- **GIVEN** only a background shell task is running
- **WHEN** the user requests TUI exit
- **THEN** Smith names the running task and requests confirmation
- **AND** confirmed exit terminates its owned process group and records its
  terminal reason
