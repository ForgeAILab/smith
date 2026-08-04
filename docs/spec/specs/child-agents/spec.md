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
or read-only view and MUST be visible in child lifecycle events.

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
dropped during progress coalescing.

#### Scenario: Child finishes during parent streaming

- **GIVEN** the parent model is streaming when a child completes
- **WHEN** the final child result arrives
- **THEN** the TUI marks the child complete immediately
- **AND** the result is queued for the parent's next safe continuation
- **AND** the active parent stream is not interrupted

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
