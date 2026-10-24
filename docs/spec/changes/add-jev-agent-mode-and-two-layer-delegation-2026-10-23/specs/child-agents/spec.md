## RENAMED Requirements

- FROM: `### Requirement: One-level agent hierarchy`
- TO: `### Requirement: Bounded two-layer agent hierarchy`
- FROM: `### Requirement: Profile-controlled root delegation availability`
- TO: `### Requirement: Profile-controlled delegation availability`
- FROM: `### Requirement: Root agents can inspect a child's committed conversation`
- TO: `### Requirement: Agents can inspect authorized descendant conversations`

## MODIFIED Requirements

### Requirement: Bounded two-layer agent hierarchy

Smith SHALL support Root (depth 0) -> Layer-1 clarifier/lead (depth 1) -> Layer-2
worker (depth 2), with `DelegationLimits::max_depth = 2` explicitly composed by
Smith. The neutral runtime default SHALL remain 1. Session lineage MUST retain
immediate parent, root session, depth, and hierarchical child ID. Spawn admission
MUST reject a parent at or above the effective maximum even if an injected call
bypasses tool visibility. Layer-1 delegation requires its effective profile and
host policy; Layer-2 agents MUST never receive a delegation-management tool.

#### Scenario: Eligible lead spawns a worker

- **GIVEN** Root spawned a depth-1 lead with delegation enabled
- **WHEN** the lead requests an authorized child within capacity
- **THEN** Smith creates a depth-2 worker with the lead as immediate parent
- **AND** the worker retains the same root identity and receives no agent tool

#### Scenario: Leaf attempts a third layer

- **GIVEN** a depth-2 worker emits an injected spawn-shaped call
- **WHEN** runtime admission validates its lineage
- **THEN** it rejects the request as a depth violation
- **AND** no third-layer runtime, lease, or provider turn is created

#### Scenario: Neutral consumer retains depth-one behavior

- **GIVEN** a runtime consumer uses default DelegationLimits
- **WHEN** a direct child attempts to create its own coordinator or child
- **THEN** the depth-one default rejects the attempt


### Requirement: Parent-selected child specification

Each eligible parent MUST choose each direct child's task, expected result, provider/model,
turn/token/deadline limits, permission policy, and workspace policy. Workspace
policy SHALL be one of shared project, explicit directory, isolated worktree,
or read-only view and MUST be visible in child lifecycle events. The root MAY
additionally select one registered child-enabled agent profile per spawn, which
SHALL resolve through the same preflighted route the user-invoked presets use.
An unselected profile SHALL inherit the parent's, and an unresolvable profile
MUST fail the spawn without creating a child.

A Layer-1 parent's selected profile, posture, tool scope, workspace, deadlines,
approval, and budget MUST narrow the Root-authorized upper bound. Jev output and
profile descriptions SHALL NOT authorize a new profile, workspace, tool, or child.
Profile eligibility is resolved through the same host-preflighted route at either
layer; retained recovery MUST NOT widen the recorded authority.

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

#### Scenario: Lead preserves parent authority

- **GIVEN** a read-only Layer-1 lead selects a write-capable worker profile
- **WHEN** Smith intersects the route with the lead and Root upper bounds
- **THEN** the worker remains read-only or the spawn is refused
- **AND** the profile name does not grant write authority

### Requirement: Child management controls

Smith SHALL expose operations to spawn, list, send or follow up, wait, fetch
result, resume, and stop direct children to eligible Root and Layer-1 parents. Operations MUST be addressed by
stable child ID and return structured lifecycle or error results.

The `agent.wait` operation SHALL NOT hold the parent tool call open while a
child runs. It SHALL accept an optional `timeout_ms` whose resolved
configuration paths SHALL be
`profiles.<name>.child_agents.wait_default_timeout_ms` and
`profiles.<name>.child_agents.wait_max_timeout_ms`. The default and maximum
ranges SHALL be `0..=300_000` and `1..=300_000` milliseconds respectively, and
the built-in values SHALL both be 300,000 milliseconds. The resolved default
MUST NOT exceed the resolved maximum. A requested timeout of zero SHALL be an
immediate status check that arms nothing; a requested timeout above the
resolved maximum SHALL be clamped to that maximum and the result SHALL report
that it was clamped. Positive timeouts SHALL NOT be raised to a minimum.

