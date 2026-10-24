## MODIFIED Requirements

### Requirement: One Smith factory composes durable child recovery

Smith SHALL compose Agent Runtime's child catalog, child session/checkpoint
stores, lifecycle leases, policy revisions, and recovery operation through the
same `smith-runtime::factory` path used by root, TUI, headless, test, and
embedded sessions. It MUST NOT add a Smith-local child loop, reconstruct exact
state from journals, or bypass runtime authorization/checkpoint semantics.

The same factory SHALL compose eligible Layer-1 coordinators through depth-aware
child-builder and post-start hooks, using Smith max_depth=2 and the shared process
capacity pool. It MUST wire and recover nested delegation before the child's
first initial/follow-up/resume turn. Layer-2 and delegation-disabled routes SHALL
remain leaf compositions. Failed post-start wiring MUST settle partial resources
without executing the task or leaking a capacity lease.

#### Scenario: Rebuild a recovered read-only child

- **GIVEN** a durable read-only child is idle after parent restart
- **WHEN** a follow-up requires its runtime to be lazily reconstructed
- **THEN** the one factory composes the original compatible provider/model,
  protected stores, workspace, and narrowed read-only ability view
- **AND** delegation-management ability remains narrowed by its validated depth
  and effective profile policy

#### Scenario: Current policy would widen authority

- **GIVEN** a recovered child record declares a narrower tool/workspace policy
  than current defaults
- **WHEN** Smith rebuilds it
- **THEN** the factory retains the recorded upper bound or fails closed
- **AND** defaults do not silently grant edit, shell, network, interaction, or
  child-management authority

#### Scenario: Lead resumes before nested work is restored

- **GIVEN** a retained lead owns a compatible worker catalog
- **WHEN** its runtime is lazily rebound
- **THEN** the ordinary post-start hook wires and recovers that catalog before any resumed lead turn
- **AND** inspection alone still does not bind or execute it

### Requirement: Child completion uses conditional internal turns

Smith SHALL call Agent Runtime's `try_admit_child_completion_if_idle` with a
`ChildCompletionAdmissionRequest` and expected `ChildOutcomeCursor`. Runtime
may accept the bounded provenance-bearing child-completion turn only if the
parent remains idle at one serialized decision boundary and returns
`ChildCompletionAdmission::{Accepted, Busy, Stale, Shutdown, Conflict}`. An
accepted turn SHALL be attributed as `delegation.child-completion`, consume
every ready protected terminal outcome in deterministic order, and use the
parent runtime composition applicable at admission. When that composition
runs turns on an external agent backend, the accepted turn SHALL run on that
backend as "Internal turns on a harness session run on the harness"
requires.

The internal input MUST NOT append a fabricated user-role canonical message,
queue ahead of real user work, bypass the provider, context, tool, approval,
workspace, checkpoint, cancellation, retry, and global-limit contracts an
ordinary turn on that session observes, or survive as unattributed
background work. Admission identity and outcome consumption SHALL be crash-
and replay-safe so one committed continuation is not repeated.

Agent Runtime SHALL own canonical admission and cache operation/evidence event
payloads used by this turn. Smith may consume and project those redaction-safe
events into product surfaces, but MUST NOT define local RuntimeEvent variants
or a second normalized provider event vocabulary.

The contract SHALL apply to each eligible parent independently, including a
parked Layer-1 lead. Ready successful, NeedsInput, failed, and stopped outcomes
SHALL retain their immediate-parent ownership and protected deterministic
identity. Forward-only grandchild events MUST NOT become Root delivery input or
acknowledge the lead's ledger. A continuation SHALL remain part of its parent's
active logical task episode until quiescence or typed termination.

#### Scenario: Idle parked session accepts child completion

- **GIVEN** a root session on a model provider is idle in
  `parked-awaiting-child`
- **AND** one or more protected terminal outcomes are ready
- **WHEN** its controller conditionally requests continuation
- **THEN** Agent Runtime accepts one attributed internal turn
- **AND** normal context planning, provider execution, usage, and cache
  observation apply

#### Scenario: Idle parked harness session accepts child completion

