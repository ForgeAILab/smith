## ADDED Requirements

### Requirement: Minimal native Gemini connection

Smith SHALL expose Google Gemini as a trusted native API-key provider in guided
setup and `/connect google`. The normal connection and model-selection flow
MUST NOT ask for or write an endpoint, model limits, modalities, capabilities,
or reasoning options.

#### Scenario: Connect and select Gemini

- **GIVEN** the user has a Gemini API key and Smith is idle
- **WHEN** they connect Google and select a valid catalog model
- **THEN** Smith records only the trusted provider identity, reviewed
  credential reference, and profile selection
- **AND** sends no inference request during connection
- **AND** writes no explicit model metadata

#### Scenario: Catalog model is unavailable

- **GIVEN** the frozen catalog lacks valid metadata for the recommended model
- **WHEN** the user opens Google setup or `/model`
- **THEN** Smith keeps the model unavailable with a local reason
- **AND** does not ask the user to guess limits to bypass preflight

#### Scenario: Reconnect without changing catalog selection

- **GIVEN** Google is configured with a selected catalog model
- **WHEN** the user reconnects it with a replacement key
- **THEN** Smith changes only its authentication source
- **AND** preserves profiles, model identity, catalog provenance, and unrelated
  configuration

### Requirement: Focused model override editing

Smith SHALL direct explicit model metadata and override workflows to the
applicable `models.toml` file. Reviews and diagnostics MUST identify that file
and MUST NOT present unrelated runtime policy as part of a model edit.

#### Scenario: Add a custom model override

- **GIVEN** a user deliberately overrides a catalog model field
- **WHEN** Smith prepares the reviewed edit
- **THEN** the preview targets `~/.smith/models.toml`
- **AND** shows only the affected model fields and secret-free provenance
- **AND** leaves `config.toml` unchanged
