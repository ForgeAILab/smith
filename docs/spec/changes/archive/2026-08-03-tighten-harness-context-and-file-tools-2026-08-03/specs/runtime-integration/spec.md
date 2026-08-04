## MODIFIED Requirements

### Requirement: Semantic summarization triggers on context pressure

Semantic summarization SHALL be triggered by measured input-budget pressure
rather than by a count of completed turns. The trigger MUST compare usage
accumulated after the stable cached prefix against a configured fraction of the
resolved input budget. A minimum completed-turn count MUST remain as an
eligibility floor so a short session is never summarized, but reaching that
count alone MUST NOT trigger summarization.

#### Scenario: A long but small session is not summarized
- **GIVEN** a session with ten completed turns whose post-prefix usage is well
  under the configured fraction of the input budget
- **WHEN** the next turn is planned
- **THEN** no semantic summary is produced
- **AND** no summary model call is made

#### Scenario: A short session with a large tool result is summarized
- **GIVEN** a session past the minimum turn floor whose post-prefix usage
  crosses the configured fraction after one large tool result
- **WHEN** the next turn is planned
- **THEN** a semantic summary is produced

#### Scenario: The turn floor prevents summarizing a young session
- **GIVEN** a session below the minimum completed-turn floor
- **WHEN** post-prefix usage crosses the configured fraction
- **THEN** no semantic summary is produced

#### Scenario: A large stable prefix does not pull the trigger forward
- **GIVEN** two sessions with identical conversation bodies
- **AND** one activates substantially more skills and project instructions
  than the other
- **WHEN** both are planned
- **THEN** neither triggers summarization earlier than the other on account of
  its prefix size

## ADDED Requirements

### Requirement: One appended budget notice before the compaction boundary

When the remaining input budget crosses a configured threshold, Smith SHALL
append exactly one bounded notice to the conversation for the current
compaction window, informing the model that the context boundary is near. The
notice MUST be appended after existing content so it never rewrites history,
and MUST NOT be repeated until a new compaction window begins.

#### Scenario: The notice is delivered once per window
- **GIVEN** a session whose remaining input budget has crossed the notice
  threshold
- **WHEN** two further turns are planned without compaction occurring
- **THEN** exactly one notice appears in the conversation

#### Scenario: The notice does not rewrite history
- **GIVEN** a session that receives the budget notice
- **WHEN** the provider request is assembled
- **THEN** every message preceding the notice is byte-identical to the previous
  request's corresponding message

#### Scenario: A new window re-arms the notice
- **GIVEN** a session that received the notice and then compacted
- **WHEN** the remaining budget crosses the threshold again
- **THEN** one further notice is delivered

### Requirement: Session usage is reported and recorded

On exit, Smith SHALL report the session's token totals per counter kind with
their provenance, never presenting a derived or estimated count as
provider-reported. Smith SHALL also append one bounded usage record per session
to a durable log containing the session identity, model, turn count, per-counter
totals with confidence, the number of compaction windows, and the number of
budget notices and semantic summaries produced.

#### Scenario: Exit reports totals with provenance
- **GIVEN** a session whose input counts were provider-reported and whose
  reasoning counts were estimated
- **WHEN** the user exits the TUI
- **THEN** the summary marks the estimated counts as estimated
- **AND** it does not mark them as reported

#### Scenario: The usage record carries no conversation content
- **GIVEN** a completed session
- **WHEN** its usage record is appended
- **THEN** the record contains counts, identities, and trigger tallies
- **AND** it contains no prompt text, tool arguments, or file contents

#### Scenario: Compaction behavior is analyzable across sessions
- **GIVEN** several completed sessions
- **WHEN** their usage records are read
- **THEN** each records how many compaction windows and semantic summaries
  occurred
- **AND** a threshold change can be evaluated against them