- **GIVEN** a root session on a harness is idle in `parked-awaiting-child`
- **AND** one or more protected terminal outcomes are ready
- **WHEN** its controller conditionally requests continuation
- **THEN** Agent Runtime accepts one attributed internal turn
- **AND** the turn runs on the harness with no provider request

#### Scenario: Parent became busy before admission

- **GIVEN** terminal outcomes were injected while the parent appeared idle
- **WHEN** another turn wins before conditional admission commits
- **THEN** the child-completion attempt returns
  `ChildCompletionAdmission::Busy`
- **AND** the outcomes remain protected for the next safe boundary

#### Scenario: Continuation history is inspected

- **GIVEN** one or more child-completion internal turns committed
- **WHEN** canonical history, lifecycle events, or checkpoints are inspected
- **THEN** no fabricated user continuation message is present
- **AND** attributed evidence identifies each internal turn and consumed
  outcome set

#### Scenario: Replay sees a committed child continuation

- **GIVEN** a child-completion turn and its outcome-consumption watermark
  committed before a crash
- **WHEN** Smith resumes and replays the journal
- **THEN** presentation can reconstruct the attributed lifecycle
- **AND** replay cannot schedule the committed provider turn again

#### Scenario: Failed worker wakes an idle lead

- **GIVEN** a depth-1 lead is parked and its depth-2 worker has a durable failure outcome
- **WHEN** conditional admission wins at the lead idle boundary
- **THEN** one attributed internal turn delivers the redacted failure to the lead
- **AND** Root does not receive a successful interim lead result

### Requirement: Parked state performs no provider work by itself

Smith SHALL perform no provider work solely because a session is parked.
Entering, remaining in, listing, inspecting, persisting, resuming, or leaving
`parked-awaiting-child` SHALL perform no provider request by itself. Provider
work may begin only for admitted real-user, goal, or child-completion turns or
for separately authorized cache maintenance under the prompt-cache policy.
Parking MUST NOT be represented by a synthetic canonical assistant message.

These rules SHALL hold for Root and Layer-1 parked states. Jev requests SHALL NOT
be triggered by entering or observing parking, lifecycle forwarding, internal
child-completion/wait-timeout turns, or cache maintenance. Required descendants
MUST remain tracked without polling the provider or losing the task episode.

#### Scenario: Parent parks while child continues

- **GIVEN** a parent turn completes and a child remains running
- **WHEN** Smith records the parked lifecycle state
- **THEN** provider invocation count is unchanged
- **AND** the child coordinator remains able to deliver terminal evidence

#### Scenario: Resumed session renders parked evidence

- **GIVEN** a persistent session records that its prior process parked while a
  child was active
- **WHEN** a later process reconciles and renders the session
- **THEN** it performs no provider request merely to show that evidence
- **AND** existing child durability policy determines interrupted or terminal
  state

#### Scenario: Cache maintenance is separately authorized

- **GIVEN** a parked parent is eligible for one adaptive maintenance request
- **WHEN** Smith dispatches it under the cache policy
- **THEN** the attempt carries a synthetic cache purpose rather than a parked
  continuation purpose
- **AND** parking alone is not reported as the provider-work cause

## ADDED Requirements

### Requirement: Lossless nested task episode collection

Agent Runtime SHALL follow a delegated task across its initial and admitted
internal continuation turns using retained sequence/identity evidence rather
than only the bounded event stream or latest handle. Pending-work checks MUST be
observational and terminal commit MUST verify quiescence at a serialized boundary.
A lead's parking turn MUST NOT publish the task's Completed outcome while nested
work, persistence, ready outcomes, or continuation admission remains pending.

#### Scenario: Worker completion races lead turn completion

- **GIVEN** a lead finishes its parking turn as the worker records an outcome
- **WHEN** the collector subscribes/checks and continuation admission races
- **THEN** the outcome remains pending or the continuation is followed losslessly
- **AND** exactly one final integrated lead result reaches Root

#### Scenario: A terminal failure has no successful result

