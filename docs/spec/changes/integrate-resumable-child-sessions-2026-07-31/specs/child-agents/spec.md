## ADDED Requirements

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

## MODIFIED Requirements

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
