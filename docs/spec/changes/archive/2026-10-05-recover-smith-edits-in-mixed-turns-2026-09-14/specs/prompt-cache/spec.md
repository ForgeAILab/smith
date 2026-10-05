## MODIFIED Requirements

### Requirement: Significant cache-miss notices are factual and optional

Smith SHALL gate local cache-miss transcript notices behind the layered
`cache.miss_notices` setting, defaulting to enabled. When enabled, it SHALL
emit at most one notice for a completed root turn whose canonical misses total
at least 20,000 tokens or whose known derived extra cost is at least $0.10.
Elapsed idle time MAY be displayed as factual context but MUST NOT establish or
claim expiry.

#### Scenario: Notices are on without configuration

- **GIVEN** no configuration layer declares `cache.miss_notices`
- **AND** a completed root turn crosses the significant-miss threshold
- **WHEN** the turn completes
- **THEN** Smith appends one bounded factual miss notice
- **AND** a layer that sets the flag to `false` suppresses it without changing
  provider requests or canonical cache evidence

#### Scenario: Large miss follows an idle gap

- **GIVEN** notices are enabled
- **AND** one logical request misses 105,000 expected cache-read tokens after
  nine minutes without another logical provider request
- **WHEN** the root turn completes
- **THEN** Smith appends a local `Cache miss after 9m idle` notice with the
  re-billed tokens
- **AND** it does not call the cache expired or verified unavailable

#### Scenario: Small miss stays quiet

- **GIVEN** notices are enabled
- **AND** a completed turn misses fewer than 20,000 tokens
- **AND** its known derived extra cost is less than $0.10
- **WHEN** the turn completes
- **THEN** no transcript notice is appended
- **AND** the canonical state and status metrics remain available

#### Scenario: Provider diagnostic is unavailable

- **GIVEN** a cache miss and any elapsed idle duration
- **AND** no provider diagnostic established matching requests plus an
  unavailable cache entry
- **WHEN** Smith renders the miss
- **THEN** it uses `Cache miss` or `Cache miss after Nm idle`
- **AND** it does not use `expired` or `likely expired`
