## ADDED Requirements
### Requirement: Catalog-augmented runtime inventory

Smith SHALL augment the pure local selection inventory with models from an
immutable, schema-validated catalog snapshot only for configured providers
whose available adapter and normalized endpoint match a Smith-owned catalog
binding. Catalog data MUST retain the configured provider's local identity and
MUST NOT create or modify a provider, endpoint, adapter, credential, header, or
profile.

#### Scenario: Configured OpenRouter exposes catalog models

- **GIVEN** effective configuration declares an available OpenAI-compatible
  provider at the normalized OpenRouter endpoint
- **AND** the local catalog snapshot contains valid OpenRouter models not named
  by local model records or profiles
- **WHEN** Smith builds the selection inventory
- **THEN** it includes those valid models under the configured provider's local
  identity
- **AND** it does not add any other Models.dev provider

#### Scenario: Z.AI quick start uses the Coding Plan catalog

- **GIVEN** effective configuration declares Smith's Z.AI Coding Plan endpoint
- **AND** the local catalog snapshot contains several valid
  `zai-coding-plan` models
- **WHEN** Smith builds the selection inventory
- **THEN** it includes those models as `zai/<model-id>` pairs
- **AND** retains the local provider identity `zai`

#### Scenario: Familiar provider name points elsewhere

- **GIVEN** a configured provider is named `openrouter`
- **BUT** its normalized endpoint is not the Smith-owned OpenRouter binding
- **WHEN** Smith builds the selection inventory
- **THEN** it does not attach the OpenRouter catalog to that provider
- **AND** only locally configured or otherwise trusted models remain candidates

#### Scenario: Inventory consumes a prepared snapshot

- **GIVEN** the host has prepared an immutable catalog snapshot
- **WHEN** Smith enumerates profiles, providers, and models or filters a picker
- **THEN** enumeration reads only configuration and that in-memory snapshot
- **AND** does not access a network, provider credential, keychain, or provider
  endpoint

### Requirement: Catalog model validation and precedence

Smith SHALL normalize only schema-valid catalog metadata into provider-scoped
model records. Explicit Smith model configuration MUST retain field-level
precedence over catalog metadata, and every winning catalog field MUST identify
its catalog revision and retrieval provenance.

#### Scenario: Complete catalog limits are normalized

- **GIVEN** a catalog model publishes positive context and output limits within
  Smith's integer bounds
- **AND** its optional separate input limit is absent
- **WHEN** Smith normalizes the model record
- **THEN** `context_tokens` is the published context limit
- **AND** `max_output_tokens` is the published output limit
- **AND** `max_input_tokens` is the published total context limit
- **AND** runtime context policy still holds back declared output and reasoning
  reserves before admitting input

#### Scenario: Separate input limit is published

- **GIVEN** a catalog model publishes a positive input limit no greater than its
  total context limit
- **WHEN** Smith normalizes the model record
- **THEN** `max_input_tokens` is that separate published input limit
- **AND** no larger input ceiling is inferred

#### Scenario: Invalid catalog limits

- **GIVEN** a catalog entry has a zero or out-of-range limit, output above
  context, or a separate input limit above context
- **WHEN** Smith validates the snapshot
- **THEN** that entry cannot become a selectable model record
- **AND** Smith does not clamp or guess a replacement limit

#### Scenario: Effective reserves leave no input budget

- **GIVEN** a catalog model has internally valid published limits
- **BUT** the effective Smith output and reasoning reserves equal or exceed its
  context window
- **WHEN** Smith prepares runtime choices
- **THEN** the model is visible but disabled with a local reserve diagnostic
- **AND** Smith does not lower the published output ceiling or configured
  reserve to make it selectable

#### Scenario: Explicit limit overrides catalog metadata

- **GIVEN** a catalog record and an explicit
  `[models."<provider>/<model>"]` record both supply a limit field
- **WHEN** Smith resolves the model profile
- **THEN** the explicit field wins through the existing catalog precedence
- **AND** catalog and explicit contributions remain available for provenance
  diagnostics

#### Scenario: Catalog cannot grant provider entitlement

- **GIVEN** a valid catalog model is associated with a configured provider
- **WHEN** Smith adds it to the inventory
- **THEN** Smith labels it as catalog-advertised rather than account-verified
- **AND** does not claim that the credential, subscription, region, or account
  can use that model
