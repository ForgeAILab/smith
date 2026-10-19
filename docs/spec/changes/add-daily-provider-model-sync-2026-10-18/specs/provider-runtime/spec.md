## MODIFIED Requirements

### Requirement: Validated provider catalog cache

The Smith host SHALL load public Models.dev model metadata from a generated embedded
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

## ADDED Requirements

### Requirement: Authenticated endpoint model snapshots

Smith SHALL fetch and cache bounded OpenAI-compatible model listings from
authorized configured provider endpoints separately from the credential-free
Models.dev cache. A snapshot MUST be bound to the local provider, normalized
endpoint, adapter, and credential/account scope, and MUST contain only validated
metadata with source, revision, and successful retrieval time.

#### Scenario: Xiaoge advertises a new model

- **GIVEN** Xiaoge's authenticated listing advertises `selected-large` with
  context/input `1048576` and output `256000`
- **WHEN** a bounded discovery request succeeds
- **THEN** Smith caches that ID and its exact limits under Xiaoge's identity
- **AND** it retains validated advertised capabilities and modalities
- **AND** it writes no model records or defaults to user configuration

#### Scenario: Credential or endpoint changes

- **GIVEN** an endpoint snapshot belongs to one account and normalized URL
- **WHEN** the account, credential revision, endpoint, or adapter changes
- **THEN** that snapshot is ineligible for the new binding
- **AND** no secret appears in the snapshot, filename, log, or diagnostic

#### Scenario: A listing redirects or exceeds bounds

- **WHEN** discovery redirects, exceeds its streamed byte/time/count bounds,
  fails authentication, or returns an invalid document
- **THEN** Smith retains the previous last-good snapshot and retrieval time
- **AND** the failure does not block otherwise usable startup or model listing
- **AND** no credential is forwarded to a redirected destination

#### Scenario: Public catalog remains credential-free

- **GIVEN** authenticated endpoint discovery is enabled
- **WHEN** Smith also refreshes Models.dev
- **THEN** Models.dev receives no provider credentials or custom headers
- **AND** endpoint cache metadata cannot replace public source identity

### Requirement: Daily host-owned model synchronization

Smith SHALL refresh enabled, eligible configured sources after the configured
interval, defaulting to 24 hours since their last successful validation. Refresh
MUST run outside inventory enumeration, remain non-blocking for usable cached
models, coalesce across processes, and stop with the host process.

#### Scenario: Startup cache is stale

- **GIVEN** a usable snapshot is at least 24 hours old
- **WHEN** Smith starts with synchronization enabled
- **THEN** startup loads the snapshot immediately and schedules its refresh
- **AND** a successful refresh is published atomically for a safe rebuild

#### Scenario: Smith stays open overnight

- **GIVEN** the source's last successful validation was 24 hours ago
- **WHEN** Smith remains running without a new session start
- **THEN** the process schedules the due refresh independently of model use
- **AND** a sleep/wake interval produces one catch-up attempt per due source

#### Scenario: Concurrent Smith processes refresh

- **GIVEN** two processes observe the same due source
- **WHEN** they attempt refresh concurrently
- **THEN** a shared lock and freshness recheck avoid duplicate fetches
- **AND** readers see either the old complete snapshot or the new complete one

#### Scenario: Background credentials are unavailable

- **GIVEN** an inactive provider would require unlocking a credential service
- **WHEN** its automatic refresh becomes due
- **THEN** Smith keeps its last-good metadata with a deferred diagnostic
- **AND** the background task opens no credential-service prompt

#### Scenario: Smith exits

- **WHEN** the Smith process exits
- **THEN** its timer and in-flight refresh work are cancelled
- **AND** no system scheduler, launch agent, or service is installed

### Requirement: Safe adoption of refreshed endpoint metadata

Smith SHALL adopt refreshed model inventories only through an idle host rebuild
that preserves the same snapshot for picker selection and runtime preflight.
An active request, child, or open picker MUST retain its existing immutable
metadata; refresh MUST NOT automatically change the selected model.

#### Scenario: Refresh completes during a turn

- **GIVEN** a turn and its children use snapshot A
- **WHEN** refresh publishes snapshot B
- **THEN** their limits, capabilities, and price references stay on A
- **AND** the next safe idle rebuild can offer B's newly advertised models
- **AND** persisted history and the selected model are preserved

#### Scenario: A cached-only model is removed upstream

- **GIVEN** model X was advertised only in endpoint snapshot A
- **WHEN** a complete valid snapshot B omits X and is adopted
- **THEN** X is absent from newly built discovery choices
- **AND** existing explicit records and active runtime snapshots remain intact
