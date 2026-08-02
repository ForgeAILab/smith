## ADDED Requirements

### Requirement: Goal state uses canonical session durability

Smith SHALL persist goal state through the existing versioned extension-state,
checkpoint, completed-turn snapshot, atomic write, and session lifecycle lease
contracts. It MUST NOT add a separate goal database, sidecar, project file, or
best-effort event reconstruction as the source of truth.

#### Scenario: Goal mutation completes before process loss

- **GIVEN** a goal tool or user control committed a new goal generation
- **WHEN** the process exits after the canonical persistence boundary
- **THEN** resume restores that exact state, identity, status, usage provenance,
  and timestamps
- **AND** does not reconstruct the objective from redacted journal prose

#### Scenario: Mutation is discarded before commit

- **GIVEN** a goal mutation began but its tool/turn transition never committed
- **WHEN** Smith recovers the session
- **THEN** the durable prior goal remains authoritative
- **AND** no goal event or automatic continuation is fabricated for the
  discarded mutation

### Requirement: Resume restores before continuation

On resume, Smith SHALL decode and validate the persisted goal before exposing
controls or scheduling work. It SHALL publish one current projection to the
attached host, restore accounting baselines without charging downtime, and only
then admit an active goal continuation.

#### Scenario: Active goal resumes in a later process

- **GIVEN** the persisted goal status is `active`
- **WHEN** a compatible interactive or headless host resumes the session
- **THEN** the host first receives the restored goal projection
- **AND** the controller then attempts one conditional internal continuation

#### Scenario: Stopped goal resumes

- **GIVEN** the persisted goal is paused, blocked, usage-limited,
  budget-limited, or complete
- **WHEN** Smith resumes the session
- **THEN** it restores and displays that status without starting provider work
- **AND** waits for an explicit valid user transition

#### Scenario: Goal schema is incompatible

- **GIVEN** persisted goal state has an unknown revision or malformed value
- **WHEN** Smith attempts resume
- **THEN** resume fails closed with a bounded compatibility diagnostic
- **AND** it does not clear, reinterpret, or automatically continue the goal

### Requirement: Process shutdown ends automatic work

Goal continuation SHALL be scoped to the current hosted Smith process. Shutdown
MUST cancel and drain serving goal turns through existing bounds, persist the
latest compatible state, and release the session lease; no daemon or detached
task may continue afterward.

#### Scenario: User exits with an active goal

- **GIVEN** an active goal has unfinished work
- **WHEN** the user confirms Smith shutdown
- **THEN** Smith cancels/drains current work and persists the latest state
- **AND** no provider or tool work continues after process shutdown

#### Scenario: Active goal is later resumed

- **GIVEN** a prior process shut down with an active persisted goal
- **WHEN** no later Smith process resumes that session
- **THEN** the goal remains dormant regardless of elapsed wall-clock time
- **AND** incurs no automatic usage

### Requirement: Goal continuation is crash- and replay-safe

Smith SHALL bind automatic continuation to durable goal identity/generation and
session turn identity so recovery cannot repeat a committed provider or tool
side effect. Journal replay is presentation-only and MUST NOT itself schedule
work.

#### Scenario: Crash follows completed continuation

- **GIVEN** a continuation turn and its goal state committed before a crash
- **WHEN** Smith resumes from compatible snapshot/checkpoint watermarks
- **THEN** it does not repeat that committed turn or its tools
- **AND** may start only the next continuation if the restored goal remains
  active

#### Scenario: TUI replays the journal

- **GIVEN** goal and turn events are replayed to reconstruct presentation
- **WHEN** the reducer observes an old active terminal boundary
- **THEN** it updates display state only
- **AND** cannot invoke the goal controller or provider
