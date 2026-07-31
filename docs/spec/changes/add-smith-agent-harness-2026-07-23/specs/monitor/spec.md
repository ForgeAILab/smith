## ADDED Requirements

### Requirement: Exactly one monitor source

The `monitor` tool SHALL accept exactly one of `command` or `ws`. `command`
MUST be a shell command string; `ws` MUST contain a WebSocket URL and bounded
connection options. Supplying neither or both MUST fail validation before work
starts.

#### Scenario: Start a command monitor

- **GIVEN** a valid command, description, and lifecycle policy
- **WHEN** the agent calls `monitor`
- **THEN** Smith starts one owned command source
- **AND** returns a work ID and output-file reference

#### Scenario: Ambiguous monitor source

- **GIVEN** one request contains both `command` and `ws`
- **WHEN** Smith validates it
- **THEN** the request fails without opening a socket or starting a process

### Requirement: Command line event semantics

A command monitor SHALL run in the same shell environment and current-directory
model as Smith's shell tool. Every stdout line MUST become one raw event. Smith
MUST spool bounded stdout and stderr to the output file, but stderr MUST NOT
generate notifications unless the command explicitly merges it into stdout,
such as with `2>&1`.

#### Scenario: Command writes stdout and stderr

- **GIVEN** a monitored command writes `ready` to stdout and `warning` to
  stderr
- **WHEN** both lines are received
- **THEN** the output file contains both streams with stream metadata
- **AND** only `ready` becomes a chat notification

#### Scenario: Command merges stderr

- **GIVEN** a monitored command uses `2>&1`
- **WHEN** it writes an error to its merged stdout
- **THEN** that line becomes a raw monitor event

### Requirement: WebSocket text event semantics

Each WebSocket text frame SHALL become one raw monitor event. Binary frames
MUST NOT be inserted into chat and SHALL be ignored or stored only according to
an explicit bounded binary policy. Socket closure or terminal connection error
MUST end the monitor with a terminal notification.

#### Scenario: WebSocket sends text then closes

- **GIVEN** a monitored socket emits two text frames
- **WHEN** the second frame is followed by a normal close
- **THEN** Smith delivers both frames in order
- **AND** emits one terminal closed notification

### Requirement: Timeout and persistent lifecycle

Non-persistent monitors SHALL default to a five-minute timeout and MUST reject
timeouts above one hour. `persistent: true` SHALL run until `TaskStop`, session
shutdown, or source termination and MUST be mutually exclusive with
`timeout_ms`.

#### Scenario: Default timeout expires

- **GIVEN** a non-persistent monitor omits `timeout_ms`
- **WHEN** it remains active for five minutes
- **THEN** Smith terminates its owned process or closes its socket
- **AND** emits a timeout terminal notification

#### Scenario: Stop a persistent monitor

- **GIVEN** a persistent monitor is active
- **WHEN** `TaskStop` targets its work ID
- **THEN** Smith stops the source and emits one stopped terminal notification

### Requirement: Batching and flood protection

Smith MUST combine raw events received within 200 milliseconds into one chat
notification while preserving event order and boundaries. A configurable flood
guard SHALL auto-stop a source that exceeds its raw-event or byte threshold;
the initial defaults MUST be 1,000 events or 1 MiB within a rolling ten-second
window.

#### Scenario: Burst is batched

- **GIVEN** three stdout lines arrive within one 200 ms window
- **WHEN** Smith publishes monitor notifications
- **THEN** the host receives one notification containing the three ordered
  lines

#### Scenario: Monitor is too noisy

- **GIVEN** a source exceeds the configured flood threshold
- **WHEN** the guard trips
- **THEN** Smith stops the source
- **AND** emits a terminal noise-limit notification with the output-file path

### Requirement: Immediate host delivery and safe model steering

Monitor notifications SHALL reach the active host without a user prompt. Smith
MUST queue them into the session inbox and MUST NOT interrupt or mutate an
in-flight model stream or tool result. Coalesced notifications SHALL be supplied
to the agent at the next safe provider/tool boundary.

#### Scenario: Build failure arrives while agent works

- **GIVEN** the main agent is streaming a response
- **WHEN** a monitor sees `FAILED`
- **THEN** the TUI displays it immediately
- **AND** Smith pushes it to the model with the next safe continuation
- **AND** does not cancel the current stream

### Requirement: Monitor usage guidance

Smith's tool documentation SHALL explain line-buffer flushing, failure-aware
filters, and when to use a one-shot background shell loop instead of a monitor.

#### Scenario: Agent requests monitor documentation

- **GIVEN** the model inspects the `monitor` tool description
- **WHEN** it constructs a piped command
- **THEN** the description calls out line-buffered `grep`, explicit `awk`
  flushing, and failure signatures
- **AND** distinguishes one completion notification from repeated occurrences
