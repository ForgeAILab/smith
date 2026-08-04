## ADDED Requirements

### Requirement: Intent-aligned coding capability bootstrap

Smith's initial capability retrieval SHALL prefer the least-authority exact
tools implied by coding intent. Inspection activates bounded list/search/read,
modification adds exact edit, command/build/test intent adds broad shell,
multi-step intent adds todos, and delegation/review intent adds the root agent
ability; protected search remains available for misses.

#### Scenario: Explicit production code fix
- **GIVEN** the user asks Smith to inspect, edit, test, and delegate review
- **WHEN** initial activation is resolved
- **THEN** the view includes read inspection, exact edit, required validation,
  todos, and root delegation capabilities
- **AND** shell is present only because command/test intent was explicit

#### Scenario: Read-only repository question
- **GIVEN** the user asks a question answerable by inspection
- **WHEN** initial activation is resolved
- **THEN** no edit or broad shell mutation capability is advertised
- **AND** the answer may still use bounded list/search/read

### Requirement: Terminal todo coherence

Every turn terminal boundary SHALL produce a terminal todo snapshot. Smith
MUST preserve explicitly completed/cancelled items and MUST convert remaining
pending or in-progress items to cancelled with a stable unfinished reason
rather than guessing completion.

#### Scenario: Successful answer leaves report item active
- **GIVEN** the model emits a successful final answer while one todo remains
  in progress
- **WHEN** Smith commits the turn result
- **THEN** the item becomes cancelled as `turn_ended_unfinished`
- **AND** JSON, TUI, checkpoint, and replay report zero non-terminal items

#### Scenario: Interrupted turn has pending work
- **GIVEN** a plan contains active and pending items
- **WHEN** the user interrupts the turn
- **THEN** all unfinished items become terminally cancelled with interruption
  provenance
- **AND** a later turn may create a new plan normally

### Requirement: Limit-safe visible output

Retry, output-limit, and time-limit terminals MUST NOT promote uncommitted
reasoning or speculative text into the assistant transcript or final output.
They SHALL return a concise structured reason, committed usage/attempt evidence,
and terminal plan state.

#### Scenario: GLM-5.2 reaches request output budget before editing
- **GIVEN** an attempt emitted reasoning but no committed assistant response
- **WHEN** it reaches the request output limit
- **THEN** Smith returns `limit_reached` without exposing the reasoning as the
  final assistant answer
- **AND** reports attempt, usage, plan, and remediation evidence structurally

### Requirement: Model-specific request output budget

Smith SHALL resolve a product request-output budget separately from the
provider model's immutable maximum and retain source provenance. Cataloged
Z.AI Coding Plan `glm-5.2` SHALL default to 32,768 request tokens, never exceed
its declared model limit, and remain explicitly overridable.

#### Scenario: Resolve cataloged GLM-5.2
- **GIVEN** no higher-precedence request-output override exists
- **WHEN** Smith resolves the Z.AI Coding Plan GLM-5.2 profile
- **THEN** the request budget is 32,768 and the immutable maximum remains
  131,072
- **AND** `smith config explain max_output_tokens` identifies catalog/default
  provenance

#### Scenario: Explicit lower budget
- **GIVEN** owner-controlled configuration selects 8,192 request tokens
- **WHEN** Smith resolves the same model
- **THEN** the explicit value wins and is shown in provenance
- **AND** a later limit result remains concise and structurally honest
