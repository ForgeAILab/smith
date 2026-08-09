## MODIFIED Requirements

### Requirement: User input wins automatic continuation admission

Smith SHALL dispatch pending real-user input before allowing an idle-only goal
or child-completion continuation attempt at the same serialized boundary. An
accepted steer to an automatically owned serving turn MUST remain real user
input and MUST NOT change goal, delegation, authority, cache, or accounting
identity implicitly.

Goal and child-completion controllers SHALL use Agent Runtime's same
conditional idle-only admission mechanism. They MUST defer locally before
calling Runtime or handle `ChildCompletionAdmission::Busy` when user input has
won, and MUST NOT queue a competing provider turn behind it. Protected child
outcomes remain must-deliver until included at a safe boundary or consumed by
a later admitted continuation.

#### Scenario: Goal and queued user input reach an idle boundary

- **GIVEN** an active goal is eligible for automatic continuation
- **AND** Smith has one queued real-user turn
- **WHEN** the serving turn reaches its terminal boundary
- **THEN** Smith submits the real-user turn first
- **AND** the goal controller observes busy or waits for a later boundary

#### Scenario: Child completion and user input race

- **GIVEN** a parked parent receives a terminal child outcome
- **AND** real user input becomes ready at the same boundary
- **WHEN** Agent Runtime serializes admission
- **THEN** the real-user turn wins
- **AND** the child outcome is included safely or remains protected
- **AND** no child-completion provider turn starts concurrently

#### Scenario: Goal and child completion are both eligible

- **GIVEN** an idle parent has an active goal and ready terminal child outcomes
- **WHEN** automatic continuation admission is arbitrated
- **THEN** Agent Runtime admits at most one attributed continuation
- **AND** the other source remains eligible for a later boundary without loss
  or duplicate provider work

## ADDED Requirements

### Requirement: Child completion uses conditional internal turns

Smith SHALL call Agent Runtime's `try_admit_child_completion_if_idle` with a
`ChildCompletionAdmissionRequest` and expected `ChildOutcomeCursor`. Runtime
may accept the bounded provenance-bearing child-completion turn only if the
parent remains idle at one serialized decision boundary and returns
`ChildCompletionAdmission::{Accepted, Busy, Stale, Shutdown, Conflict}`. An
accepted turn SHALL be attributed as `delegation.child-completion`, consume
every ready protected terminal outcome in deterministic order, and use the
parent runtime composition applicable at admission.

The internal input MUST NOT append a fabricated user-role canonical message,
queue ahead of real user work, bypass ordinary provider/context/tool/approval/
workspace/checkpoint/cancellation/retry/global-limit contracts, or survive as
unattributed background work. Admission identity and outcome consumption SHALL
be crash- and replay-safe so one committed continuation is not repeated.

Agent Runtime SHALL own canonical admission and cache operation/evidence event
payloads used by this turn. Smith may consume and project those redaction-safe
events into product surfaces, but MUST NOT define local RuntimeEvent variants
or a second normalized provider event vocabulary.

#### Scenario: Idle parked session accepts child completion

- **GIVEN** a root session is idle in `parked-awaiting-child`
- **AND** one or more protected terminal outcomes are ready
- **WHEN** its controller conditionally requests continuation
- **THEN** Agent Runtime accepts one attributed internal turn
- **AND** normal context planning, provider execution, usage, and cache
  observation apply

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

### Requirement: Parked state performs no provider work by itself

Smith SHALL perform no provider work solely because a session is parked.
Entering, remaining in, listing, inspecting, persisting, resuming, or leaving
`parked-awaiting-child` SHALL perform no provider request by itself. Provider
work may begin only for admitted real-user, goal, or child-completion turns or
for separately authorized cache maintenance under the prompt-cache policy.
Parking MUST NOT be represented by a synthetic canonical assistant message.

#### Scenario: Parent parks while child continues

- **GIVEN** a parent turn completes and a child remains running
- **WHEN** Smith records the parked lifecycle state
- **THEN** provider invocation count is unchanged
- **AND** the child coordinator remains able to deliver terminal evidence

#### Scenario: Resumed session renders parked evidence

- **GIVEN** a persistent session records that its prior process parked while a
  child was active
- **WHEN** a later process reconciles and renders the session
- **THEN** it performs no provider request merely to show that evidence
- **AND** existing child durability policy determines interrupted or terminal
  state

#### Scenario: Cache maintenance is separately authorized

- **GIVEN** a parked parent is eligible for one adaptive maintenance request
- **WHEN** Smith dispatches it under the cache policy
- **THEN** the attempt carries a synthetic cache purpose rather than a parked
  continuation purpose
- **AND** parking alone is not reported as the provider-work cause
