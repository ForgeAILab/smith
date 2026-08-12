## ADDED Requirements

### Requirement: Default-unlimited per-turn tool-loop ceiling

Smith SHALL apply no tool-step ceiling to a turn by default. The built-in
default for `limits.max_tool_steps` MUST be `0`, and a value of `0` MUST leave
the tool loop without a step ceiling. A positive value MUST remain the enforced
ceiling. The setting MUST keep typed layered resolution and source provenance
like every other limit.

#### Scenario: Default run is unlimited

- **GIVEN** no layer configures `limits.max_tool_steps`
- **WHEN** Smith resolves the run configuration
- **THEN** the runtime policy carries no tool-step ceiling for the turn
- **AND** the turn ends only when the model stops, a configured wall-clock
  deadline trips, or the run is interrupted

#### Scenario: Positive configured value remains enforced

- **GIVEN** user configuration sets `limits.max_tool_steps = 32`
- **WHEN** Smith resolves the run configuration
- **THEN** the turn is bounded by a thirty-two-step tool loop
- **AND** the ceiling's source is retained for `smith config explain`

#### Scenario: Explicit zero removes the ceiling

- **GIVEN** user configuration sets `limits.max_tool_steps = 0`
- **WHEN** Smith resolves the run configuration
- **THEN** the runtime policy carries no tool-step ceiling for the turn
- **AND** `smith config explain` reports the value and its source
