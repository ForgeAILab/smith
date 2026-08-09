## MODIFIED Requirements

### Requirement: Child management controls

Smith SHALL expose root-only operations to spawn, list, send or follow up, wait,
fetch result, resume, and stop direct children. Operations MUST be addressed by
stable child ID and return structured lifecycle or error results.

The `agent.wait` operation SHALL accept an optional `timeout_ms`. Its resolved
configuration paths SHALL be
`profiles.<name>.child_agents.wait_default_timeout_ms` and
`profiles.<name>.child_agents.wait_max_timeout_ms`. The default range is
`0..=30_000` milliseconds, the maximum range is `1..=30_000`, and their
defaults are 5,000 and 30,000 milliseconds respectively. The resolved default
MUST NOT exceed the resolved maximum. A requested timeout of zero SHALL be an
immediate status check; a requested timeout above the resolved maximum SHALL
be rejected before waiting. A timeout before terminal child delivery SHALL
return a successful structured `running` result and MUST NOT leave the parent
tool call blocked indefinitely. The model-facing description SHALL state that
terminal outcomes are delivered automatically.

#### Scenario: Follow up with an idle child

- **GIVEN** a child completed one response but remains available
- **WHEN** the root sends a follow-up task
- **THEN** the child resumes under its existing limits and workspace
- **AND** the new activity remains attributed to the same child ID

#### Scenario: Stop a running child

- **GIVEN** a child is executing a cancellable tool
- **WHEN** the root stops that child
- **THEN** cancellation reaches the tool and provider stream
- **AND** the parent receives one terminal stopped result

#### Scenario: Child exceeds wait timeout

- **GIVEN** the parent calls `agent.wait` with a 5-second timeout
- **AND** the child remains active
- **WHEN** the timeout expires
- **THEN** the tool returns `status = "running"`
- **AND** the parent may complete or continue normally
- **AND** the child remains active and its terminal outcome remains
  must-deliver

#### Scenario: Requested timeout exceeds the host maximum

- **GIVEN** the model requests a timeout above the resolved maximum
- **WHEN** Smith prepares the wait operation
- **THEN** it rejects the value according to the documented shared contract
  before waiting
- **AND** it cannot extend the parent tool call beyond the host maximum

### Requirement: Safe parent reporting

Child progress and results SHALL appear immediately in the active host and
enter the parent session inbox. Model-facing terminal outcomes MUST use the
protected lossless must-deliver channel with deterministic ordering keys and
MUST be introduced only at a safe provider/tool boundary. Concurrent terminal
outcomes MUST NOT be dropped, duplicated, or reordered by the bounded
coalescable progress queue.

In the interactive TUI, immediate appearance is satisfied by the delegated-work
panel and retained child log. The root transcript SHALL carry only delegation
boundaries—spawn, exact-checkpoint resume start, needs-input, interruption,
completion, stop, and failure. Mid-flight progress MAY be coalesced for
model-facing delivery but every lifecycle event, printed or not, MUST append to
the child's bounded inspectable log. Injecting a terminal outcome MUST NOT
interrupt an active parent stream; idle continuation follows the separate
admission requirement.

#### Scenario: Child finishes during parent streaming

- **GIVEN** the parent model is streaming when a child completes
- **WHEN** the terminal result arrives
- **THEN** the TUI marks the child complete immediately
- **AND** the result is protected for the parent's next safe continuation
- **AND** the active parent stream is not interrupted

#### Scenario: Child runs a tool while the root conversation is open

- **GIVEN** a running child calls a tool
- **WHEN** the TUI folds the progress event
- **THEN** the child's panel row shows the new activity
- **AND** the event appends to that child's inspectable log
- **AND** no notice is written to the root transcript

#### Scenario: Child reaches a terminal state

- **GIVEN** a child completes, stops, is interrupted, or fails
- **WHEN** the TUI folds the terminal event
- **THEN** an attributed notice enters the root transcript
- **AND** the same event appends to that child's inspectable log
- **AND** the model-facing outcome remains in the protected channel until
  consumed

