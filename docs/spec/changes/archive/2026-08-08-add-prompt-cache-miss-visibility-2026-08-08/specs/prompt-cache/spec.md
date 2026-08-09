## MODIFIED Requirements

### Requirement: Evidence-based cache status

Smith SHALL project Agent Runtime's attributed cache evidence into at least
`unsupported`, `unknown`, `eligible`, `warm_observed`,
`miss_observed`, and Smith-owned `suspended` states. Only provider-reported
usage or a provider cache API exposed through canonical shared events MAY
establish `warm_observed` or `miss_observed`. Smith MUST preserve the
difference between an explicit zero and omitted cache evidence, MUST correlate
state by request, attempt, and exact cache-plan fingerprint, and MUST NOT treat
a first eligible request or a changed cache identity as a miss.

#### Scenario: Cache read tokens are reported

- **GIVEN** a provider response reports a positive cache-read count
- **WHEN** Agent Runtime emits the attributed canonical cache state
- **THEN** Smith records `warm_observed` for the matching identity
- **AND** attributes the cache-read tokens to that request and attempt

#### Scenario: Explicit zero follows a reusable plan

- **GIVEN** Agent Runtime reports a positive expected cache read for a
  comparable plan
- **AND** the provider explicitly reports zero cache-read tokens
- **WHEN** Smith reduces the canonical state event
- **THEN** it records `miss_observed` and the shared derived missed-token count
- **AND** it does not reinterpret the zero as missing provider data

#### Scenario: Cache evidence is omitted

- **GIVEN** provider caching is supported
- **AND** the completed response contains no cache observation
- **WHEN** Smith updates cache status
- **THEN** the status is `unknown` rather than `miss_observed`
- **AND** Smith fabricates neither a zero read nor re-billed tokens

#### Scenario: First eligible request reports zero

- **GIVEN** a cache-capable first request has no comparable predecessor
- **AND** the provider explicitly reports zero
- **WHEN** Smith updates cache status
- **THEN** it remains `eligible`
- **AND** no miss notice or re-billed total is added

#### Scenario: Provider or model identity changes

- **GIVEN** the next request resolves a different exact cache identity
- **WHEN** Agent Runtime reports no reusable expectation from the prior plan
- **THEN** Smith clears the prior identity's hit indicator
- **AND** it does not count the non-transferable prior prefix as a cache miss

## ADDED Requirements

### Requirement: Smith-owned adapters preserve zero cache evidence

Every Smith-owned provider adapter SHALL follow Agent Runtime's presence-aware
cache observation contract. It MUST emit a present zero when the provider
reported zero, MUST leave an omitted field absent, and MUST preserve disjoint
uncached, cached, and cache-write usage.

#### Scenario: Experimental ChatGPT response reports zero

- **GIVEN** the ChatGPT Responses usage object contains `cached_tokens: 0`
- **WHEN** Smith's adapter normalizes the response
- **THEN** it emits a present zero cache-read observation
- **AND** it records no positive `InputCached` counter

### Requirement: Significant cache-miss notices are factual and optional

Smith SHALL gate local cache-miss transcript notices behind the layered
`cache.miss_notices` setting, defaulting to disabled. When enabled, it SHALL
emit at most one notice for a completed root turn whose canonical misses total
at least 20,000 tokens or whose known derived extra cost is at least $0.10.
Elapsed idle time MAY be displayed as factual context but MUST NOT establish or
claim expiry.

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
