## ADDED Requirements

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
