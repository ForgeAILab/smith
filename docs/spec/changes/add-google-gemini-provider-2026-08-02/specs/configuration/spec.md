## ADDED Requirements

### Requirement: Dedicated model catalog files

Smith SHALL discover optional `models.toml` files for explicit model metadata
at user and project scopes, plus `models.local.toml` at project-local scope.
These files MUST accept only model declarations and MUST retain ordinary layer
and path provenance.

#### Scenario: Load a user model override

- **GIVEN** `~/.smith/models.toml` contains a valid explicit model field
- **WHEN** Smith resolves the matching provider/model profile
- **THEN** the field participates at user-file precedence
- **AND** its provenance identifies the dedicated file and exact model key

#### Scenario: Unrelated setting appears in models file

- **GIVEN** a models file declares a provider, profile, credential, approval,
  persistence, tool, or other policy setting
- **WHEN** Smith parses the file
- **THEN** it rejects the unknown or forbidden field before provider or
  credential I/O
- **AND** does not merge any part of the malformed file

#### Scenario: Project model metadata cannot grant authority

- **GIVEN** a project models file advertises capabilities or larger limits
- **WHEN** Smith loads the project layer
- **THEN** those fields remain model metadata subject to project trust and
  runtime capability validation
- **AND** cannot create an adapter, endpoint, credential, tool, approval, or
  wider workspace

### Requirement: Model-file migration compatibility

Smith SHALL accept legacy `[models]` tables in `config*.toml` for one transition
release and provide a reviewed migration into dedicated model files. It MUST
reject same-scope duplicate model fields rather than choosing by file order.

#### Scenario: Read legacy model configuration

- **GIVEN** an existing config contains a valid `[models]` table and no
  duplicate dedicated-file field
- **WHEN** Smith resolves configuration during the transition release
- **THEN** it preserves existing behavior and provenance
- **AND** reports a bounded migration diagnostic

#### Scenario: Legacy and dedicated files conflict

- **GIVEN** the same model field is declared in config and models files at one
  scope
- **WHEN** Smith resolves the layer
- **THEN** it reports an ambiguity before provider or credential I/O
- **AND** does not silently prefer discovery order

#### Scenario: Migration preflight fails

- **GIVEN** Smith has prepared new config and models file candidates
- **WHEN** combined resolution or runtime preflight fails
- **THEN** it restores the exact prior bytes of every affected file
- **AND** leaves no partially migrated model declaration

### Requirement: Built-in Google catalog resolution

Smith SHALL define `google` as a trusted `gemini-interactions` provider with a
fixed native endpoint and SHALL resolve its model metadata from the frozen
Models.dev `google` catalog. Normal setup MUST require no explicit model record
or endpoint.

#### Scenario: Resolve Google model automatically

- **GIVEN** configuration selects provider `google` and model
  `gemini-3.6-flash`
- **AND** the frozen Google catalog contains valid metadata for that model
- **WHEN** Smith resolves inventory and runtime preflight
- **THEN** limits, modalities, capabilities, reasoning support, and effort
  names come from that same snapshot
- **AND** no `[models]` table or base URL is required

#### Scenario: Catalog cannot configure authentication

- **GIVEN** Models.dev advertises environment names, an SDK package, or other
  provider integration fields
- **WHEN** Smith normalizes the Google catalog
- **THEN** it imports none of those fields
- **AND** the trusted Smith endpoint, adapter, and credential policy remain
  authoritative
