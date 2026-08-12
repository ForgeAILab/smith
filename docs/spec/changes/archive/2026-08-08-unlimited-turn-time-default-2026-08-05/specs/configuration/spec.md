## ADDED Requirements

### Requirement: Default-unlimited per-turn wall-clock deadline

Smith SHALL apply no wall-clock deadline to a turn by default. The built-in
default for `limits.turn_time_limit_ms` MUST be `0`, and a value of `0` MUST
leave a turn without a wall-clock ceiling. A positive value MUST remain the
enforced deadline. The setting MUST keep typed layered resolution and source
provenance like every other limit.

#### Scenario: Default run is unlimited

- **GIVEN** no layer configures `limits.turn_time_limit_ms`
- **WHEN** Smith resolves the run configuration
- **THEN** the runtime policy carries no wall-clock deadline for the turn
- **AND** the turn ends only when the model stops, a tool-loop ceiling trips, or
  the run is interrupted

#### Scenario: Positive configured value remains enforced

- **GIVEN** user configuration sets `limits.turn_time_limit_ms = 120000`
- **WHEN** Smith resolves the run configuration
- **THEN** the turn is bounded by a 120000-millisecond wall-clock deadline
- **AND** the deadline's source is retained for `smith config explain`

#### Scenario: Explicit zero removes the ceiling

- **GIVEN** user configuration sets `limits.turn_time_limit_ms = 0`
- **WHEN** Smith resolves the run configuration
- **THEN** the runtime policy carries no wall-clock deadline for the turn
- **AND** `smith config explain` reports the value and its source