When the addressed child is terminal at call time, `agent.wait` SHALL return
its terminal status and acknowledge the outcome as `agent.result` does. When
the child is running and the timeout is non-zero, `agent.wait` SHALL return
immediately with a structured `running` result that reports the armed wake
duration and deadline, and SHALL arm a wait timer for that child as "Timed
wait wakes an idle parent" requires. The child's lifecycle, limits, and exact
state MUST be unchanged, and its terminal outcome MUST remain must-deliver for
automatic delivery. The model-facing description SHALL state that `wait`
returns at once, that the parent should end its turn, and that it will be
woken by the child's terminal outcome or by the timer, whichever comes first.

Spawn/follow-up/resume controls SHALL remain bound to the immediate parent's
catalog. Root host inspection and explicit operator subtree stop MAY traverse
validated owning coordinators, but a lead MUST NOT mutate a sibling's workers.
List/status/history views MAY include authorized descendants without transferring
their ready outcomes into the ancestor's delivery ledger. The immediate-return
wait/timer contract applies at either eligible layer.

#### Scenario: Waiting on a running child returns at once

- **GIVEN** the root calls `agent.wait` for a child that remains active
- **WHEN** the call is executed
- **THEN** `agent.wait` returns a successful `running` result reporting the
  armed wake duration and deadline without waiting for the child
- **AND** the parent tool call is released for normal completion or new work
- **AND** the child continues running with its original lifecycle and limits

#### Scenario: Waiting on a child that already finished

- **GIVEN** the root calls `agent.wait` for a child whose task is terminal
- **WHEN** the call is executed
- **THEN** `agent.wait` returns the terminal/idle child status
- **AND** the outcome is acknowledged so no child-completion continuation
  delivers it again
- **AND** no wait timer is armed

#### Scenario: Zero timeout is a status check

- **GIVEN** the root requests `timeout_ms = 0` for a running child
- **WHEN** the call is executed
- **THEN** Smith returns the child's current status
- **AND** arms no wait timer

#### Scenario: Requested timeout above the maximum

- **GIVEN** the resolved maximum is 300,000 milliseconds
- **WHEN** the root requests `timeout_ms = 900000` for a running child
- **THEN** Smith arms a 300,000 millisecond timer
- **AND** the result reports the effective duration and that it was clamped

#### Scenario: Short timer is not raised to a floor

- **GIVEN** the root requests `timeout_ms = 1` for a running child
- **WHEN** the call is executed
- **THEN** Smith arms a one millisecond timer and returns immediately
- **AND** guidance tells the parent to end its turn and await the automatic wake

#### Scenario: A lead waits without holding a tool call

- **GIVEN** a Layer-1 lead has a running Layer-2 worker
- **WHEN** the lead calls agent.wait with a positive bounded timeout
- **THEN** the call returns immediately and arms the lead-local timer
- **AND** the lead may park and Root still sees its logical task as running

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

For a two-layer tree, canonical worker lifecycle events SHALL be visible at Root
with the original hierarchical identity, but the worker's protected model-facing
outcome SHALL belong only to its immediate lead. Root receives the lead's final
integrated outcome, not an automatically injected copy of every worker result.
Forwarding MUST preserve redaction and MUST NOT double-count usage or fabricate
additional canonical completion identities.

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

#### Scenario: Worker progress is observed without duplicate model delivery

- **GIVEN** child-1.1 completes while its lead is parked
- **WHEN** its lifecycle reaches Root and its protected outcome wakes the lead
- **THEN** Root can inspect the worker but does not consume the lead-local outcome
- **AND** Root later receives one integrated child-1 result

### Requirement: Bounded ephemeral children

Smith SHALL enforce configurable process, session, and parent running-child
concurrency limits plus cumulative per-child turn, token, deadline, and
retention limits. Live child executions MUST stop with the parent execution or
Smith process and MUST NOT restart automatically on parent resume. For durable
sessions, Smith SHALL retain the bounded child-session record and committed
state for explicit compatible follow-up or resume. For intentionally
non-persistent sessions, children remain explicitly process-ephemeral.

Root and Layer-1 coordinators SHALL share the same process-wide
Arc<DelegationCapacity> while retaining per-parent bounds. Parked leads and live
workers MUST both count against the shared pool. Descendant usage SHALL enforce
ancestor budgets without duplicating run totals. Shutdown, partial startup,
failure, and cancellation MUST release each lease once and keep queueing bounded.

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

#### Scenario: Two leads contend for process capacity

- **GIVEN** two roots or leads share a capacity pool at its limit
- **WHEN** either requests another worker
- **THEN** the host rejects or boundedly queues the request without exceeding the pool
- **AND** parking does not evade a lease or deadlock all progress

### Requirement: Profile-controlled delegation availability

