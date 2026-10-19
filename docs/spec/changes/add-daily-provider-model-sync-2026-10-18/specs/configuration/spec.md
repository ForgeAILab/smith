## MODIFIED Requirements

### Requirement: Catalog-augmented runtime inventory

Smith SHALL augment the pure local selection inventory with models from an
immutable, schema-validated catalog snapshot only for configured providers
whose available adapter and normalized endpoint match a Smith-owned catalog
binding or a validated endpoint snapshot scoped to their exact configured
provider/adapter/endpoint/account identity. Catalog data MUST retain the configured provider's local identity and
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
- **AND** only locally configured, otherwise trusted, or exact-endpoint cached
  models remain candidates

#### Scenario: Inventory consumes a prepared snapshot

- **GIVEN** the host has prepared an immutable catalog snapshot
- **WHEN** Smith enumerates profiles, providers, and models or filters a picker
- **THEN** enumeration reads only configuration and that in-memory snapshot
- **AND** does not access a network, provider credential, keychain, or provider
  endpoint

#### Scenario: Custom endpoint adds a model without a TOML record

- **GIVEN** configured provider `xiaoge` has a prepared valid endpoint snapshot
- **AND** that snapshot advertises `selected-large` with usable model metadata
- **WHEN** Smith builds inventory and runtime preflight from that snapshot
- **THEN** it offers `xiaoge/selected-large` with the same frozen limits
- **AND** explicit model fields retain precedence over advertised fields
- **AND** another endpoint's models or prices are never inherited by name


### Requirement: Catalog model validation and precedence

Smith SHALL normalize only schema-valid catalog metadata into provider-scoped
model records. Explicit Smith model configuration MUST retain field-level
precedence over catalog metadata, and every winning catalog field MUST identify
its catalog revision and retrieval provenance. A catalog entry MAY carry an
optional per-counter price in USD per million tokens. Individually valid
prices MUST be retained; missing, ill-typed, negative, or overflowing counters
MUST remain unpriced without rejecting or disabling the model. A model's
published maximum output MUST remain a ceiling rather than becoming Smith's
implicit per-request output budget.

#### Scenario: Complete catalog limits are normalized

- **GIVEN** a catalog model publishes positive context and output limits within
  Smith's integer bounds
- **AND** its optional separate input limit is absent
- **WHEN** Smith normalizes the model record
- **THEN** `context_tokens` is the published context limit
- **AND** the model profile's `max_output_tokens` is the published output
  ceiling
- **AND** `max_input_tokens` is the published total context limit
- **AND** runtime context policy holds back the effective output and reasoning
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

#### Scenario: A published price is normalized

- **GIVEN** a catalog model publishes finite non-negative input, output, cache
  read, and cache write costs
- **WHEN** Smith normalizes the model record
- **THEN** each cost is retained per counter in USD per million tokens
- **AND** it carries the same catalog revision and retrieval provenance as the
  entry's other fields

#### Scenario: A partial or ill-typed price

- **GIVEN** a catalog model publishes only an input cost, or publishes a
  negative, infinite, or non-numeric cost
- **WHEN** Smith normalizes the model record
- **THEN** the model stays selectable
- **AND** only the individually valid costs are retained, with no value
  inferred for a counter the source did not price

#### Scenario: A price cannot disable a model

- **GIVEN** a catalog model publishes no price at all
- **WHEN** Smith prepares runtime choices
- **THEN** the model is selectable exactly as it is today
- **AND** the absence of a price is not a validation diagnostic

#### Scenario: A snapshot at the prior schema revision

- **GIVEN** a cached catalog snapshot was written at the schema revision before
  prices existed
- **WHEN** Smith loads it
- **THEN** it is rejected as stale by the existing revision check
- **AND** Smith falls back to the embedded seed and schedules a refresh, as it
  does for any stale revision

#### Scenario: Published output ceiling equals context

- **GIVEN** a valid catalog text/tool model publishes a 500,000-token context
  and a 500,000-token maximum output ceiling
- **AND** no explicit request output limit or context output reserve is selected
- **WHEN** Smith prepares runtime choices
- **THEN** it derives a 32,768-token automatic request output budget and default
  output reserve
- **AND** the model remains selectable without a local model override
- **AND** the published 500,000-token ceiling and provenance remain unchanged

#### Scenario: Explicit reserves leave no input budget

- **GIVEN** a catalog model has internally valid published limits
- **BUT** an explicit Smith output or reasoning reserve equals or exceeds its
  context window when combined
- **WHEN** Smith prepares runtime choices
- **THEN** the model is visible but disabled with a local reserve diagnostic
- **AND** Smith does not clamp the explicit values to make it selectable

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

## ADDED Requirements

### Requirement: User-owned model synchronization policy

Smith SHALL expose user-only `model_sync.enabled` and
`model_sync.interval_hours`, defaulting to `true` and `24` respectively, with
a validated integer interval from 1 through 168 hours. This policy governs
process-owned refresh for eligible configured sources; it MUST NOT authorize
project-controlled credential forwarding, endpoint changes, or model overrides.

#### Scenario: Default daily refresh

- **GIVEN** the user has not configured a synchronization policy
- **WHEN** Smith prepares eligible authorized providers
- **THEN** synchronization is enabled with a 24-hour interval
- **AND** offline model enumeration remains independent of refresh

#### Scenario: User disables synchronization

- **GIVEN** owner-controlled configuration sets `model_sync.enabled = false`
- **WHEN** Smith starts or remains open past the interval
- **THEN** no automatic source refresh is scheduled
- **AND** existing caches and explicit `smith models sync` remain usable

#### Scenario: Invalid or project-owned policy

- **GIVEN** an interval is zero, non-integer, above 168, or the policy is
  declared by a project configuration layer
- **WHEN** Smith validates configuration
- **THEN** it reports the offending key and source before scheduling refresh
- **AND** no credential read or metadata request is triggered by that value
