## ADDED Requirements

### Requirement: Cache lifecycle state is explainable across surfaces

Smith SHALL project the same bounded cache lifecycle facts through interactive
status, session/exit summaries, final JSON, and streaming JSON events. Human
and machine surfaces SHALL keep these values
separate when available:

- structurally preserved prefix tokens;
- provider-reported cache-read and cache-write tokens;
- provider cache status and guarantee timestamp;
- exact cache identity/revision without private prompt content;
- requested and effective maintenance mode;
- maintenance call budget and calls used;
- scheduled, suppressed, completed, or suspended disposition and reason; and
- separately attributed synthetic usage and cost.

An unknown value MUST remain unknown rather than zero, warm, expired, or
guaranteed. Existing cache-miss visibility and `CH` projections SHALL continue
to use the canonical attempt evidence from
`add-prompt-cache-miss-visibility-2026-08-08`.

#### Scenario: Structurally reusable prefix has no provider evidence

- **GIVEN** context planning preserves a stable prefix
- **AND** the provider reports no cache observation
- **WHEN** status and final JSON are rendered
- **THEN** both expose the structural count separately
- **AND** provider cache status remains unknown
- **AND** neither surface calls it a verified hit

#### Scenario: Scheduled maintenance is suppressed

- **GIVEN** a keepalive was scheduled and later suppressed by real activity
- **WHEN** interactive and machine status update
- **THEN** both expose the bounded suppression reason and unchanged call usage
- **AND** neither reports a provider attempt or cost

#### Scenario: Active cache-miss projection remains composed

- **GIVEN** a provider explicitly reports zero after an expected reusable plan
- **WHEN** the new lease projection and existing cache-miss projection reduce
  the same canonical attempt
- **THEN** every surface reports one consistent miss and suspension state
- **AND** no second miss count or re-billed-token value is derived

### Requirement: Parked parent and automatic continuation are visible

Interactive and headless lifecycle output SHALL distinguish an idle ordinary
session, `parked-awaiting-child`, an admitted
`delegation.child-completion` turn, and an adaptive cache-maintenance attempt.
The projection MUST NOT imply that a provider stream remains open while parked
or that cache maintenance is child execution.

#### Scenario: Parent waits without provider work

- **GIVEN** a parent is parked with one running child
- **WHEN** status or machine output is inspected
- **THEN** it identifies the parked state and pending child
- **AND** reports no active parent provider turn unless one actually exists

#### Scenario: Child completion wakes the parent

- **GIVEN** a child-completion internal turn is admitted
- **WHEN** lifecycle and usage output are inspected
- **THEN** the turn is attributed to `delegation.child-completion`
- **AND** any cache observation or provider usage belongs to that real
  continuation rather than the prior parked interval

#### Scenario: User input wins the race

- **GIVEN** user input wins admission over a ready child outcome
- **WHEN** clients render the boundary
- **THEN** they show one active user turn
- **AND** do not render a phantom concurrent child-completion turn

### Requirement: Resume-capsule diagnostics reveal no sensitive content

Smith SHALL keep resume-capsule diagnostics bounded and redaction-safe. Status
and machine output MAY expose schema revision, freshness, summary
purpose/model/revision, source coverage, and last successful persistence
boundary. They MUST NOT expose raw canonical history, private prompt bodies,
credentials, protected interaction content, provider cache contents, or
unbounded summary text.

#### Scenario: Handoff summary is persisted

- **GIVEN** a same-model handoff checkpoint updates the capsule
- **WHEN** final JSON reports the capsule projection
- **THEN** it exposes purpose, model/revision, timestamp, coverage, and outcome
- **AND** omits the summary body and stable-prefix content

#### Scenario: Exact state conflicts with summary

- **GIVEN** recovery detects a summary inconsistency
- **WHEN** Smith presents the diagnostic
- **THEN** it reports bounded field/category and authoritative-source metadata
- **AND** does not copy conflicting private text into logs or status

### Requirement: Synthetic cache traffic never appears as conversation

Keepalive and handoff-checkpoint request/response content SHALL be absent from
canonical transcripts, replayed conversation, copied answers, and model history.
Clients MAY render bounded local lifecycle diagnostics and separately
attributed usage, but those blocks MUST remain noncanonical.

#### Scenario: User continues after a handoff checkpoint

- **GIVEN** a handoff checkpoint completed during a parked interval
- **WHEN** the user sends the next real prompt
- **THEN** provider context contains no synthetic checkpoint instruction or
  response as a canonical turn
- **AND** the resume capsule may contribute only through its reviewed bounded
  continuation projection

#### Scenario: Journal replay reconstructs maintenance

- **GIVEN** canonical redaction-safe maintenance lifecycle events were journaled
- **WHEN** the TUI replays them
- **THEN** it can reconstruct status and usage diagnostics
- **AND** it cannot fabricate ping, pong, or summary text into conversation
