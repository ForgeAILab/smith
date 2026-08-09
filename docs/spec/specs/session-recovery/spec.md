# session-recovery Specification

## Purpose
TBD - created by archiving change integrate-stable-session-harness. Update Purpose after archive.
## Requirements
### Requirement: Completed turns persist immediately

Smith SHALL persist a compatible redacted session snapshot after every
completed turn when persistence is enabled. The snapshot MUST retain canonical
history, usage, identity, all ordered manifests, and the latest durable
checkpoint watermark.

#### Scenario: Process exits before explicit shutdown
- **GIVEN** a turn completed and its snapshot save succeeded
- **WHEN** the Smith process exits unexpectedly
- **THEN** resuming retains that turn and all earlier manifests
- **AND** does not depend on orderly session shutdown

### Requirement: Exact mid-turn state uses protected checkpoints

Smith SHALL implement Agent Runtime's checkpoint contract with authenticated
encryption, integrity/version checks, atomic or transactional commits, and a
user-scoped protection key. It MUST NOT silently store exact pending actions,
answers, or tool state as plaintext.

#### Scenario: Protection key is unavailable
- **GIVEN** Smith cannot initialize protected checkpoint storage
- **WHEN** a persistent session starts
- **THEN** Smith reports that mid-turn crash recovery is unavailable
- **AND** does not claim pending approvals or questions will survive restart

### Requirement: Journal and checkpoint have explicit watermarks

The redacted event journal and protected checkpoint SHALL record compatible
watermarks so startup can distinguish durable execution state from later
presentation-only events. Redacted events MUST NOT be expanded into guessed
raw arguments or answers.

#### Scenario: Journal is ahead of checkpoint
- **GIVEN** complete journal records exist after the latest checkpoint
- **WHEN** Smith resumes
- **THEN** it may replay those records for observability according to policy
- **AND** does not repeat provider or tool side effects absent an idempotent
  checkpointed transition

### Requirement: Pending work resumes or reconciles safely

Smith SHALL restore checkpointed approvals and questionnaires exactly once,
resume partially completed tool batches without repeating committed results,
and mark prior ephemeral children or monitors interrupted without restarting
them.

#### Scenario: Restart occurs with a question open
- **GIVEN** one questionnaire request was durably checkpointed
- **WHEN** the interactive TUI resumes the session
- **THEN** it presents the same request identity and accepts one response
- **AND** resumes the same turn without creating a synthetic new user turn

### Requirement: No-prompt protected checkpoint initialization

When an explicit inline or environment checkpoint key resolves, Smith SHALL
initialize the existing authenticated-encrypted checkpoint store without
opening or querying an operating-system credential service. The key source
changes protection only; checkpoint schema, exact-state boundaries, journal
watermarks, atomicity, and no-plaintext-fallback rules remain unchanged.

#### Scenario: Resume with an inline key
- **GIVEN** a session checkpoint was encrypted under the configured inline key
- **WHEN** Smith restarts and resumes it
- **THEN** it decrypts and validates the exact compatible state without an OS
  credential prompt
- **AND** does not repeat committed provider or tool work

#### Scenario: Configured key is wrong
- **GIVEN** a checkpoint envelope cannot authenticate under the resolved key
- **WHEN** Smith loads it
- **THEN** recovery fails closed with a redacted key-mismatch/integrity result
- **AND** does not delete, replace, or treat the checkpoint as empty

### Requirement: Atomic checkpoint-key rotation

Changing an established checkpoint key SHALL either re-encrypt every selected
compatible checkpoint atomically under an exclusive lease or refuse before
modification. Smith MUST retain a recoverable prior state until the complete
rotation commits.

#### Scenario: Rotation succeeds
- **GIVEN** every selected checkpoint validates under the old key
- **WHEN** the user confirms rotation to a new source
- **THEN** Smith publishes all re-encrypted envelopes and config as one
  recoverable transaction
- **AND** zeroizes old and new working key material after commit

