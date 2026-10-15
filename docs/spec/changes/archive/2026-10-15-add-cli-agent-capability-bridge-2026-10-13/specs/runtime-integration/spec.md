## MODIFIED Requirements

### Requirement: Child completion uses conditional internal turns

Smith SHALL call Agent Runtime's `try_admit_child_completion_if_idle` with a
`ChildCompletionAdmissionRequest` and expected `ChildOutcomeCursor`. Runtime
may accept the bounded provenance-bearing child-completion turn only if the
parent remains idle at one serialized decision boundary and returns
`ChildCompletionAdmission::{Accepted, Busy, Stale, Shutdown, Conflict}`. An
accepted turn SHALL be attributed as `delegation.child-completion`, consume
every ready protected terminal outcome in deterministic order, and use the
parent runtime composition applicable at admission. When that composition
runs turns on an external agent backend, the accepted turn SHALL run on that
backend as "Internal turns on a harness session run on the harness"
requires.

The internal input MUST NOT append a fabricated user-role canonical message,
queue ahead of real user work, bypass the provider, context, tool, approval,
workspace, checkpoint, cancellation, retry, and global-limit contracts an
ordinary turn on that session observes, or survive as unattributed
background work. Admission identity and outcome consumption SHALL be crash-
and replay-safe so one committed continuation is not repeated.

Agent Runtime SHALL own canonical admission and cache operation/evidence event
payloads used by this turn. Smith may consume and project those redaction-safe
events into product surfaces, but MUST NOT define local RuntimeEvent variants
or a second normalized provider event vocabulary.

#### Scenario: Idle parked session accepts child completion

- **GIVEN** a root session on a model provider is idle in
  `parked-awaiting-child`
- **AND** one or more protected terminal outcomes are ready
- **WHEN** its controller conditionally requests continuation
- **THEN** Agent Runtime accepts one attributed internal turn
- **AND** normal context planning, provider execution, usage, and cache
  observation apply

#### Scenario: Idle parked harness session accepts child completion

- **GIVEN** a root session on a harness is idle in `parked-awaiting-child`
- **AND** one or more protected terminal outcomes are ready
- **WHEN** its controller conditionally requests continuation
- **THEN** Agent Runtime accepts one attributed internal turn
- **AND** the turn runs on the harness with no provider request

#### Scenario: Parent became busy before admission

- **GIVEN** terminal outcomes were injected while the parent appeared idle
- **WHEN** another turn wins before conditional admission commits
- **THEN** the child-completion attempt returns
  `ChildCompletionAdmission::Busy`
- **AND** the outcomes remain protected for the next safe boundary

#### Scenario: Continuation history is inspected

- **GIVEN** one or more child-completion internal turns committed
- **WHEN** canonical history, lifecycle events, or checkpoints are inspected
- **THEN** no fabricated user continuation message is present
- **AND** attributed evidence identifies each internal turn and consumed
  outcome set

#### Scenario: Replay sees a committed child continuation

- **GIVEN** a child-completion turn and its outcome-consumption watermark
  committed before a crash
- **WHEN** Smith resumes and replays the journal
- **THEN** presentation can reconstruct the attributed lifecycle
- **AND** replay cannot schedule the committed provider turn again

## ADDED Requirements

### Requirement: External turns dispatch host tools only through Agent Runtime

Agent Runtime SHALL give an external agent backend a turn-scoped handle that
lists the tool specifications the host offers for the turn and executes one
call through the runtime's own tool path. Smith MUST use that handle for
every bridged call and MUST NOT invoke a tool implementation directly.

A bridged call SHALL resolve against the session's authorized tool view and
SHALL NOT require the tool to be in an activation epoch or count against the
activation budget. The call and its result SHALL commit to canonical history
in the order they occurred relative to the text the backend reported, and
the handle MUST refuse a call once its turn has reached a terminal event.

#### Scenario: A bridged call during an external turn

- **GIVEN** an external turn in flight
- **WHEN** the backend submits a call through the handle
- **THEN** the runtime emits the ordinary tool request and result events
  attributed to that turn
- **AND** no provider request is made

#### Scenario: A tool that was never activated

- **GIVEN** an authorized tool outside the session's activation epoch
- **WHEN** the backend calls it through the handle
- **THEN** the call is dispatched

#### Scenario: History after a bridged call

- **GIVEN** the backend reported text, then made a bridged call, then
  reported more text
- **WHEN** the turn completes
- **THEN** canonical history holds the first text with the call, the tool
  result, and the remaining text, in that order

#### Scenario: A call after the turn ended

- **GIVEN** an external turn that has published its terminal event
- **WHEN** a call is submitted through its handle
- **THEN** the handle refuses it and nothing is dispatched

#### Scenario: Process ends during a bridged call

- **GIVEN** a bridged call in flight when the process exits
- **WHEN** the session is resumed
- **THEN** the turn is finalized as interrupted
- **AND** the call is not executed again

### Requirement: Internal turns on a harness session run on the harness

Agent Runtime SHALL execute an admitted internal turn on the external agent
backend when the session's turns run on one, continuing the backend's own
conversation, and MUST NOT call the session's provider.
Admission, attribution, outcome consumption, and replay safety SHALL be the
ones the internal turn already has.

The internal input SHALL be sent to the backend as bounded text marked as a
delivery from Smith. It MUST NOT be appended to Smith's canonical history as
a user message.

#### Scenario: A child completes while a harness parent is idle

- **GIVEN** a root session on a harness, parked awaiting a child
- **WHEN** the child completes and the completion turn is admitted
- **THEN** the turn runs on the CLI, resuming its session
- **AND** the provider is not called
- **AND** the turn is attributed as `delegation.child-completion`

#### Scenario: Continuation history on a harness session

- **GIVEN** a committed child-completion turn on a harness session
- **WHEN** canonical history is inspected
- **THEN** it contains no fabricated user message for the delivery

#### Scenario: Replay after a committed harness continuation

- **GIVEN** a child-completion turn on a harness committed before a crash
- **WHEN** Smith resumes and replays the journal
- **THEN** the CLI is not invoked again for that outcome
