## ADDED Requirements

### Requirement: Layered cache-miss notice control

Smith SHALL expose `cache.miss_notices` as an explainable layered Boolean with
a default of `false`. The setting SHALL control only local human-facing miss
notices and MUST NOT change provider requests, cache state derivation, machine
events, usage accounting, or cache-retention behavior.

#### Scenario: No setting is declared

- **GIVEN** no configuration layer declares `cache.miss_notices`
- **WHEN** Smith resolves a run
- **THEN** significant miss notices are disabled
- **AND** cache state remains available through status and machine output

#### Scenario: User enables notices

- **GIVEN** user configuration sets `cache.miss_notices = true`
- **WHEN** Smith explains and runs the resolved configuration
- **THEN** explain output names the user source and enabled value
- **AND** qualifying completed turns may append local miss notices

#### Scenario: Setting cannot alter mechanism

- **GIVEN** two otherwise identical runs differ only in
  `cache.miss_notices`
- **WHEN** both prepare and send a provider request
- **THEN** their provider request and canonical cache-state events are
  equivalent
- **AND** only local human-facing notice presentation may differ
