# child-agents Specification

## Purpose
TBD - created by archiving change add-smith-agent-harness. Update Purpose after archive.
## Requirements
### Requirement: One-level agent hierarchy

Only the root/main agent SHALL be authorized to create child agents. Direct
children MUST not receive child-management tools, and runtime authorization
MUST reject spawn attempts from depth one even if a malformed or injected call
reaches the host.

#### Scenario: Root spawns a child

- **GIVEN** capacity and permission are available
- **WHEN** the root agent submits a valid child specification
- **THEN** Smith starts one direct child attributed to the parent session

#### Scenario: Child attempts to spawn

- **GIVEN** a direct child emits a call shaped like a spawn request
- **WHEN** Smith authorizes the call
- **THEN** it rejects the request as a depth violation
- **AND** no grandchild is created

### Requirement: Parent-selected child specification

The root agent MUST choose each child's task, expected result, provider/model,
turn/token/deadline limits, permission policy, and workspace policy. Workspace
policy SHALL be one of shared project, explicit directory, isolated worktree,
or read-only view and MUST be visible in child lifecycle events. The root MAY
additionally select one registered child-enabled agent profile per spawn, which
SHALL resolve through the same preflighted route the user-invoked presets use.
An unselected profile SHALL inherit the parent's, and an unresolvable profile
MUST fail the spawn without creating a child.

#### Scenario: Parent delegates a read-only review

- **GIVEN** the root decides that no write is needed
- **WHEN** it spawns a reviewer with read-only workspace policy
- **THEN** the child receives read tools but cannot mutate files or run
  write-authorized commands

#### Scenario: Parent chooses an isolated worktree

- **GIVEN** the root delegates write-capable implementation
- **WHEN** it chooses isolated-worktree policy
- **THEN** Smith creates or validates the isolated workspace before the child
  starts
- **AND** reports the workspace identity to the parent

#### Scenario: The model selects a registered profile

- **GIVEN** `plan`, `explore`, and `build` are registered child-enabled agent
  profiles with preflighted routes
- **WHEN** the model spawns a child naming the `explore` profile
- **THEN** the child runs on that profile's preflighted provider, model,
  prompt, and posture
- **AND** it resolves through the same route key a user-invoked preset resolves

#### Scenario: No profile is named

- **GIVEN** a spawn names no profile
- **WHEN** Smith builds the child
- **THEN** it inherits the parent's profile exactly as it did before profile
  selection existed
- **AND** no previously valid call changes meaning

#### Scenario: An unavailable profile is refused

- **GIVEN** a spawn names a profile that is unregistered, not child-enabled, or
  has no preflighted route
- **WHEN** Smith resolves the spawn
- **THEN** the tool call fails with an error naming the available profiles
- **AND** no child session, lifecycle event, or partial state is created

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

### Requirement: Bounded ephemeral children

Smith SHALL enforce configurable process, session, and parent running-child
concurrency limits plus cumulative per-child turn, token, deadline, and
retention limits. Live child executions MUST stop with the parent execution or
Smith process and MUST NOT restart automatically on parent resume. For durable
sessions, Smith SHALL retain the bounded child-session record and committed
state for explicit compatible follow-up or resume. For intentionally
non-persistent sessions, children remain explicitly process-ephemeral.

#### Scenario: Concurrency limit is reached

- **GIVEN** the parent has reached its configured running-child limit
- **WHEN** it requests another child
- **THEN** Smith returns a capacity result or queues it according to explicit
  policy
- **AND** does not exceed the running limit

#### Scenario: Smith exits with running and idle children

- **GIVEN** one durable child is running and another is idle
- **WHEN** the Smith process exits and the parent is later resumed
- **THEN** neither child runs during startup
- **AND** the first is shown interrupted while the second remains available for
  follow-up under the same IDs

#### Scenario: Retention limit expires a child

- **GIVEN** a durable child exceeds the configured retained-child age or count
  policy