An effective profile SHALL narrow delegation at Root and eligible Layer-1
placements. When delegation is disabled, Smith MUST omit both the delegation
instruction fragment and the agent tool from the sealed capability registry.
An eligible depth-1 profile with delegation enabled MAY receive both under host
policy and max_depth=2; depth-2 surfaces MUST remain non-delegating regardless of
profile settings. Omitted delegation SHALL retain its inherited true value while
placement, posture, tool scope, and host authority remain upper bounds.

#### Scenario: Disable delegation on a lead

- **GIVEN** a depth-1 profile resolves delegation=false
- **WHEN** Smith composes the lead
- **THEN** neither delegation prompt nor agent tool is registered
- **AND** unrelated eligible tools are unchanged

#### Scenario: Existing main profile omits delegation

- **GIVEN** a main profile has no delegation override
- **WHEN** it resolves under the existing host policy
- **THEN** Root delegation remains enabled as before
- **AND** the prompt and registry agree

#### Scenario: Leaf inherits delegation true

- **GIVEN** a depth-2 worker inherits delegation=true
- **WHEN** Smith composes it
- **THEN** both delegation prompt and tool are stripped
- **AND** runtime depth admission still refuses injected child creation


### Requirement: Delegation from a harness session

An eligible Root or Layer-1 session running on a harness SHALL be able to use the `agent` tool
through the bridge with the actions, profile selection, tool scope,
workspace rules, wait policy, and bounded two-layer hierarchy it has in a native
session. A child MAY run on any child-enabled profile, whether that profile
uses a provider or a harness.

A terminal child outcome the CLI's model read through `wait` or `result`
SHALL be acknowledged against that bridged tool call, so it is not delivered
again. An outcome it did not read SHALL be delivered by a child-completion
turn that runs on the harness.

#### Scenario: A harness parent spawns a provider child

- **GIVEN** a root session on `cli/claude-code/opus` with delegation enabled
- **WHEN** the CLI calls `agent` with action `spawn` and a provider profile
- **THEN** the child starts on that provider under the existing child rules
- **AND** the spawn is approved or refused as a native spawn would be

#### Scenario: The CLI waits for its child

- **GIVEN** the CLI spawned a child and read its result with `wait`
- **WHEN** the parent turn ends
- **THEN** no child-completion turn starts for that result

#### Scenario: The CLI does not wait

- **GIVEN** the CLI spawned a child and ended its turn
- **WHEN** the child completes
- **THEN** one child-completion turn delivers the result to the CLI's
  resumed session

#### Scenario: Delegation disabled on the profile

- **GIVEN** a harness session whose main profile disables delegation
- **WHEN** a turn runs
- **THEN** `agent` is not bridged and no delegation guidance is delivered

#### Scenario: A harness leaf cannot delegate

- **GIVEN** a depth-2 child or delegation-disabled lead running on a harness
- **WHEN** the CLI lists the bridged tools
- **THEN** `agent` is absent

#### Scenario: Harness lead integrates a worker

- **GIVEN** an authorized depth-1 lead runs on a harness with delegation enabled
- **WHEN** its worker completes and a continuation is admitted
- **THEN** the lead continuation runs on its harness under the same episode barrier
- **AND** Root receives no premature completed result

### Requirement: Agents can inspect authorized descendant conversations

Smith SHALL expose `agent` action `transcript` for a parent's authorized subtree.
Root may inspect its descendants; a Layer-1 lead may inspect only its workers. The action SHALL return ordered canonical conversation
content with message roles, child/session identity, tool call/result pairing,
current lifecycle state, and stable pagination. Accessible content SHALL
include the delegated task, assistant messages, reasoning permitted by existing
visibility policy, tool invocations/results, and final replies. Internal
instructions, credentials, and protected reasoning MUST remain excluded.

Descendant lookup SHALL resolve the owning coordinator through validated lineage,
including retained dormant catalogs without starting a runtime or consuming
completion readiness. A syntactically valid hierarchical ID MUST NOT grant access
to a sibling or another root. Existing pagination, artifact ownership, and
redaction contracts apply at both layers.

#### Scenario: The parent diagnoses a running child

- **GIVEN** a child has committed assistant messages and tool results
- **WHEN** its parent requests `agent transcript` for that child
- **THEN** the parent receives those messages in order with their roles and
  tool identities, rather than only a status summary
- **AND** an unfinished model request is not reported as a completed result

#### Scenario: A large entry requires more than one context insertion

- **GIVEN** a child tool result exceeds the ordinary inline output budget
- **WHEN** the parent reads the transcript page containing it
- **THEN** the response remains bounded and provides a parent-readable
  reference to the complete accessible entry
- **AND** reading successive pages/references does not silently omit content

#### Scenario: A live child appends while the parent pages history

