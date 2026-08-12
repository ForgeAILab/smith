## MODIFIED Requirements

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
