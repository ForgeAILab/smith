## ADDED Requirements

### Requirement: Built-in Anthropic catalog resolution

Smith SHALL define `anthropic` as a trusted `anthropic-messages` provider bound
to the exact endpoint `https://api.anthropic.com/v1`, and SHALL resolve its
model metadata from the frozen Models.dev `anthropic` catalog. The binding
SHALL also apply when `base_url` is omitted, because that resolves to the same
endpoint. Normal setup MUST require no explicit model record.

#### Scenario: Resolve a Claude model automatically

- **GIVEN** configuration selects provider `anthropic` with kind
  `anthropic-messages`, `base_url` `https://api.anthropic.com/v1`, and model
  `claude-opus-5-5`
- **AND** the frozen Anthropic catalog contains valid metadata for that model
- **WHEN** Smith resolves inventory and runtime preflight
- **THEN** limits, modalities, tool support, prices, and the effort ladder come
  from that same snapshot
- **AND** no `[models]` table is required

#### Scenario: Gateway endpoint inherits nothing

- **GIVEN** a provider of kind `anthropic-messages` at any endpoint other than
  the exact normalized official endpoint
- **WHEN** Smith resolves a model whose ID matches an Anthropic catalog entry
- **THEN** the catalog binding does not apply
- **AND** the provider still needs explicit limits, as it does today

#### Scenario: Gateway setup matches the Anthropic catalog by ID

- **GIVEN** a gateway advertises model `claude-fable-5-1` without limit fields
- **AND** the trusted catalog contains `anthropic/claude-fable-5-1` with
  complete limits
- **WHEN** setup falls back from the endpoint listing to the catalog
- **THEN** setup prefills limits from that exact Anthropic entry and names it
  in review
- **AND** the written provider remains a gateway with an explicit model record

#### Scenario: Explicit model record wins

- **GIVEN** the official binding and an explicit
  `[models."anthropic/claude-opus-5-5"]` record that sets a smaller context
  window
- **WHEN** Smith resolves the model profile
- **THEN** the explicit value is used with configured provenance
- **AND** catalog values fill only the fields the record omits

#### Scenario: Catalog cannot configure authentication

- **GIVEN** Models.dev advertises environment names, an SDK package, or other
  provider integration fields for `anthropic`
- **WHEN** Smith normalizes the Anthropic catalog
- **THEN** it imports none of those fields
- **AND** the trusted Smith endpoint, adapter, and credential policy remain
  authoritative

## MODIFIED Requirements

### Requirement: Explicit native Anthropic reasoning metadata

Smith SHALL accept `anthropic-effort` as a trusted reasoning request dialect
for an exact explicitly configured provider/model binding. The metadata SHALL
be able to declare mandatory reasoning, an ordered effort ladder, defaults,
and provenance. Smith SHALL also derive the dialect for the trusted built-in
`anthropic` catalog binding at the exact official endpoint when the frozen
catalog advertises an effort ladder for the selected model. Smith MUST NOT
infer this dialect from a provider alias, endpoint shape, or model-name
prefix.

#### Scenario: Explicit Fable 5.1 controls resolve

- **GIVEN** an exact dddai Fable 5.1 model entry declares mandatory reasoning,
  efforts `low`, `medium`, `high`, `xhigh`, and `max`, default effort `high`,
  and dialect `anthropic-effort`
- **WHEN** Smith resolves the model profile
- **THEN** the capability snapshot retains the ordered effort ladder and
  mandatory-on state
- **AND** its provenance identifies the exact explicit model metadata

#### Scenario: Catalog-advertised Claude controls resolve

- **GIVEN** the official `anthropic` binding selects `claude-opus-5-5`
- **AND** the frozen catalog advertises efforts `low`, `medium`, `high`,
  `xhigh`, and `max` for it
- **AND** no explicit reasoning metadata exists for that model
- **WHEN** Smith resolves the model profile
- **THEN** the capability snapshot uses dialect `anthropic-effort` with that
  ladder in advertised order, mandatory-on reasoning, and default effort
  `high`
- **AND** its provenance names the Models.dev Anthropic catalog

#### Scenario: Anthropic-looking model lacks explicit controls

- **GIVEN** a model name begins with `claude-` or `fable-`
- **BUT** its exact trusted metadata does not declare a reasoning dialect
- **AND** the binding is not the official `anthropic` catalog binding with an
  advertised effort ladder
- **WHEN** Smith resolves the model profile
- **THEN** Smith does not infer Anthropic effort controls
- **AND** an attempted effort selection fails before credential or provider I/O

#### Scenario: Unsupported Anthropic effort is selected

- **GIVEN** exact metadata advertises the five Fable 5.1 effort levels
- **WHEN** configuration or invocation selects a different value
- **THEN** startup fails locally with the requested value and supported ladder
- **AND** no provider request is made