- **GIVEN** the parent is reading a transcript snapshot over multiple pages
- **WHEN** the child commits additional messages
- **THEN** existing page cursors continue to identify the same ordered snapshot
- **AND** later content is reachable through an explicit subsequent read

#### Scenario: Transcript access preserves ownership and visibility

- **GIVEN** a request names an unknown child or one outside the caller's authorized subtree
- **WHEN** the transcript action resolves the target
- **THEN** it returns a clear error without reading the unrelated history
- **AND** successful reads apply existing credential and reasoning redaction

#### Scenario: Root inspects a retained grandchild

- **GIVEN** Root owns child-1 and its retained child-1.1 with no live binding
- **WHEN** Root requests the grandchild transcript
- **THEN** Smith reads permitted canonical history through the owning catalog without provider work
- **AND** completion readiness and child lifecycle are unchanged

### Requirement: Parked-state shutdown and headless behavior

A parked parent SHALL remain cancellable and SHALL shut down in bounded order.
On shutdown, Smith SHALL stop cache-maintenance scheduling, cancel every armed
wait timer, cancel in-flight synthetic requests, freeze child-completion and
wait-timeout admission, apply existing child stop/wait/durability policy,
persist the latest compatible resume capsule when enabled, release
provider/cache resources according to adapter policy, and emit one terminal
shutdown sequence.

A headless host MAY remain process-alive while required child work is pending
under ordinary child policy, but MUST NOT remain alive solely to preserve a
provider cache or to honor an armed wait timer after required work is
complete.

Required work SHALL include the complete Layer-1 task episode while its Layer-2
work, terminal persistence, or continuation admission remains pending. Shutdown
MUST propagate through both layers and outstanding DirectHost requests in bounded
order. Retained idle records and stale timer notifications alone are not required
work and MUST NOT prevent headless completion.

#### Scenario: User exits while parent is parked

- **GIVEN** a parked parent with a running child and an armed wait timer
- **WHEN** the host begins shutdown
- **THEN** no new cache maintenance, child-completion, or wait-timeout turn
  starts
- **AND** the configured child shutdown policy applies
- **AND** shutdown remains bounded

#### Scenario: Required child work completes in headless mode

- **GIVEN** a headless host stayed alive to receive a required child outcome
- **WHEN** the outcome is integrated and no other required work remains
- **THEN** the host completes its ordinary terminal sequence
- **AND** it does not extend process life for cache retention or a wait timer

#### Scenario: Headless waits through a lead parking turn

- **GIVEN** a lead finished turn-1 while its worker still runs
- **WHEN** headless evaluates completion
- **THEN** the process remains alive until worker integration and final Root delivery settle
- **AND** it does not exit on the lead parking prose or wait afterward solely for a timer


### Requirement: Explicit user-invoked child presets

Registered `@agent` references SHALL map to bounded host-controlled depth-one
entry presets. Before dispatch Smith MUST show inherited provider/model,
workspace posture, limits, expected result, and provider-spend confirmation;
the preset cannot widen parent authority.

A depth-1 entry MAY be an explicitly eligible lead under the bounded two-layer
policy. Pre-dispatch information SHALL include its delegation and interaction
posture. Legacy read-only reviewer presets SHALL remain leaf compositions unless
the host explicitly resolves them as delegating leads; spend confirmation MUST
NOT be treated as approval for arbitrary worker tools or workspaces.

#### Scenario: Invoke a read-only reviewer
- **GIVEN** the user submits `@review` with a bounded task and delegation=false
- **WHEN** they confirm provider spend
- **THEN** Smith creates one attributed read-only child on the parent model
- **AND** the child has no write, shell-mutation, or child-management ability

#### Scenario: Child preset attempts nesting
- **GIVEN** a direct preset child with delegation=false emits a delegation call
- **WHEN** Smith authorizes it
- **THEN** the effective profile/host delegation ceiling rejects the call
- **AND** no grandchild is created

#### Scenario: Invoke a designated lead preset

- **GIVEN** a child-enabled lead is approved for depth-1 delegation
- **WHEN** the user invokes and confirms the displayed lead route
- **THEN** Smith composes its depth-2 capability under the same parent upper bounds
- **AND** the confirmation does not grant unrelated tool or workspace authority


### Requirement: Profile-based direct child composition

Smith SHALL create an explicit direct child from a child-enabled profile using
the same typed profile resolution and standard runtime composition boundaries
used for the main agent. Before dispatch it MUST show the selected profile,
bounded instruction summary, provider/model, effective limits, resolved
workspace posture, and provider-spend confirmation.

