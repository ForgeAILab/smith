## ADDED Requirements

### Requirement: Named context windows

A model binding SHALL be able to declare one or more named context windows
and exactly one default window. Each window SHALL carry `context_tokens`, and
MAY carry `max_input_tokens`. When a window omits
`max_input_tokens`, Smith SHALL derive it as `context_tokens` minus the
model's resolved output ceiling. One configuration layer MUST NOT declare
both windows and a flat `context_tokens` or `max_input_tokens` for the same
binding. An explicit flat limit SHALL override lower-layer windows and pin
the binding to one window.

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
  `chatgpt/gpt-5.6-luna`, or `chatgpt/gpt-6-astra`
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

### Requirement: Context window selection precedence

Smith SHALL select the active window from, in increasing precedence: the
model default, profile `context_window`, the `--context-window` flag, and the
session override. `smith config explain context_window` SHALL report the
winner and every overridden source. Smith SHALL reject an unknown window name
before any credential or provider I/O.

#### Scenario: Session override wins

- **GIVEN** a profile sets `context_window = "272k"`
- **WHEN** the session override selects `872k`
- **THEN** the run uses the `872k` window
- **AND** `config explain` lists the profile value as overridden

#### Scenario: Unknown window name

- **WHEN** `--context-window 2m` is given for a model without a `2m` window
- **THEN** Smith fails before resolving credentials
- **AND** the error lists the valid window names
