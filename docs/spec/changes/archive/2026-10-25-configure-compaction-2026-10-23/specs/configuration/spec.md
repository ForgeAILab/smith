## ADDED Requirements

### Requirement: Compaction defaults and model-specific inactivity

Smith SHALL resolve optional `context.compaction.model` and
`context.compaction.instructions` with ordinary configuration provenance.
The model target SHALL accept a configured profile or `provider/model` and
default to the conversation route. Model-specific
`models."provider/model".cache.inactivity_limit_ms` SHALL use the existing
inactivity bounds. Layer precedence SHALL apply, with model specificity
breaking same-layer ties. These settings SHALL NOT claim to control provider
cache retention.

#### Scenario: A model overrides the shared cutoff

- **WHEN** the selected model has an inactivity value in the same layer as the shared default
- **THEN** Smith uses that model's value and exposes its actual source

#### Scenario: A higher layer overrides a model default

- **WHEN** a higher layer supplies a shared inactivity value
- **THEN** that higher-layer value wins over a lower-layer model-specific value

#### Scenario: Invalid settings are rejected before publication

- **WHEN** a requested model route, instruction, or duration is invalid
- **THEN** Smith rejects the candidate before provider execution or a configuration write