Direct refers to the immediate parent's child: Root may create a Layer-1 lead,
and an eligible lead may create a Layer-2 worker using the same typed profile
route. Writable child access MUST still satisfy profile posture, requested full
tool scope, and a non-read-only workspace together. A profile alone SHALL NOT
widen the host ceiling, and Layer-2 never receives child-management tools.

#### Scenario: Invoke a child on the parent's model
- **GIVEN** a child-enabled read-only review profile with delegation=false
  resolves to the parent's provider/model
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
  host placement/depth/tool/workspace ceiling, and profile posture
- **AND** any incompatible value is rejected or source-explainably narrowed

#### Scenario: Lead composes a worker profile

- **GIVEN** a Layer-1 lead is authorized to delegate within its narrowed workspace
- **WHEN** it selects a preflighted child-enabled code profile
- **THEN** Smith uses the ordinary route and composes a depth-2 leaf
- **AND** write capability remains the intersection of posture, scope, workspace, and host policy


### Requirement: Child timeline navigation

Smith SHALL expose current and completed authorized descendants through `/agent`,
`/timeline`, and keyboard selection of the delegated-work panel, with stable
previous, next, and parent navigation. Child inspection MUST be a temporary
read-only view and MUST NOT move persistent input focus away from the root
composer. Turn, token, session, and workspace figures shown for a child MUST
come from the delegation coordinator, never from client-side estimation.

For a two-layer tree, navigation SHALL follow the same numeric parent-before-
descendant order as the panel. Parent navigation from a worker SHALL identify
its Layer-1 lead, and returning to Root MUST preserve the root composer and
transcript. Inspection alone does not authorize a worker follow-up.

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

## ADDED Requirements

### Requirement: Logical delegated task episodes span internal continuation turns

A delegated task SHALL span its initial turn and associated internal descendant-
completion and wait-timeout turns until quiescent completion, NeedsInput, failure,
cancellation, or shutdown. Root MUST keep a parked Layer-1 task Running while
pending delegated work exists. Only the final quiescent result SHALL be committed
as Completed and projected as Idle; a retained sequence/cursor and atomic pending-
work check MUST prevent lost wakes, skipped fast turns, and duplicate completion.

#### Scenario: Lead parks and later synthesizes

- **GIVEN** child-1 spawns child-1.1 and ends its first turn to park
- **WHEN** the worker completes and an internal lead turn integrates the result
- **THEN** Root receives exactly one final child-1 result from the synthesis turn
- **AND** no earlier parking reply is marked completed or delivered as final

#### Scenario: Fast continuation precedes collector wake

- **GIVEN** a continuation starts and finishes before its collector is polled
- **WHEN** the collector follows its retained episode cursor
- **THEN** it observes that turn and its terminal outcome without a broadcast race
- **AND** it neither skips the result nor executes the provider again

### Requirement: Failed or stopped workers wake their parked lead

Failed and stopped/cancelled worker outcomes SHALL be protected, ordered,
redaction-safe for model delivery, and admitted exactly once to the immediate
parent at an eligible idle boundary. A parked lead MUST be woken even when no
worker completed successfully. Cancellation and budget exhaustion MUST NOT be
converted into success or extend limits.

#### Scenario: Only worker fails

- **GIVEN** a lead is parked for its only Layer-2 worker
- **WHEN** that worker fails or is stopped
- **THEN** a typed terminal outcome wakes the lead for recovery or final reporting
- **AND** Root does not receive the lead's old parking text as a successful result

### Requirement: Designated Layer-1 agents may interact through the root host

Smith SHALL restrict direct child interaction to host-approved depth-1 profiles
with direct_interaction=true and an available root broker. Such a profile MAY
present an attributed questionnaire directly. Default workers SHALL use
ReturnToParent, and depth-2 workers MUST never receive DirectHost.
Questionnaire answers SHALL remain task information, not approval or permission.

#### Scenario: Clarifier asks the user directly

- **GIVEN** an authorized Layer-1 clarifier has the root broker and explicit opt-in
- **WHEN** it calls ask_user
- **THEN** the root surface presents the originating request without rerouting its identity
- **AND** the answer resumes only that exact child exchange and grants no tool authority

#### Scenario: Worker escalates through its lead

- **GIVEN** a depth-2 worker needs information its context lacks
- **WHEN** it calls ask_user
- **THEN** its protected NeedsInput goes to its immediate Layer-1 parent
- **AND** the lead answers or explicitly escalates to Root without opening a worker modal

#### Scenario: Direct surface is unavailable

- **GIVEN** a lead profile requests direct interaction in headless mode without a broker
- **WHEN** a questionnaire is required
- **THEN** the existing typed input-required or ReturnToParent path applies
- **AND** no hidden modal or indefinite wait is created
