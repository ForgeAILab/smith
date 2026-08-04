## ADDED Requirements

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

Smith SHALL enforce configurable process, session, and parent child-concurrency
limits plus each child's turn/token/deadline limits. Children MUST stop with the
parent or Smith process and MUST NOT restart on session resume.

#### Scenario: Concurrency limit is reached

- **GIVEN** the parent has reached its configured running-child limit
- **WHEN** it requests another child
- **THEN** Smith returns a capacity result or queues it according to explicit
  policy
- **AND** does not exceed the limit

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
