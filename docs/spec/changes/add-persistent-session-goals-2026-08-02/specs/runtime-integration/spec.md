## ADDED Requirements

### Requirement: Conditional internal goal turns

Agent Runtime SHALL provide a bounded provenance-bearing internal turn that can
be accepted only if the session remains idle at one serialized decision
boundary. Internal goal input MUST NOT append a user-role canonical history
message, queue ahead of real user work, bypass ordinary runtime policy, or
survive as unattributed background work.

#### Scenario: Idle session accepts a continuation

- **GIVEN** a persistent root session is idle with an active goal
- **WHEN** its controller conditionally requests one goal continuation
- **THEN** Agent Runtime accepts an attributed internal turn
- **AND** applies the normal provider, context, tool, approval, workspace,
  checkpoint, cancellation, retry, and global-limit contracts

#### Scenario: Real user input wins the idle race

- **GIVEN** an active goal becomes idle while the user submits a prompt
- **WHEN** user and automatic work compete for the session boundary
- **THEN** the automatic request returns busy or is skipped rather than queued
- **AND** the user's ordinary turn retains submission priority

#### Scenario: Continuation history is inspected

- **GIVEN** one or more internal goal turns completed
- **WHEN** canonical session history is persisted or resumed
- **THEN** it contains no fabricated user continuation message
- **AND** lifecycle/checkpoint evidence still identifies each internal turn and
  its goal source

### Requirement: Reusable goal harness component

The shared runtime SHALL own goal state decoding, validation, context
contribution, tool-result mutation, usage accounting inputs, turn-commit
mutation, typed event projection, and controller deduplication as one versioned
harness capability. Smith MUST consume that capability through its one runtime
factory and MUST NOT retain a parallel TUI or headless goal state machine.

#### Scenario: Goal tool mutation commits

- **GIVEN** a goal tool returns a valid state mutation
- **WHEN** the harness processes its exact tool output
- **THEN** the component state, model-facing result, and typed goal event commit
  at one durability-aligned boundary
- **AND** live clients never observe a goal event for a discarded mutation

#### Scenario: Goal context is planned

- **GIVEN** a goal exists for the serving turn
- **WHEN** Agent Runtime builds the provider request
- **THEN** it contributes bounded goal identity, objective, status, usage, and
  remaining-budget evidence as a versioned no-cache fragment
- **AND** the context planner budgets and fingerprints that fragment normally

#### Scenario: Terminal event is replayed twice internally

- **GIVEN** the controller observes duplicate or replayed terminal evidence for
  one goal generation
- **WHEN** it evaluates automatic continuation
- **THEN** at most one new internal turn is accepted
- **AND** deduplication does not suppress a later distinct active generation

### Requirement: Goal ability scope is explicit

Smith SHALL install a stable dormant goal tool ability only for persistent root
sessions so direct natural-language goal requests do not depend on heuristic
intent classification. Tool instructions MUST restrict creation to explicit
user or higher-priority intent, and goal context/state/events SHALL remain
absent until a goal exists. Child, review, and explicitly ephemeral sessions
MUST NOT advertise or inherit root goal control.

#### Scenario: Ordinary persistent root turn

- **GIVEN** no goal exists and the user expresses no goal intent
- **WHEN** Smith freezes the turn's ability epoch
- **THEN** the fixed eligible-session goal tool schemas remain stable
- **AND** no goal context fragment, state, event, or automatic turn appears

#### Scenario: Existing goal resumes

- **GIVEN** a persistent root session restores a current goal
- **WHEN** the next ability epoch is derived
- **THEN** required goal tools and context are available without a new creation
  phrase
- **AND** the exact component revision appears in composition evidence

#### Scenario: Child session is constructed

- **GIVEN** a root session with an active goal delegates a child
- **WHEN** Smith derives the child's scoped ability view and snapshot
- **THEN** the child receives no root goal state or goal tools
- **AND** child work remains attributable through the existing delegation path

### Requirement: Non-goal runtime behavior remains unchanged

Adding goal support SHALL preserve ordinary user-turn submission, canonical
history, provider/tool execution, interruption, persistence, replay, and
headless terminal semantics when no goal exists or goal capability is inactive.

#### Scenario: Existing no-goal fixture runs

- **GIVEN** a pre-change deterministic TUI/headless fixture with no goal intent
- **WHEN** it runs through the goal-capable build
- **THEN** its canonical messages, committed tool results, lifecycle outcomes,
  and usage semantics are equivalent
- **AND** no goal event or automatic turn appears
