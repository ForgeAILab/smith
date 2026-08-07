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
fetch result, and stop direct children. Operations MUST be addressed by stable
child ID and return structured lifecycle or error results.

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

### Requirement: Safe parent reporting

Child progress and results SHALL appear immediately in the active host and
enter the parent session inbox. They MUST be introduced to the parent model only
at a safe provider/tool boundary, and a final child result MUST not be silently
dropped during progress coalescing. A spawn SHALL announce itself exactly once
in the parent transcript, and every terminal child outcome SHALL keep its own
attributed line.

#### Scenario: Child finishes during parent streaming

- **GIVEN** the parent model is streaming when a child completes
- **WHEN** the final child result arrives
- **THEN** the TUI marks the child complete immediately
- **AND** the result is queued for the parent's next safe continuation
- **AND** the active parent stream is not interrupted

#### Scenario: A spawn announces itself once

- **GIVEN** the model spawns a child and the runtime reports it spawned
- **WHEN** Smith renders the parent transcript
- **THEN** exactly one row reports the spawn, carrying the child id, the task
  excerpt, the profile, the tool scope, and the workspace posture
- **AND** Smith adds no separate started notice repeating those terms

#### Scenario: Terminal outcomes stay attributed

- **GIVEN** a child completes, needs input, is interrupted, is stopped, or
  fails
- **WHEN** Smith renders the parent transcript
- **THEN** that outcome keeps its own attributed line where it occurred in time
- **AND** it is not folded back into the spawn row, because it is new
  information rather than a repetition of the spawn

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

Smith SHALL expose current and completed direct children through `/agent` and
`/timeline` with stable previous, next, and parent navigation. Child inspection
MUST be a temporary read-only view and MUST NOT move persistent input focus
away from the root composer.

#### Scenario: Inspect consecutive child results
- **GIVEN** two attributed children completed
- **WHEN** the user opens one and requests next
- **THEN** Smith displays the next child's bounded lifecycle and result
- **AND** returning to parent restores the same composer draft and scroll state

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
