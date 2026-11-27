## ADDED Requirements

### Requirement: Minimal native Anthropic connection

Smith SHALL expose Anthropic as a trusted API-key provider in guided setup and
`/connect anthropic`, using the existing reviewed credential methods. The
normal connection and model-selection flow MUST NOT ask for or write model
limits, modalities, capabilities, or reasoning options. It SHALL write the
fixed official endpoint so the catalog binding is visible.

#### Scenario: Connect and select Claude

- **GIVEN** the user has an Anthropic API key and Smith is idle
- **WHEN** they connect Anthropic and accept the recommended
  `claude-opus-5-5`
- **THEN** Smith records provider `anthropic` with kind `anthropic-messages`,
  the official `base_url`, the reviewed credential reference, and a `claude`
  profile selecting that model
- **AND** sends no inference request during connection
- **AND** writes no explicit model metadata

#### Scenario: Choose another catalog model

- **GIVEN** the frozen Anthropic catalog lists several valid models
- **WHEN** the user opens the model choice during connection or `/model`
- **THEN** Smith lists those models with catalog provenance and any disabled
  reasons
- **AND** selecting one changes only the profile's model

#### Scenario: Recommended model is unavailable

- **GIVEN** the frozen catalog lacks valid metadata for `claude-opus-5-5`
- **WHEN** the user opens Anthropic setup or `/model`
- **THEN** Smith keeps the model unavailable with a local reason
- **AND** does not ask the user to guess limits to bypass preflight

#### Scenario: Reconnect with a replacement key

- **GIVEN** Anthropic is configured with a selected catalog model
- **WHEN** the user reconnects it with a replacement key
- **THEN** Smith changes only its authentication source
- **AND** preserves profiles, model identity, and unrelated configuration

### Requirement: Anthropic-compatible gateway setup

Smith SHALL keep a separate setup path for Anthropic-compatible gateways under
the `anthropic-messages` setup id. That path SHALL collect a provider name and
endpoint instead of fixing the official endpoint, and SHALL keep the existing
explicit-limit and catalog-ID-match behavior.

#### Scenario: Add a gateway

- **GIVEN** the user chooses the Anthropic-compatible endpoint entry
- **WHEN** they enter a provider name, gateway endpoint, credential, and model
- **THEN** Smith writes a provider of kind `anthropic-messages` at that
  endpoint with an explicit model record
- **AND** review identifies the path as a gateway, not the official
  connection

#### Scenario: Official endpoint has one route

- **GIVEN** the setup picker is open
- **WHEN** the user looks for the official Anthropic API
- **THEN** only the `anthropic` connection fixes `https://api.anthropic.com/v1`
- **AND** the gateway entry does not prefill that endpoint

### Requirement: Catalog-bound Claude effort selection

Smith SHALL drive `/think`, `/effort`, `--effort`, `/status`, and `/context`
for the official `anthropic` binding from the catalog-derived capability
snapshot, without provider probing or model-name special cases. Reasoning on
this binding is mandatory-on with a `high` default effort, so these surfaces
MUST NOT offer an off switch.

#### Scenario: Effort picker lists the catalog ladder

- **GIVEN** the idle binding is `anthropic/claude-opus-5-5` with a catalog
  ladder of `low` through `max`
- **WHEN** the user opens `/effort`
- **THEN** the picker lists those levels in advertised order with `high`
  marked as the default
- **AND** opening or choosing the setting sends no provider request

#### Scenario: Thinking cannot be turned off

- **GIVEN** the official binding with a catalog effort ladder
- **WHEN** the user opens `/think` or submits `/think off`
- **THEN** the off choice is unavailable with a written reason
- **AND** the direct command fails locally without provider I/O

#### Scenario: Toggle-only model shows no effort controls

- **GIVEN** the binding selects a catalog model that advertises only a toggle
- **WHEN** the user opens `/effort` or passes `--effort high`
- **THEN** the picker reports no effort controls with a written reason
- **AND** the flag fails locally without provider I/O
