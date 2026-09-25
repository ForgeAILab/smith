## ADDED Requirements

### Requirement: Explicit native Anthropic reasoning metadata

Smith SHALL accept `anthropic-effort` as a trusted reasoning request dialect
for an exact explicitly configured provider/model binding. The metadata SHALL
be able to declare mandatory reasoning, an ordered effort ladder, defaults,
and provenance. Smith MUST NOT infer this dialect from a provider alias,
endpoint shape, or model-name prefix.

#### Scenario: Explicit Fable 5.1 controls resolve

- **GIVEN** an exact dddai Fable 5.1 model entry declares mandatory reasoning,
  efforts `low`, `medium`, `high`, `xhigh`, and `max`, default effort `high`,
  and dialect `anthropic-effort`
- **WHEN** Smith resolves the model profile
- **THEN** the capability snapshot retains the ordered effort ladder and
  mandatory-on state
- **AND** its provenance identifies the exact explicit model metadata

#### Scenario: Anthropic-looking model lacks explicit controls

- **GIVEN** a model name begins with `claude-` or `fable-`
- **BUT** its exact trusted metadata does not declare a reasoning dialect
- **WHEN** Smith resolves the model profile
- **THEN** Smith does not infer Anthropic effort controls
- **AND** an attempted effort selection fails before credential or provider I/O

#### Scenario: Unsupported Anthropic effort is selected

- **GIVEN** exact metadata advertises the five Fable 5.1 effort levels
- **WHEN** configuration or invocation selects a different value
- **THEN** startup fails locally with the requested value and supported ladder
- **AND** no provider request is made
