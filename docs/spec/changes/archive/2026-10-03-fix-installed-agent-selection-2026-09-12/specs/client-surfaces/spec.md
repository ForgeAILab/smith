## ADDED Requirements

### Requirement: Installed agents render once in selection surfaces

Model and profile selection surfaces SHALL present installed coding agents
through the curated `cli/<kind>/<model>` namespace exactly once per agent
model, including the local installation check, even when the selection
inventory also enumerates a provider-qualified pair referencing the same
agent. A profile selecting an installed agent MUST appear selectable without
requiring any `[models]` declaration.

#### Scenario: Model picker shows one row per installed-agent model

- **GIVEN** a profile references `google/cli/claude-code/sonnet`
- **AND** the inventory enumerates the provider-qualified pair
- **WHEN** the user opens the model picker
- **THEN** `cli/claude-code/sonnet` appears exactly once from the curated
  namespace with its built-in limit labeling
- **AND** no duplicate provider-qualified row for the same agent model is
  shown

#### Scenario: Profile picker no longer marks installed-agent profiles unavailable

- **GIVEN** profiles `cc` and `cx` select `cli/claude-code/sonnet` and
  `cli/codex/gpt-6-astra` with no `[models]` declarations
- **WHEN** the user opens the profile picker
- **THEN** both profiles are selectable with their resolved provider/model
  pair
- **AND** neither is disabled with a profile-does-not-resolve reason
