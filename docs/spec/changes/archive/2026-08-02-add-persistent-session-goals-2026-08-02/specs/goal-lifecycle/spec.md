## ADDED Requirements

### Requirement: One explicit persisted goal per eligible session

An eligible persistent root session SHALL retain at most one versioned goal
containing a stable identity, bounded objective, lifecycle status, optional
positive token budget, token usage with provenance, active elapsed time, bounded
stopped reason, and created/updated timestamps. Goal state MUST NOT grant tool
authority, create project metadata, or replace per-turn todo state.

#### Scenario: User creates the first goal

- **GIVEN** an eligible root session has no current goal
- **WHEN** the user explicitly creates a bounded objective
- **THEN** Smith persists a new stable goal identity with `active` status
- **AND** token and elapsed accounting begin at that activation boundary

#### Scenario: Unfinished goal already exists

- **GIVEN** the session has a goal whose status is not `complete`
- **WHEN** another create operation is attempted
- **THEN** Smith rejects replacement with a local or tool-visible conflict
- **AND** preserves the existing goal and its usage exactly

#### Scenario: Complete goal is replaced

- **GIVEN** the current goal is `complete`
- **WHEN** the user explicitly creates a new objective
- **THEN** Smith assigns a new goal identity and zeroed accounting baseline
- **AND** retains no stale continuation for the completed identity

### Requirement: Restricted model goal tools

Smith SHALL expose `get_goal`, `create_goal`, and `update_goal` through one
goal ability. Tool instructions MUST prohibit inferred creation and budgets;
model status updates MUST be limited to `complete` or `blocked`, while pause,
resume, edit, budget mutation, and clear remain user/system controls.

#### Scenario: Ordinary task does not request a goal

- **GIVEN** the user asks for ordinary work without requesting a persistent goal
- **WHEN** the model chooses its tools
- **THEN** it does not call `create_goal`
- **AND** Smith does not infer goal state from task length or todo usage

#### Scenario: Explicit budgeted goal is requested

- **GIVEN** the user explicitly requests an objective and positive token budget
- **WHEN** the model calls `create_goal` with those values
- **THEN** Smith validates and persists the requested budget
- **AND** returns current usage and remaining-budget evidence

#### Scenario: Model attempts a user-controlled transition

- **GIVEN** a current goal exists
- **WHEN** `update_goal` requests pause, resume, budget change, clear, or an
  objective replacement
- **THEN** Smith rejects the request as outside the model tool contract
- **AND** leaves the goal unchanged

#### Scenario: Model completes the goal

- **GIVEN** all required work is genuinely complete
- **WHEN** the model calls `update_goal` with `complete`
- **THEN** Smith finalizes in-flight accounting exactly once and persists the
  complete status
- **AND** returns final usage evidence for the model's user report

### Requirement: Goal lifecycle controls continuation

Only an `active` goal SHALL be eligible for automatic continuation. Paused,
blocked, usage-limited, budget-limited, complete, or cleared goals MUST NOT
start automatic work until an explicit valid user transition makes them active.

#### Scenario: Active turn ends with unfinished goal

- **GIVEN** a serving turn reaches a normal terminal boundary
- **AND** its goal remains `active`
- **WHEN** the session becomes idle
- **THEN** Smith attempts one conditional internal continuation
- **AND** does not require or fabricate a new user message

#### Scenario: Model declares a genuine blocker

- **GIVEN** the same material blocker has met the model-tool policy threshold
- **WHEN** the model marks the goal `blocked`
- **THEN** Smith persists the stopped status and bounded blocker reason
- **AND** starts no further automatic turn

#### Scenario: User resumes a stopped goal

- **GIVEN** a paused, blocked, or usage-limited condition has changed
- **WHEN** the idle user explicitly resumes the goal
- **THEN** Smith makes it active and starts a fresh continuation attempt
- **AND** a resumed blocked goal begins a new blocker audit

#### Scenario: Budget-limited goal is resumed without more budget

- **GIVEN** observed usage is at or above the current token budget
- **WHEN** the user requests resume without increasing or removing the budget
- **THEN** Smith rejects reactivation
- **AND** preserves `budget_limited` and the actual usage evidence

#### Scenario: User raises a budget before resume

- **GIVEN** a budget-limited goal is idle
- **WHEN** the user raises its budget above observed usage or removes the
  budget
- **THEN** Smith persists the requested budget mutation but keeps the goal
  stopped
- **AND** a later explicit resume may make the goal active

### Requirement: Honest goal usage and budget enforcement

Smith SHALL charge provider-reported uncached input plus output tokens after
goal activation and SHALL exclude cached input. It SHALL derive elapsed time
only while an active goal owns a serving turn. A token budget MUST be enforced
at observed safe boundaries with explicit provenance and MUST NOT be described
as a pre-spend hard cap.

#### Scenario: Provider reports cached and uncached usage

- **GIVEN** an active goal receives provider-reported input, cached-input, and
  output counters
- **WHEN** Smith accounts the response
- **THEN** charged goal tokens equal uncached input plus output
- **AND** cached input is not charged again

#### Scenario: Response crosses the token budget

- **GIVEN** an active budgeted goal is below its budget before a provider call
- **WHEN** reported usage reaches or exceeds the budget at the next safe
  boundary
- **THEN** Smith records the actual possibly-overshot usage and changes status
  to `budget_limited`
- **AND** starts no later automatic turn

#### Scenario: Budget accounting evidence is unavailable

- **GIVEN** an active goal has an explicit token budget
- **WHEN** a completed usage boundary omits the trustworthy counters required
  for enforcement
- **THEN** Smith stops the goal as `blocked` with
  `accounting_unavailable`
- **AND** does not display a guessed remaining budget or continue automatically

#### Scenario: Process is not serving goal work

- **GIVEN** the goal is idle, paused, stopped, or Smith is not running
- **WHEN** wall-clock time passes
- **THEN** active elapsed usage does not increase
- **AND** later resume starts a new in-process timing baseline

### Requirement: Terminal and exceptional outcomes stop safely

Smith SHALL finalize attributable in-flight usage once before applying a
terminal or stopped transition. A user goal interruption SHALL pause it, an
external account limit SHALL make it usage-limited, and an unrecoverable turn
error SHALL block it unless a more specific completed or budget-limited state
already won the serialized transition.

#### Scenario: User interrupts active goal work

- **GIVEN** an automatic or user-steered goal turn is serving
- **WHEN** the user invokes the goal-aware interrupt action
- **THEN** Smith interrupts that turn and commits the goal as `paused`
- **AND** final token/time deltas are accounted exactly once

#### Scenario: Provider usage limit stops the turn

- **GIVEN** an active goal turn reaches an external account usage limit
- **WHEN** the terminal limit is committed
- **THEN** Smith records `usage_limited` with the existing structured limit
  evidence
- **AND** no automatic retry starts

#### Scenario: Unrecoverable turn error occurs

- **GIVEN** retries are exhausted or a non-retryable turn error occurs
- **WHEN** Smith commits the terminal error
- **THEN** the active goal becomes `blocked` with a bounded error category
- **AND** continuation cannot loop on the same error
