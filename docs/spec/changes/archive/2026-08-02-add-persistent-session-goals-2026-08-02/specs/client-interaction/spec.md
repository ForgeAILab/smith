## ADDED Requirements

### Requirement: Local goal controls use one typed host path

The interactive TUI SHALL provide local goal summary, create, edit, budget,
pause, resume, and clear commands through the existing command registry and
one typed goal-control service. Intercepted controls MUST issue no provider
request and MUST use the same validation regardless of direct arguments or
future picker presentation.

#### Scenario: User creates a goal locally

- **GIVEN** the eligible session is idle with no unfinished goal
- **WHEN** the user submits `/goal <objective>`
- **THEN** Smith validates and commits the goal locally without a provider
  request for the command itself
- **AND** the controller may then start an attributed internal goal turn

#### Scenario: User requests a summary

- **GIVEN** any current goal status
- **WHEN** the user submits bare `/goal`
- **THEN** Smith renders bounded objective, status, elapsed time, token usage
  provenance, budget, and stopped reason locally
- **AND** creates no canonical user message or provider request

#### Scenario: User changes a goal budget

- **GIVEN** an idle goal has a positive budget or is budget-limited
- **WHEN** the user submits `/goal budget <positive-tokens|none>`
- **THEN** Smith validates and commits the new optional budget locally
- **AND** a stopped goal remains stopped until the user separately resumes it

#### Scenario: User mutates a busy goal unsafely

- **GIVEN** a turn is serving
- **WHEN** the user attempts create, edit, budget, resume, or clear
- **THEN** Smith refuses the mutation locally as busy and preserves the draft
  or command arguments
- **AND** the serving goal state remains unchanged

#### Scenario: Objective requires deferred attachment handling

- **GIVEN** a goal objective exceeds the direct bound or depends on image/paste
  attachment materialization
- **WHEN** the user attempts creation or edit
- **THEN** Smith reports the unsupported bounded-objective requirement locally
- **AND** does not create attachment files or silently truncate the objective

### Requirement: Goal-aware interruption pauses automatic work

The interactive goal pause action SHALL be available while a goal turn is
serving. It MUST serialize the pause request with interruption and final
accounting so the goal reaches one paused state; ordinary non-goal interruption
retains its existing turn-local behavior.

#### Scenario: Pause active goal turn

- **GIVEN** an active goal owns the serving turn
- **WHEN** the user invokes `/goal pause` or the documented goal-aware interrupt
- **THEN** Smith enters visible interrupting state, cancels that turn, and
  commits `paused` after final accounting
- **AND** no automatic continuation is admitted between interruption and pause

#### Scenario: Interrupt ordinary turn

- **GIVEN** no active goal owns the serving turn
- **WHEN** the user invokes the ordinary interrupt action
- **THEN** Smith cancels only that turn under the existing contract
- **AND** creates or changes no goal state

### Requirement: Compact replay-equivalent goal visibility

The TUI SHALL derive one compact non-focusable goal projection from restored
state and typed runtime events. It SHALL distinguish goal status from the
per-turn todo pane, show token and elapsed provenance honestly, remain legible
without color, and produce equivalent live and journal-replay state.

#### Scenario: Active goal renders with a todo plan

- **GIVEN** an active persistent goal and a public todo plan for the current
  turn
- **WHEN** Smith renders the composer area
- **THEN** compact goal status remains distinguishable from todo item progress
- **AND** neither projection is duplicated into ordinary transcript history

#### Scenario: Goal reaches a stopped state

- **GIVEN** the current goal becomes paused, blocked, usage-limited,
  budget-limited, or complete
- **WHEN** the typed event commits
- **THEN** the compact projection updates status and bounded reason in place
- **AND** status persists across later idle rendering and compatible resume

#### Scenario: Token usage is unknown

- **GIVEN** an unbudgeted goal lacks provider-reported usage evidence
- **WHEN** Smith renders goal status or summary
- **THEN** it labels token usage unknown while retaining derived elapsed time
- **AND** does not display zero as if it were reported

#### Scenario: Journaled goal state is replayed

- **GIVEN** the journal contains durability-aligned goal updates
- **WHEN** the TUI rebuilds presentation from replay
- **THEN** it reaches the same goal projection as live reduction
- **AND** replay triggers no host control or automatic turn

### Requirement: Goal commands are discoverable and bounded

`/help` SHALL list the supported goal command grammar and state that goal work
may span multiple provider turns. Errors for missing goals, invalid status,
busy mutation, invalid objective, and invalid budget MUST be local, bounded,
and actionable.

#### Scenario: User opens local help

- **GIVEN** goal capability is available for the current session
- **WHEN** the user submits `/help`
- **THEN** help lists summary, create, edit, budget, pause, resume, and clear
  forms
- **AND** names persistent multi-turn execution and local control behavior

#### Scenario: Goal capability is unavailable

- **GIVEN** the session is ephemeral, a child, or a review surface
- **WHEN** the user attempts `/goal`
- **THEN** Smith explains locally why persistent goals are unavailable
- **AND** neither provider work nor local goal state is created
