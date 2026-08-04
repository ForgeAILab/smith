# usage-accounting Specification

## Purpose
TBD - created by archiving change add-smith-agent-harness. Update Purpose after archive.
## Requirements
### Requirement: Disjoint usage counters

Smith SHALL preserve Agent Runtime's disjoint counters for uncached input,
cache-read input, cache-write input, visible output, reasoning output, and other
output. It MUST NOT reinterpret the shared categories or count cached input
again as uncached input.

#### Scenario: Provider reports cache use

- **GIVEN** a response reports 8,000 cache-read input tokens, 500 uncached input
  tokens, and 300 output tokens
- **WHEN** the shared provider/runtime normalizes usage
- **THEN** Smith stores and displays those categories unchanged
- **AND** the total rollup can be derived without double counting

### Requirement: Per-counter accuracy provenance

Every shared usage counter MUST retain its label `provider_reported`,
`derived_from_provider_total`, `tokenizer_estimated`,
`character_estimated`, or `unknown`. Smith MUST NOT display `unknown` as zero or
remove the distinction during aggregation.

#### Scenario: Stream ends without final usage

- **GIVEN** a provider stream is cancelled before final usage arrives
- **WHEN** the shared runtime can estimate tokens from captured content
- **THEN** Smith preserves the tokenizer or character provenance
- **AND** the UI and machine output label it estimated

#### Scenario: No estimate is available

- **GIVEN** a provider attempt exposes neither usage nor countable content
- **WHEN** Smith records the attempt
- **THEN** the affected counters remain unknown rather than zero

### Requirement: Attempt and purpose attribution

Smith MUST preserve every shared provider attempt, including failed retries,
and SHALL attach Smith-owned child, compaction, or synthetic-keepalive purpose
metadata without hiding shared request/attempt identity, timing, finish state,
or usage.

#### Scenario: Request succeeds after retry

- **GIVEN** the first provider attempt consumes tokens and fails retryably
- **AND** the second attempt succeeds
- **WHEN** Smith updates session totals
- **THEN** both attempts contribute separately
- **AND** neither is hidden by the successful retry

#### Scenario: Keepalive consumes tokens

- **GIVEN** a synthetic cache keepalive receives a pong
- **WHEN** the usage record is written
- **THEN** its purpose is `cache_keepalive`
- **AND** its cost is visible even though ping/pong are absent from the
  transcript

### Requirement: Labelled cost calculation

Smith SHALL calculate cost only from a versioned price reference and compatible
usage counters. Calculated values MUST be labelled exact, estimated, or unknown
according to their inputs.

#### Scenario: Price is unavailable

- **GIVEN** a custom compatible endpoint reports tokens but has no configured
  price
- **WHEN** Smith renders usage
- **THEN** it shows the token counters
- **AND** reports cost as unknown rather than assuming an OpenAI price

### Requirement: Consistent usage surfaces

Smith SHALL expose Agent Runtime's versioned usage schema through the TUI,
final non-interactive JSON result, streaming JSON events, and embedding
boundary. The TUI MUST show current-turn and session totals with cache and
provenance labels.

#### Scenario: Compare CLI and runtime usage

- **GIVEN** one deterministic session is run through the headless host
- **WHEN** its runtime events and final JSON output are inspected
- **THEN** both expose equivalent counters, provenance, and attribution
