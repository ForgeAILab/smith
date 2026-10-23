## ADDED Requirements

### Requirement: Compaction instructions and persistent controls

Smith SHALL accept `/compact [instructions]` for an individual attempt and
`/compact model`, `/compact instructions`, and `/compact idle` for persistent
defaults. Setting commands SHALL run only at an idle boundary and SHALL
preserve comments and unrelated user configuration through validated atomic
publication. `default` SHALL remove the corresponding user-layer override.
The active idle session SHALL adopt the committed selection safely.

#### Scenario: A user customizes one summary

- **WHEN** a user runs `/compact preserve the test results and next steps`
- **THEN** those instructions apply only to that attempt and no config is written

#### Scenario: A user changes the default summary model

- **WHEN** a user runs `/compact model zai/glm-4.5-air`
- **THEN** Smith validates and persists the default and applies it to subsequent compaction
- **AND** canonical history and prior summary provenance remain recoverable

#### Scenario: A user changes a model's idle cutoff

- **WHEN** a user runs `/compact idle 15m`
- **THEN** Smith persists 900000 milliseconds for the active provider/model
- **AND** reports the effective value and its source