- **WHEN** Smith performs owner-state cleanup
- **THEN** the child becomes terminal and non-resumable with bounded evidence
- **AND** no project file is removed as child-control metadata

### Requirement: Shared-workspace conflict visibility

Write-capable shared workspaces MUST produce a visible conflict warning. Smith
MUST serialize overlapping declared writes where possible and SHALL preserve a
structured conflict result when safe coordination is impossible.

#### Scenario: Parent and child target the same file

- **GIVEN** the parent and a shared-workspace child both request writes to one
  path
- **WHEN** their declared scopes overlap
- **THEN** Smith does not execute those writes concurrently
- **AND** reports the scheduling or conflict outcome to both runs

### Requirement: Bounded root agent modes

Smith SHALL provide a small host-owned registry of root agent modes whose
prompt policy and capability posture can only narrow the resolved run. Mode
selection MUST NOT change provider, model, credentials, trust, approval, or
authoritative permission policy implicitly.

#### Scenario: Select plan mode
- **GIVEN** the current run permits editing
- **WHEN** the user selects the built-in `plan` mode
- **THEN** Smith rebuilds the safe-boundary view without mutation abilities
- **AND** a provider-generated edit or shell-write call fails closed

#### Scenario: Repository defines a privileged mode
- **GIVEN** project configuration claims a mode grants write or shell authority
- **WHEN** Smith resolves modes
- **THEN** it rejects or narrows that claim under project-trust policy
- **AND** repository text cannot authorize a side effect

### Requirement: Explicit user-invoked child presets

Registered `@agent` references SHALL map to bounded host-controlled depth-one
child presets. Before dispatch Smith MUST show inherited provider/model,
workspace posture, limits, expected result, and provider-spend confirmation;
the preset cannot widen parent authority.

#### Scenario: Invoke a read-only reviewer
- **GIVEN** the user submits `@review` with a bounded task
- **WHEN** they confirm provider spend
- **THEN** Smith creates one attributed read-only child on the parent model
- **AND** the child has no write, shell-mutation, or child-management ability

#### Scenario: Child preset attempts nesting
- **GIVEN** a direct child returns a call shaped like delegation
- **WHEN** Smith authorizes it
- **THEN** the existing depth-one rule rejects the call
- **AND** the new UX creates no grandchild path

### Requirement: Child timeline navigation

Smith SHALL expose current and completed direct children through `/agent`,
`/timeline`, and keyboard selection of the delegated-work panel, with stable
previous, next, and parent navigation. Child inspection MUST be a temporary
read-only view and MUST NOT move persistent input focus away from the root
composer. Turn, token, session, and workspace figures shown for a child MUST
come from the delegation coordinator, never from client-side estimation.

#### Scenario: Inspect consecutive child results
- **GIVEN** two attributed children completed
- **WHEN** the user opens one and requests next
- **THEN** Smith displays the next child's bounded lifecycle and result
- **AND** returning to parent restores the same composer draft and scroll state

#### Scenario: Walk the delegated-work panel from the keyboard
- **GIVEN** at least one direct child exists and composer history has nowhere
  left to go
- **WHEN** the user presses the panel's next-agent key
- **THEN** Smith selects the next child in the panel's own order
- **AND** replaces the transcript region with that child's read-only view
- **AND** leaves the composer focused and its draft unchanged

#### Scenario: Continue the inspected child from the composer
- **GIVEN** a child that accepts a follow-up is being inspected
- **WHEN** the user submits an ordinary prompt
- **THEN** Smith confirms a follow-up turn addressed to that child
- **AND** a local command submitted in the same view still addresses the root

#### Scenario: No child is selected
- **GIVEN** no direct child exists
- **WHEN** the user opens child navigation
- **THEN** Smith reports the empty state locally with available preset hints
- **AND** does not fabricate a session or provider request

### Requirement: Durable addressable child sessions