#### Scenario: One checkpoint is corrupt
- **GIVEN** any selected envelope fails validation
- **WHEN** rotation preflight runs
- **THEN** no checkpoint or config value changes
- **AND** Smith reports the affected session without secret material

### Requirement: User-state-only agent metadata

Smith SHALL keep agent modes, child navigation, live-work reconstruction,
timeline records, and prepared composer recovery in process memory, canonical journals,
protected checkpoints, or owner-only user state. Smith MUST NOT create
continuation or agent-control metadata in the project checkout.

#### Scenario: Complete an agent-heavy coding turn
- **GIVEN** Smith uses todos, file references, shell, and a child reviewer
- **WHEN** the turn and session complete
- **THEN** `git status` shows only task-requested project changes
- **AND** no `.smith`, `.omo`, timeline, session, or continuation metadata is
  present in the checkout

### Requirement: Parent resume restores the child coordinator without execution

Smith SHALL load and validate a resumed parent's durable child catalog,
reconcile lifecycle leases and checkpoint watermarks, and wire those records
into the delegation coordinator before accepting child operations. Recovery
MUST be lazy and MUST NOT construct a provider request, invoke a tool, or
silently restart child work merely to list, inspect, or render the session.

#### Scenario: Parent resumes with two children

- **GIVEN** a saved parent owns one idle child and one child interrupted by
  process exit
- **WHEN** Smith resumes the parent and renders `/agent`
- **THEN** both stable child IDs and accurate states are available
- **AND** provider and tool invocation counts remain zero until an explicit
  operation is submitted

### Requirement: Child recovery is exact, protected, and no-prompt compatible

Smith SHALL store exact child turn state through the authenticated protected
checkpoint path and SHALL reconcile it exactly once with the durable child
record. When the configured checkpoint key comes from the existing inline or
environment source, child persistence and resume MUST NOT open or query an
operating-system credential service and MUST NOT fall back to plaintext.

#### Scenario: Resume under an environment checkpoint key

- **GIVEN** root and child checkpoints were protected by the configured
  environment key
- **WHEN** Smith restarts and explicitly resumes the child
- **THEN** it authenticates and continues the exact checkpoint without an OS
  credential prompt
- **AND** repeats no committed provider, approval, interaction, or tool effect

#### Scenario: Child checkpoint fails authentication

- **GIVEN** the child record references a checkpoint that does not authenticate
- **WHEN** Smith recovers the parent
- **THEN** the child is shown blocked/non-resumable with a redacted integrity
  reason
- **AND** Smith does not replace, delete, or execute it

### Requirement: Legacy ephemeral children are never fabricated as durable

Smith SHALL preserve presentation evidence for historical journal-only child
runs, but MUST label them legacy ephemeral when no protected child record and
session state exist. It MUST NOT reconstruct raw history from redacted events,
offer resume, or bind the old child ID to a new session.

#### Scenario: Resume an older Smith session

- **GIVEN** the parent journal contains a completed or unresolved child from a
  schema predating durable child records
- **WHEN** Smith resumes that parent
- **THEN** timeline inspection retains the bounded historical child evidence
- **AND** follow-up/resume reports that the legacy child is unavailable
- **AND** no provider request or replacement spawn occurs

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

### Requirement: Durable resume capsule

Persistent root sessions SHALL maintain a versioned resume capsule sufficient
for correct cold continuation. The capsule is a logical projection over the
existing canonical snapshot, versioned extension state, journal watermarks,
and authenticated protected checkpoint; it MUST NOT create a second database,
project-local sidecar, or competing source of truth.

Its redaction-safe and protected portions SHALL contain, when available:

```text
schema_version
session_id
parent_turn_id
created_at
model_profile_identity
agent_profile_identity
project_instruction_revision
active_goal and generation
todo or plan projection
child ids, tasks, states, and terminal outcomes
changed-file paths and bounded diff metadata
validation commands and observed exit statuses
artifacts and durable references
unresolved approvals or interactions
unresolved decisions and constraints
semantic summary
summary purpose, model, and revision
summary source coverage
retained recent canonical turns
```

