## ADDED Requirements

### Requirement: Independently selected compaction route and instruction

Smith SHALL bind configured compaction targets through its normal credential,
model-limit, and provider preflight. Custom instructions SHALL participate in
summary adapter identity. Manual overrides SHALL remain attempt-local.
Changing a summary route or instruction SHALL preserve canonical originals
and provenance and SHALL NOT authorize fabricated state or discard prior
exact persistence when a new summary cannot be produced.

#### Scenario: A dedicated model produces a summary

- **WHEN** the configured summary model differs from the conversation model
- **THEN** only the summary request uses that route and its usage attribution
- **AND** subsequent conversation turns keep the conversation route

#### Scenario: A summary default changes during a session

- **WHEN** an idle session adopts a new summary model or instruction
- **THEN** exact canonical history remains recoverable and incompatible optional summary state cannot prevent startup

#### Scenario: An overridden attempt fails

- **WHEN** an attempt with custom instructions fails
- **THEN** originals and the previous committed summary remain available
- **AND** the override does not change later automatic summaries