Smith SHALL, when persistence and protected checkpoint storage are enabled,
retain one stable child ID and child session ID for every spawned child bound to the
original parent. Smith MUST restore its canonical history, manifests, usage,
cumulative limits, model/provider, tool scope, workspace posture, latest
outcome, and artifact lineage after restart. Following up that child MUST use
the same session and MUST NOT dispatch a replacement agent.

#### Scenario: Reuse a reviewer after restart

- **GIVEN** a read-only review child completed one task in a durable session
- **WHEN** Smith restarts, resumes the parent, and the root sends a follow-up to
  that child ID
- **THEN** the same child session receives the new turn with its prior history
- **AND** its model, workspace, authority, limits, and attribution remain
  compatible with the original specification
- **AND** Smith emits no new child-spawn event

#### Scenario: Persistence is disabled

- **GIVEN** the user intentionally starts a non-persistent Smith session
- **WHEN** a child is spawned and listed
- **THEN** Smith labels it process-ephemeral
- **AND** the UI and machine output do not promise post-restart follow-up

### Requirement: Existing-child follow-up and interrupted-task resume are distinct

Smith SHALL use `follow_up` to start a new turn on an idle durable child and
SHALL use explicit `resume` only to continue that child's interrupted exact
checkpoint. Startup, list, result, timeline, and inspection MUST NOT execute
provider or tool work. Unknown, legacy, terminal, or incompatible child IDs
MUST return structured results and MUST NOT fall back to spawn.

#### Scenario: Resume an interrupted child

- **GIVEN** a child process exited after a committed checkpoint and before its
  task completed
- **WHEN** the user explicitly confirms resume for that child
- **THEN** Smith continues the same child turn after the committed boundary
- **AND** does not consume another task slot or repeat committed work

#### Scenario: Follow up an unknown child

- **GIVEN** the root requests a follow-up for a child ID absent from its
  parent-scoped durable catalog
- **WHEN** Smith resolves the operation
- **THEN** it returns an unknown-child result
- **AND** creates no new child or provider request

### Requirement: Profile-based direct child composition

Smith SHALL create an explicit direct child from a child-enabled profile using
the same typed profile resolution and standard runtime composition boundaries
used for the main agent. Before dispatch it MUST show the selected profile,
bounded instruction summary, provider/model, effective limits, read-only
workspace posture, and provider-spend confirmation.

#### Scenario: Invoke a child on the parent's model
- **GIVEN** a child-enabled review profile resolves to the parent's
  provider/model
- **WHEN** the user submits and confirms `@review <task>`
- **THEN** Smith creates one attributed child with that profile's instructions
  and effective limits
- **AND** the child remains depth-one and read-only

#### Scenario: Invoke a child on another declared model
- **GIVEN** a child-enabled profile selects another fully configured declared
  provider/model
- **WHEN** the user confirms the displayed model and spend
- **THEN** Smith runs normal credential, catalog, context, and runtime preflight
  before allocating or dispatching the child
- **AND** no partial child or hidden fallback to the parent model is created

#### Scenario: Profile requests broader child authority
- **GIVEN** a child-enabled profile has build posture or a setting that would
  widen the host child ceiling
- **WHEN** Smith computes the effective child policy
- **THEN** the result remains the intersection of parent authority, the
  depth-one read-only child ceiling, and profile posture
- **AND** any incompatible value is rejected or source-explainably narrowed

### Requirement: Durable child profile identity

Smith SHALL include the effective profile name, revision, placement, provider/
model selection, and authority posture in durable child policy compatibility
evidence. Follow-up and resume MUST retain the same profile identity and MUST
fail closed when exact compatible composition is unavailable.

#### Scenario: Resume with an unchanged profile
- **GIVEN** an interrupted child has a durable checkpoint and its effective
  profile revision remains available
- **WHEN** the user explicitly resumes that child
- **THEN** Smith continues the unfinished turn with the same profile identity
- **AND** does not consume a new task slot or repeat committed work

#### Scenario: Profile changed before resume
- **GIVEN** the selected profile's effective instructions, model, or posture
  changed after checkpoint creation