#### Scenario: Multiple children finish together

- **GIVEN** two children finish before the next parent boundary
- **WHEN** their outcomes enter the parent
- **THEN** both outcomes are retained
- **AND** their relative order is deterministic
- **AND** neither outcome can be lost to progress coalescing

## ADDED Requirements

### Requirement: Parent parking instead of indefinite model wait

A parent SHALL NOT require a provider stream or tool call to remain open solely
while waiting for a child. After spawning a child, the parent MAY complete its
current provider turn. When no parent turn is active and at least one direct
child remains nonterminal/pending, the session SHALL enter
`parked-awaiting-child`.

Parking SHALL preserve canonical parent history and exact child identity, keep
the child coordinator active, require no provider request, permit user input,
cancellation, shutdown, and child-result delivery, and add no synthetic
assistant message merely to keep the turn alive. A terminal child outcome that
is awaiting delivery remains protected, but does not by itself satisfy the
nonterminal-child parking predicate.

#### Scenario: Parent has no independent work

- **GIVEN** the parent spawns a child
- **AND** has no independent work remaining
- **WHEN** the current parent turn reaches a normal completion boundary
- **THEN** the parent enters `parked-awaiting-child`
- **AND** no provider call or tool call remains open merely to await the child

#### Scenario: Parent continues independent work

- **GIVEN** the parent spawns a child
- **AND** has independent inspection or validation work
- **WHEN** the child continues in parallel
- **THEN** the parent may continue its current turn
- **AND** enters parked state only after that turn completes with an outcome
  still pending

#### Scenario: All children settle before the parent finishes

- **GIVEN** every pending child reaches a terminal state while the parent is
  still serving
- **WHEN** the parent reaches its terminal boundary
- **THEN** the outcomes are available at that safe boundary
- **AND** the session does not enter a stale parked state with no pending source

#### Scenario: Restart reconciles live child work

- **GIVEN** a parked parent has a running or otherwise uncommitted child
- **WHEN** the process exits and the session restarts
- **THEN** Smith reconciles that child to
  `interrupted_by_process_exit`
- **AND** it never auto-restarts the child
- **AND** any terminal child outcome committed before exit remains terminal
  and is delivered at most once through the protected channel

### Requirement: Terminal child completion may wake an idle parent

After injecting one or more terminal child outcomes, Smith SHALL attempt to
admit one attributed internal continuation only when the parent remains idle.
The continuation SHALL use the parent runtime, provider, model, profile,
context policy, and exact cache identity applicable at admission time; consume
all ready terminal outcomes in deterministic order; be attributed as
`delegation.child-completion`; and run through ordinary limits, context
planning, provider/tool policy, approvals, checkpoints, cancellation, retries,
usage accounting, and cache observation.

The continuation MUST NOT queue behind real user work, start concurrently with
another parent turn, or fabricate a user-role message.

#### Scenario: Parent is parked and idle

- **GIVEN** the parent is `parked-awaiting-child`
- **WHEN** a terminal child outcome is injected
- **THEN** Smith attempts one conditional internal continuation
- **AND** the parent can integrate the result without a new user message

#### Scenario: Parent is serving user input

- **GIVEN** a child completes while the parent processes a real user turn
- **WHEN** the outcome is injected
- **THEN** Smith starts no competing internal turn
- **AND** the outcome remains protected for the next safe boundary

#### Scenario: Several outcomes are ready at admission

- **GIVEN** multiple terminal outcomes are ready while the parent is idle
- **WHEN** one child-completion continuation is admitted
- **THEN** it consumes every ready outcome in deterministic order
- **AND** Smith does not create one provider turn per child by default

### Requirement: Real user input has admission priority

