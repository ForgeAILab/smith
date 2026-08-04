## ADDED Requirements
### Requirement: Validated provider catalog cache

The Smith host SHALL load provider model metadata from a generated embedded
seed or a schema-validated last-good user cache and MAY refresh that cache only
from Smith's exact public Models.dev HTTPS source. Catalog loading and refresh
MUST be bounded, credential-free, atomically published, and non-blocking for an
otherwise usable embedded or cached snapshot.

#### Scenario: Valid last-good cache exists

- **GIVEN** a schema-valid last-good cache exists
- **WHEN** Smith prepares the host catalog
- **THEN** it uses that cache without requiring a network response
- **AND** schedules refresh only according to the cache freshness policy

#### Scenario: Cache is absent or corrupt

- **GIVEN** the user cache is missing, truncated, malformed, or fails schema
  validation
- **WHEN** Smith prepares the host catalog
- **THEN** it uses the generated embedded seed
- **AND** the invalid cache contributes no model metadata

#### Scenario: Refresh succeeds

- **GIVEN** the current snapshot is stale
- **WHEN** the bounded Models.dev refresh returns a complete valid response
  from the allowed origin
- **THEN** Smith writes a temporary file in the cache directory and atomically
  publishes it as the new last-good cache
- **AND** exposes the new snapshot only to a later host rebuild

#### Scenario: Refresh fails safely

- **GIVEN** refresh times out, exceeds the byte limit, redirects outside the
  allowed origin, returns a bad status, or fails schema validation
- **WHEN** the refresh attempt ends
- **THEN** Smith retains the current last-good or embedded snapshot unchanged
- **AND** startup, the current session, and local picker interaction remain
  usable

#### Scenario: Refresh sends no provider secret

- **GIVEN** configured providers reference environment, keychain, or inline
  credentials
- **WHEN** Smith requests public Models.dev metadata
- **THEN** the request contains no provider credential or provider-specific
  authorization/header value
- **AND** Smith does not open the credential backend for catalog refresh

### Requirement: Frozen catalog-backed runtime profile

Smith SHALL pass the same immutable catalog source used for model enumeration
into runtime construction. Model limits and capabilities MUST be resolved and
frozen before provider I/O, and a later catalog refresh MUST NOT mutate an
active runtime or session.

#### Scenario: Select a catalog-only model

- **GIVEN** `/model` offers a selectable catalog model with no explicit local
  model record
- **WHEN** the user selects that provider/model pair
- **THEN** the host rebuild injects the same frozen catalog source into
  `RuntimeRequest.catalog_sources`
- **AND** runtime preflight resolves complete limits before any provider request

#### Scenario: Catalog changes during a session

- **GIVEN** a host was constructed from catalog revision A
- **WHEN** background refresh publishes catalog revision B
- **THEN** the active runtime continues using revision A
- **AND** revision B can affect selection or preflight only after a later host
  rebuild

#### Scenario: Catalog and picker cannot diverge

- **GIVEN** a catalog-backed entry is marked selectable in `/model`
- **WHEN** that exact entry is applied without intervening host rebuild
- **THEN** runtime preflight uses the snapshot that established its
  selectability
- **AND** does not refetch or substitute metadata by insertion order

#### Scenario: Catalog metadata conflicts at equal precedence

- **GIVEN** two cached-remote sources disagree about the same winning model
  field
- **WHEN** runtime profile resolution runs
- **THEN** existing same-layer conflict handling fails before provider I/O
- **AND** Smith does not choose whichever source was registered first
