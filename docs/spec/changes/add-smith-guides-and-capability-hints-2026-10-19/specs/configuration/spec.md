## MODIFIED Requirements

### Requirement: Vision model selection

Smith SHALL select the vision model for a main profile from, in increasing
precedence, the top-level `vision` key and the profile's own `vision` key,
inherited through `extends`. The value grammar SHALL be the one advisor
selection uses: a profile name, a `provider/model` reference, or `false`.
Resolution of a profile or model selection SHALL follow the advisor rules.
The selected model MUST accept image input; otherwise Smith SHALL reject the
configuration at the `vision` key before any credential or provider is
constructed. This check SHALL run whatever the main model is. Whether the
selection is used SHALL be decided when the runtime is composed, from the
active model: it is used only while that model is text-only.
Resolution SHALL keep three outcomes apart: unset, disabled by an explicit
`false` (with that source), and a selection. `smith config explain vision`
SHALL report the winner and every overridden source, and SHALL print `unset`
or `disabled by <source>` when no model is selected.

#### Scenario: Top-level model reference

- **GIVEN** top-level `vision = "google/gemini-3-flash"`
- **AND** the catalog lists `image` input for that model
- **WHEN** profile `glm`, whose model is text-only, starts
- **THEN** the vision model resolves to provider `google` and model
  `gemini-3-flash`

#### Scenario: Selected model cannot see

- **GIVEN** `vision = "glm/glm-4.6"`
- **AND** that model is text-only
- **WHEN** configuration loads
- **THEN** Smith reports at the `vision` key that the model does not accept
  image input
- **AND** no credential is read

#### Scenario: Main model already sees

- **GIVEN** top-level `vision = "google/gemini-3-flash"`
- **WHEN** a profile whose model accepts image input starts
- **THEN** configuration loads
- **AND** the `vision` module reports inactive because the main model
  accepts images

#### Scenario: Unusable selection is rejected even for a vision main

- **GIVEN** `vision = "glm/glm-4.6"`, a text-only model
- **WHEN** a profile whose model accepts image input starts
- **THEN** Smith reports at the `vision` key that the model does not accept
  image input

#### Scenario: Profile disables vision

- **GIVEN** top-level `vision = "sol"`
- **AND** profile `glm` sets `vision = false`
- **WHEN** profile `glm` starts
- **THEN** no vision model is resolved
- **AND** vision resolves as disabled, with the profile `glm` source

#### Scenario: Vision left unset

- **GIVEN** no `vision` key in any layer
- **WHEN** `smith config explain vision` runs
- **THEN** it reports `unset`
- **AND** vision resolves as unset, which is distinct from disabled