- **GIVEN** all nested workers stop or fail
- **WHEN** the collector evaluates the settled subtree
- **THEN** it delivers typed terminal evidence to the lead or terminates according to cancellation policy
- **AND** it does not reuse an old Completed parking turn as final success

### Requirement: SystemOne advisory decision contract

Smith SHALL use POST /v1/systemone for inverse is_actionable noul, five-level
complexity score, needs_subagents noul, and target_profile choice with explicit
eligible-role descriptions. P(clarify) MUST equal 1-P(is_actionable), and an
actionability value at or below the configured threshold SHALL suggest
clarification. Expected complexity MUST equal zero-based API score+1, and exact
complexity MUST equal
argmax probability index+1. Choice confidence MUST be interpreted as
(k*pmax-1)/(k-1), clipped to [0,1], with the single-option case defined as 1.
Score confidence and choice concentration MUST NOT be described as P(correct).

Required response types, option sets, finite probabilities, normalization,
score/legend/order, and actual model identity SHALL be validated. A request model
string MUST NOT be assumed enforced. Fake/untrained local engines and incompatible
chat-completion responses MUST NOT be used for production triage.

#### Scenario: Expected score differs from exact class

- **GIVEN** an answer has raw score 3.257994 and its highest probability at index 4
- **WHEN** Smith maps the five-level decision
- **THEN** the expected complexity is 4.257994 and the exact class is 5
- **AND** rounding the raw score is not used as the class

#### Scenario: Choice confidence differs from winning probability

- **GIVEN** five profile options and pmax=0.86
- **WHEN** Smith evaluates normalized profile confidence
- **THEN** confidence is 0.825
- **AND** the configured threshold is not compared against an incorrectly substituted raw pmax

### Requirement: Shared pre-turn advisory routing is fail-open and exact-once

All stock Smith user-submission surfaces SHALL use one host admission path for
optional Jev triage before root provider dispatch. A successful route MAY use
provider-free host control admission to start one authorized Layer-1 episode,
but MUST record the real user message exactly once and preserve ordinary
agent.delegate authorization, approval, workspace, cancellation, and limits.
Internal/goal/timer/child turns, local commands, and questionnaire replies as
standalone inputs MUST NOT recursively trigger triage.

Disabled routing, timeout, unavailable service, invalid response/identity,
ineligible route, or insufficient routing confidence SHALL fall back to ordinary
configured root handling with the original input and attachments intact. A late
response MUST NOT produce a second dispatch. Cancellation/shutdown MUST terminate
admission rather than starting fallback execution. A denied authorization MUST
remain denied; advisory fail-open is not security fail-open.

#### Scenario: Sidecar times out

- **GIVEN** an enabled local endpoint exceeds the configured total timeout
- **WHEN** the same uncancelled user admission is still current
- **THEN** ordinary root handling receives the original input once
- **AND** a late endpoint result cannot start another child

#### Scenario: User cancels during triage

- **GIVEN** a triage request is in flight
- **WHEN** the user cancels or the host shuts down
- **THEN** the request and admission are cancelled
- **AND** neither a child nor ordinary fallback provider work starts afterward

#### Scenario: Clarifier hands off without creating a third layer

- **GIVEN** an authorized Layer-1 leaf clarifier obtained exact task answers
- **WHEN** a bounded host handoff selects an eligible lead
- **THEN** Root starts a separate authorized depth-1 lead episode
- **AND** that lead may still create depth-2 workers under normal policy

### Requirement: Nested lifecycle ownership is bounded and cycle-free

Smith SHALL retain nested coordinator/lifecycle ownership until task, recovery,
interaction, and shutdown obligations settle. Session back-references and factory
registry references MUST avoid strong ownership cycles. The same process capacity
pool and host authority SHALL apply to both layers; failure during wiring or
shutdown MUST NOT leak live tasks, brokers, timers, stores, or leases.

#### Scenario: Host stops while a lead is parked

- **GIVEN** a lead owns a running worker, wait timer, and pending interaction
- **WHEN** bounded host shutdown begins
- **THEN** admission is frozen and descendants/interactions are cancelled in ownership order
- **AND** each shared capacity lease and lifecycle task is released once