Sensitive exact pending state SHALL remain protected by the existing
authenticated checkpoint contract. Redaction-safe output MUST NOT contain raw
credentials, unredacted private prompt bodies, exact protected interaction
content, or provider cache contents.

Live child execution and process-local wait state are not durable execution
authority. On restart, a running or otherwise uncommitted child SHALL be
reconciled to `interrupted_by_process_exit` and never auto-restarted; a
terminal child outcome committed before restart SHALL remain terminal and
deliverable once through its protected outcome watermark.

#### Scenario: Parent parks after spawning a child

- **GIVEN** a persistent parent spawns a child
- **WHEN** the parent enters `parked-awaiting-child`
- **THEN** Smith persists the child identity, task, state, and parent boundary
  through existing durability contracts
- **AND** a process restart can determine whether the outcome is terminal,
  pending, or interrupted without fabricating execution

#### Scenario: Persistence is intentionally disabled

- **GIVEN** a root session is explicitly non-persistent
- **WHEN** it parks or creates a handoff summary
- **THEN** Smith labels cold post-process continuation unavailable
- **AND** does not write a hidden capsule outside configured persistence

### Requirement: Exact structured state takes precedence

Smith SHALL treat exact structured state as authoritative over generated
semantic-summary text. Among committed compatible records, the highest
protected or canonical watermark SHALL win; protected exact state wins over a
redaction-safe projection at the same logical boundary. Canonical history,
committed goal or plan state, child state, tool and validation evidence, and
artifact metadata SHALL take precedence over summary text. Journal replay is
presentation-only and semantic summary prose MUST NOT override exact state or
schedule work. If a summary conflicts with exact state, Smith SHALL use the
exact state and retain a bounded redaction-safe inconsistency diagnostic.

#### Scenario: Summary claims tests passed incorrectly

- **GIVEN** semantic text says `cargo test` passed
- **AND** exact validation state records a non-zero exit
- **WHEN** Smith reconstructs continuation context
- **THEN** it presents the exact failure state
- **AND** does not treat the summary as authoritative

#### Scenario: Summary names a child as running after terminal commit

- **GIVEN** summary text predates a committed terminal child outcome
- **WHEN** the session resumes
- **THEN** the exact child record and watermark determine its state
- **AND** Smith does not restart or wait for the child because of stale prose

#### Scenario: Live child state is not resumed as live execution

- **GIVEN** a durable capsule records a child as running without a committed
  terminal outcome
- **WHEN** Smith cold-resumes after process exit
- **THEN** it records `interrupted_by_process_exit`
- **AND** it does not auto-restart or wait for the child

#### Scenario: Protected watermark outranks stale projection

- **GIVEN** a redaction-safe snapshot and journal replay disagree about child
  state
- **AND** the authenticated protected checkpoint has the highest compatible
  committed watermark
- **WHEN** Smith reconstructs the session
- **THEN** the protected exact child state wins
- **AND** replay remains presentation-only

### Requirement: Incremental capsule updates

Smith SHALL update the exact structured capsule projection at meaningful
canonical or protected commit boundaries, including completed parent turns,
committed mutating tool calls, validation completion, child spawn/terminal/
interruption/stop, goal or plan changes, handoff-checkpoint completion, and idle
compaction.

Semantic summarization MAY occur less frequently and SHALL operate over a
bounded delta plus the previous summary and exact state. Failed summary work
MUST NOT discard previously committed exact state or original canonical
history.

#### Scenario: Long session with repeated compaction

- **GIVEN** a session has an existing semantic summary
- **WHEN** another compaction is required
- **THEN** the summary route receives the previous summary, bounded new
  canonical turns, and exact structured state
- **AND** need not reread the complete original transcript unless explicit
  policy requires it

#### Scenario: Mutating tool commits before a summary update

- **GIVEN** a tool mutation commits and semantic summarization has not run
- **WHEN** the process exits after the exact persistence boundary
- **THEN** cold resume restores the mutation evidence
- **AND** summary staleness cannot erase or reverse the committed state