Real user input SHALL take priority over child-driven internal continuation.
If user input and a terminal child outcome become ready concurrently, Smith
SHALL either include the protected outcome in the admitted user turn at the
next safe boundary or preserve it for the immediately following conditional
continuation. It MUST NOT lose, duplicate, or reorder the outcome.

#### Scenario: User returns as child completes

- **GIVEN** a parked parent
- **AND** user input and a child terminal outcome become ready concurrently
- **WHEN** turn admission is serialized
- **THEN** the user turn wins admission
- **AND** the child outcome remains must-deliver
- **AND** no second provider turn starts concurrently

#### Scenario: User turn consumes the outcome

- **GIVEN** user input wins the admission boundary
- **AND** the child outcome is available at that turn's safe context boundary
- **WHEN** Agent Runtime plans the user turn
- **THEN** it may include the outcome without creating a fabricated user message
- **AND** no redundant child-only continuation is admitted afterward

### Requirement: Child activity does not pin the parent cache

Smith SHALL keep a running child's activity separate from the parent cache
lease and inactivity clock. Child provider calls, tool calls, monitors, and
progress events MUST NOT refresh the parent cache lease or reset the parent inactivity limit.
The presence of an active child MAY justify bounded adaptive maintenance under
`max_hold_while_child_ms`, but cannot extend that bound indefinitely.

#### Scenario: Chatty child progress

- **GIVEN** a child emits frequent progress events
- **WHEN** Smith evaluates the parent inactivity and cache-touch clocks
- **THEN** progress resets neither clock
- **AND** produces no parent provider request by itself

#### Scenario: Child uses the same model

- **GIVEN** parent and child resolve the same provider and model
- **WHEN** the child performs provider requests under its own session identity
- **THEN** those attempts remain attributed to the child cache identity
- **AND** do not establish a touch or hit for the parent identity

### Requirement: Parked-state shutdown and headless behavior

A parked parent SHALL remain cancellable and SHALL shut down in bounded order.
On shutdown, Smith SHALL stop cache-maintenance scheduling, cancel in-flight
synthetic requests, freeze child-completion admission, apply existing child
stop/wait/durability policy, persist the latest compatible resume capsule when
enabled, release provider/cache resources according to adapter policy, and emit
one terminal shutdown sequence.

A headless host MAY remain process-alive while required child work is pending
under ordinary child policy, but MUST NOT remain alive solely to preserve a
provider cache after required work is complete.

#### Scenario: User exits while parent is parked

- **GIVEN** a parked parent with a running child
- **WHEN** the host begins shutdown
- **THEN** no new cache maintenance or child-completion turn starts
- **AND** the configured child shutdown policy applies
- **AND** shutdown remains bounded

#### Scenario: Required child work completes in headless mode

- **GIVEN** a headless host stayed alive to receive a required child outcome
- **WHEN** the outcome is integrated and no other required work remains
- **THEN** the host completes its ordinary terminal sequence
- **AND** it does not extend process life for cache retention alone

### Requirement: Delegation parking tests

Smith SHALL provide deterministic delegation tests covering at least:

1. a parent turn completing while a child remains running;
2. entry into parked state without provider I/O;
3. bounded `agent.wait` returning `running` on timeout;
4. terminal child result retention through the must-deliver channel;
5. one idle-parent child-completion continuation;
6. real user input winning concurrent admission;
7. deterministic ordering of multiple terminal outcomes;
8. child progress not refreshing parent cache or inactivity clocks;
9. the maximum child hold stopping maintenance while the child remains alive;
   and
10. bounded parked-state shutdown.

The fixtures MUST assert coordinator, admission, provider-call, cache-clock,
usage-purpose, and persistence effects rather than only TUI presentation.

#### Scenario: Delegation parking matrix runs

- **GIVEN** a scripted coordinator, admission boundary, clock, and provider
- **WHEN** each parking and race fixture runs
- **THEN** outcome delivery and provider turn counts match the listed contract
- **AND** no wait timeout, progress event, or admission race loses a terminal
  outcome
