## ADDED Requirements
### Requirement: Catalog-backed model picker

Smith SHALL list catalog-backed models for recognized configured providers in
the existing searchable `/model` picker. Entries MUST remain
provider-qualified, deterministic, bounded for large catalogs, and coherent
with direct selection and `/provider` cascading behavior.

#### Scenario: OpenRouter picker is not limited to local TOML

- **GIVEN** OpenRouter is configured with one explicit local model
- **AND** the prepared catalog snapshot contains additional valid OpenRouter
  models
- **WHEN** the user opens `/model`
- **THEN** the picker includes the explicit model and additional catalog-backed
  models under the OpenRouter provider
- **AND** filtering can match model ID, display name, provider, or capability
  detail

#### Scenario: Z.AI Coding Plan lists its supported catalog

- **GIVEN** Smith's `zai/glm-4.7` quick start is active
- **AND** the prepared Z.AI Coding Plan catalog contains other valid models
- **WHEN** the user opens `/model`
- **THEN** those models appear as distinct `zai/<model-id>` choices
- **AND** `zai/glm-4.7` remains marked current

#### Scenario: Provider picker uses catalog model count

- **GIVEN** a configured provider has several selectable catalog models
- **WHEN** the user opens `/provider` and chooses it
- **THEN** the provider detail shows the selectable catalog-augmented count
- **AND** Smith opens `/model` filtered to that provider rather than applying
  an arbitrary model

#### Scenario: Incompatible catalog model is explained locally

- **GIVEN** a catalog entry is deprecated or lacks text output, tool calling,
  complete valid limits, or a usable input budget under effective reserves
- **WHEN** Smith prepares or filters model choices
- **THEN** deprecated entries are omitted and other incompatible entries are
  non-selectable with a bounded reason
- **AND** confirming a disabled entry sends no provider request

#### Scenario: Directly choose a catalog model

- **GIVEN** `openrouter/vendor/model` is a unique selectable catalog-backed
  choice
- **WHEN** the user submits `/model openrouter/vendor/model`
- **THEN** Smith applies provider `openrouter` and model `vendor/model`
  atomically
- **AND** preserves nested slashes inside the provider model ID

#### Scenario: Large catalog remains usable

- **GIVEN** a configured provider contributes hundreds of catalog models
- **WHEN** `/model` is opened in a narrow or wide terminal
- **THEN** rendering remains bounded to the viewport and filtering remains
  keyboard-first
- **AND** deterministic ordering, selection, Enter, and Escape behavior remain
  unchanged

#### Scenario: Picker opens while offline

- **GIVEN** networking is unavailable
- **AND** Smith has a valid last-good or embedded catalog snapshot
- **WHEN** the user opens, searches, cancels, or confirms `/model`
- **THEN** picker behavior uses only the prepared snapshot
- **AND** displays no network or credential prompt

#### Scenario: Advertised model is unavailable to the account

- **GIVEN** a catalog-backed model passes local metadata preflight
- **BUT** the provider later rejects it for account, plan, or region reasons
- **WHEN** the first provider request fails
- **THEN** Smith reports the provider error without removing or rewriting user
  configuration
- **AND** does not misrepresent catalog advertisement as verified entitlement