### Requirement: Same-model handoff and ordinary summary are distinct

The resume capsule SHALL record whether semantic text came from a same-provider
and same-model cache-assisted handoff checkpoint or from an independently
attributed semantic-summary route. The routes SHALL retain separate purpose,
provider, model, cache identity, revision, source coverage, usage, and outcome.

A handoff summary is noncanonical and nonauthoritative even when it reused a
warm parent prefix.

#### Scenario: Smaller model performs ordinary summary

- **GIVEN** a named smaller profile performs semantic summarization
- **WHEN** the summary is persisted
- **THEN** the capsule records that profile, model, purpose, and revision
- **AND** the parent cache lease remains unchanged

#### Scenario: Same-model checkpoint summary is retained after miss

- **GIVEN** a handoff checkpoint returns valid bounded summary text
- **AND** canonical cache evidence reports a miss
- **WHEN** Smith commits the capsule update
- **THEN** it may retain the text with the observed miss and source coverage
- **AND** the old lease remains suspended rather than warm by implication

### Requirement: Cold resume is always supported

A compatible persistent session SHALL resume correctly when provider cache
state is absent or unknown. On process resume, Smith SHALL restore canonical
history, exact protected state, child durability state, and the resume capsule;
restore prior structural cache-plan information only as a comparison baseline;
treat provider warmth as unknown unless an explicit cache API supplies current
evidence; send no prewarm request; and allow the next real continuation to
establish a cache naturally.

#### Scenario: Process restarts after cache eviction

- **GIVEN** a session was warm before process exit
- **AND** the provider evicted the cache
- **WHEN** Smith resumes the session
- **THEN** canonical continuation remains correct
- **AND** provider warmth begins unverified
- **AND** the next real turn may cold-fill the cache without an extra prewarm

#### Scenario: Explicit resource still exists

- **GIVEN** a resumed session has a compatible explicit cache-resource identity
- **AND** the provider API reports that exact resource still exists
- **WHEN** Smith restores the cache lease
- **THEN** it may record the provider's current evidence and expiry
- **AND** canonical session recovery remains independent of the resource

#### Scenario: Cold child-result continuation after restart

- **GIVEN** a compatible terminal child outcome is durable
- **AND** the parent provider cache is unknown after restart
- **WHEN** ordinary admission integrates that outcome
- **THEN** Smith uses canonical state and the resume capsule in one real
  continuation
- **AND** no cache-only request precedes it

#### Scenario: Terminal child commit survives restart

- **GIVEN** a child terminal outcome and its protected consumption watermark
  were committed before process exit
- **WHEN** Smith resumes the session
- **THEN** the child remains terminal
- **AND** Smith delivers the protected outcome at most once
- **AND** no child execution is restarted

### Requirement: Resume capsule tests

Smith SHALL provide persistence and recovery tests covering at least:

1. exact state surviving process resume;
2. summary conflicts deferring to exact evidence;
3. smaller-model summaries leaving the parent cache state unchanged;
4. same-model handoff requests using the parent cache identity;
5. handoff request and response remaining outside canonical history;
6. cold resume sending no prewarm request;
7. child state and terminal outcome surviving compatible persistence;
8. running children becoming interrupted without auto-restart;
9. protected-versus-canonical watermark precedence and replay safety; and
10. private prompt, interaction, cache-content, and credential material being
   absent from redaction-safe capsule output.

The fixtures MUST exercise version migration, journal/checkpoint watermarks,
authenticated protection, live-to-replay equivalence, and a completely cold
provider cache.

#### Scenario: Cold recovery matrix runs

- **GIVEN** compatible snapshots, protected checkpoints, journal tails, and
  scripted summary/cache outcomes
- **WHEN** the resume-capsule conformance suite reconstructs each case
- **THEN** exact committed state and listed security properties are preserved
- **AND** provider cache absence cannot prevent correct continuation
