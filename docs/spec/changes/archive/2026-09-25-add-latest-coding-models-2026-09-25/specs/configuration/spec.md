## MODIFIED Requirements

### Requirement: Named context windows

A model binding SHALL be able to declare one or more named context windows
and exactly one default window. Each window SHALL carry `context_tokens`, and
MAY carry `max_input_tokens`. When a window omits
`max_input_tokens`, Smith SHALL derive it as `context_tokens` minus the
model's resolved output ceiling. One configuration layer MUST NOT declare
both windows and a flat `context_tokens` or `max_input_tokens` for the same
binding. An explicit flat limit SHALL override lower-layer windows and pin
the binding to one window. Smith SHALL ship reviewed direct-ChatGPT metadata
for every GPT-6 model advertised by the installed Codex catalog.

#### Scenario: Window limits resolve

- **GIVEN** `chatgpt/gpt-5.6-terra` declares windows `272k` (default) and
  `872k` with an output ceiling of 16,384
- **WHEN** Smith resolves the model with window `872k` selected
- **THEN** the profile has `context_tokens = 872000` and
  `max_input_tokens = 855616`
- **AND** `max_output_tokens` is unchanged

#### Scenario: Model without windows is unchanged

- **GIVEN** a model binding declares no context windows
- **WHEN** Smith resolves the model
- **THEN** its limits resolve exactly as they did before this change

#### Scenario: Every ChatGPT GPT-5.6 and GPT-6 model has windows

- **GIVEN** no `[models]` block for the model
- **WHEN** Smith resolves `chatgpt/gpt-5.6-sol`, `chatgpt/gpt-5.6-terra`,
  `chatgpt/gpt-5.6-luna`, `chatgpt/gpt-6-astra`, `chatgpt/gpt-6-sol`, or
  `chatgpt/gpt-6-luna`
- **THEN** the model resolves with the `272k` window as default
- **AND** `872k` is selectable

#### Scenario: Existing flat override still wins

- **GIVEN** the user config declares `[models."chatgpt/gpt-5.6-luna"]` with a
  flat `context_tokens = 272000`
- **WHEN** Smith resolves the model
- **THEN** the flat limits apply unchanged
- **AND** selecting `872k` fails with an error that names the flat key

#### Scenario: Ambiguous limit declaration is rejected

- **GIVEN** one config layer declares both `context_windows` and a flat
  `context_tokens` for the same binding
- **WHEN** configuration loads
- **THEN** Smith reports a configuration error that names both keys

## ADDED Requirements

### Requirement: Embedded provider catalog fallback

Smith SHALL bundle a normalized Models.dev snapshot recent enough to include
the coding models supported at release time, including GPT-6 Sol, GPT-6 Luna,
and Claude Opus 5.5. A valid last-good user cache MAY supersede that snapshot.

#### Scenario: Offline startup sees the release catalog

- **GIVEN** Smith 0.2.14 starts without a user catalog cache and cannot refresh
  Models.dev
- **WHEN** it loads the embedded catalog
- **THEN** GPT-6 Sol, GPT-6 Luna, and Claude Opus 5.5 metadata is available to
  compatible endpoint-bound providers and exact model setup review