- **WHEN** the user requests resume
- **THEN** Smith reports an incompatible policy fingerprint
- **AND** does not silently spawn a replacement or run under mixed revisions

### Requirement: Child write access requires posture, scope, and a writable workspace together

A child SHALL be able to modify a workspace only when its resolved agent
profile's posture is not read-only AND its spawn declared a full tool scope AND
its spawn's workspace policy is not the read-only view. Any one of those
conditions failing MUST leave the child read-only. A writing child MUST use the
parent's own approval policy, MUST NOT hold any permission the root does not
hold, and MUST NOT write outside its declared workspace.

The workspace condition is not a convenience. A read-only view resolves to the
same workspace handle a shared project resolves to, so nothing but the child's
tool set withholds a write from it — and the read-only view is also what a
spawn that names no workspace gets. Without it, a build-posture spawn asking
for a full tool scope and naming no workspace would silently receive
write-capable tools against the shared project.

#### Scenario: A build profile with a full tool scope and a writable workspace

- **GIVEN** a spawn selects a build-posture profile, declares `tools` of `all`,
  and names a shared or explicit-directory workspace
- **WHEN** the child's view is built
- **THEN** it contains the tools that can modify the workspace
- **AND** each modification goes through the parent's approval surface

#### Scenario: A build profile with a full tool scope over a read-only view

- **GIVEN** a spawn selects a build-posture profile and declares `tools` of
  `all`, but names the read-only-view workspace or names no workspace at all
- **WHEN** the child's view is built
- **THEN** it contains no tool that can modify the workspace
- **AND** the read-only view is what an unnamed workspace resolves to, so
  saying nothing about the workspace withholds the write rather than granting
  it

#### Scenario: A build profile without a full tool scope

- **GIVEN** a spawn selects a build-posture profile but declares the default
  read-only tool scope
- **WHEN** the child's view is built
- **THEN** it contains no tool that can modify the workspace

#### Scenario: A read-only profile with a full tool scope

- **GIVEN** a spawn declares `tools` of `all` but selects a read-only-posture
  profile, or inherits a read-only-posture parent
- **WHEN** the child's view is built
- **THEN** it contains no tool that can modify the workspace
- **AND** the declared scope cannot widen what the posture withheld

#### Scenario: A writing child stays inside its workspace

- **GIVEN** a child may write and its spawn declared an explicit directory
  workspace
- **WHEN** it attempts a modification outside that directory
- **THEN** the workspace refuses it exactly as it refuses the root
- **AND** the child gains no path the declared posture excluded

### Requirement: Delegated-work panel reports substantive child activity

The delegated-work panel SHALL identify each visible child by its id and
profile and report its current activity using the same reviewed tool display
projection the transcript uses, together with the turn and token counts the
delegation coordinator owns. It MUST clip to one line per child with the
elapsed clock preserved, and MUST NOT compute child turn or token figures
itself.

#### Scenario: A working child names what it is doing

- **GIVEN** a child is running a `read` call for a known path
- **WHEN** the panel renders its row
- **THEN** the row shows the reviewed projection for that call rather than the
  bare tool name
- **AND** it shows the child's profile, turns, and tokens beside it

#### Scenario: Counts come from the coordinator

- **GIVEN** the panel shows turn and token counts for several children
- **WHEN** those figures are refreshed
- **THEN** they come from the delegation coordinator on the existing
  poll-on-redraw
- **AND** Smith derives no count of its own from the event stream

#### Scenario: A long detail cannot displace the clock

- **GIVEN** a child's activity text is wider than the panel
- **WHEN** the row renders
- **THEN** the activity clips
- **AND** the elapsed clock remains docked at the right edge

#### Scenario: No reviewed projection is available

- **GIVEN** a child runs a tool with no reviewed display schema
- **WHEN** the panel renders its row
- **THEN** it names the tool and an honest unavailable label
- **AND** does not display raw argument values

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
