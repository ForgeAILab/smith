## MODIFIED Requirements

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

## ADDED Requirements

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
