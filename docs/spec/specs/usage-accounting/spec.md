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
and SHALL attach Smith-owned child, compaction, and synthetic-cache purpose
metadata without hiding shared request/attempt identity, timing, finish state,
or usage.

Keepalive, handoff-checkpoint, and automatic idle-compaction attempts SHALL be
separately attributed from user, ordinary parent, goal, child, and provider
retry work. Each synthetic record SHALL expose at least:

```text
purpose
provider
model
cache_identity
input_uncached
input_cached
cache_write
output
reasoning
per-counter provenance
cost and cost provenance
latency_ms
status
```

The stable purpose `cache_keepalive` SHALL remain compatible, and the change
SHALL add `cache_handoff_checkpoint` and `cache_idle_compaction`. Synthetic
usage counts toward provider limits and total session spend but MUST NOT be
presented as user-authored, ordinary parent-turn, or child-turn usage.

#### Scenario: Request succeeds after retry

- **GIVEN** the first provider attempt consumes tokens and fails retryably
- **AND** the second attempt succeeds
- **WHEN** Smith updates session totals
- **THEN** both attempts contribute separately
- **AND** neither is hidden by the successful retry

#### Scenario: Keepalive consumes tokens

- **GIVEN** a synthetic cache keepalive receives a bounded response
- **WHEN** its usage record is written
- **THEN** its purpose is `cache_keepalive`
- **AND** its cost is visible even though request and response are absent from
  canonical conversation history

#### Scenario: Handoff checkpoint uses cached prefix

- **GIVEN** a handoff checkpoint reads 30,000 cached tokens and writes 500
  output tokens
- **WHEN** usage is recorded
- **THEN** those counters are attributed to `cache_handoff_checkpoint`
- **AND** ordinary parent-turn and child-turn usage remain unchanged

#### Scenario: Automatic compaction fails

- **GIVEN** one idle-compaction provider attempt consumes input and fails
- **WHEN** Smith records the no-retry outcome
- **THEN** its usage remains under `cache_idle_compaction`
- **AND** total spend includes the failed attributed attempt once

### Requirement: Labelled cost calculation

Smith SHALL calculate cost only from a versioned price reference and compatible
usage counters. Calculated values MUST be labelled exact, estimated, or unknown
according to their inputs. The catalog snapshot's per-model price entry is one
such reference, and it MUST carry the same revision and retrieval provenance as
every other catalog field. Cost MUST remain presentation only: it MUST NOT
enter routing, approval, context, or budget decisions, and MUST NOT reach the
model.

#### Scenario: Price is unavailable

- **GIVEN** a custom compatible endpoint reports tokens but has no configured
  price
- **WHEN** Smith renders usage
- **THEN** it shows the token counters
- **AND** reports cost as unknown rather than assuming an OpenAI price

#### Scenario: A priced model with reported counters

- **GIVEN** the active model's catalog record prices every counter the session
  accumulated
- **AND** every one of those counters is provider-reported
- **WHEN** Smith renders the session cost
- **THEN** it reports one USD figure labelled exact
- **AND** names the provider and model the price came from

#### Scenario: An estimated counter downgrades the label

- **GIVEN** any contributing counter is tokenizer-estimated,
  character-estimated, derived from a provider total, or unknown
- **WHEN** Smith renders the session cost
- **THEN** the figure is labelled estimated
- **AND** Smith does not present it as exact because the price reference was
  exact

#### Scenario: A model the catalog does not price

- **GIVEN** the active model's catalog record carries no price entry
- **WHEN** Smith renders the exit report
- **THEN** it prints the token lines and no cost line
- **AND** does not substitute a price from another model, provider, or
  hard-coded default

#### Scenario: Cost changes no decision

- **GIVEN** a session with a known price and any accumulated cost
- **WHEN** Smith plans a request, evaluates an approval, or trims context
- **THEN** the computed cost is not an input to any of them

### Requirement: Consistent usage surfaces

Smith SHALL expose Agent Runtime's versioned usage schema through the TUI,
final non-interactive JSON result, streaming JSON events, and embedding
boundary. The TUI MUST show current-turn and session totals with cache and
provenance labels.

#### Scenario: Compare CLI and runtime usage

- **GIVEN** one deterministic session is run through the headless host
- **WHEN** its runtime events and final JSON output are inspected
- **THEN** both expose equivalent counters, provenance, and attribution

### Requirement: Delegated usage is accounted separately

Smith SHALL accumulate per-counter usage reported by delegated children from
the child event streams the host subscribes to, and SHALL keep those counters
distinguishable from the root session's own at every surface that reports them.
It MUST report the number of children that contributed usage, and MUST NOT
present delegated tokens as root tokens or omit them from a session total.

#### Scenario: Four children report usage

- **GIVEN** a session spawns four children and each reports provider usage
- **WHEN** the user quits
- **THEN** the exit report states a merged total across root and children
- **AND** an indented root line and an indented agents line break that total
  down
- **AND** the agents line names how many children contributed

#### Scenario: A session with no delegation

- **GIVEN** a session spawned no children
- **WHEN** the user quits
- **THEN** the report shows the root counters with no agents line and no
  breakdown
- **AND** the merged total equals the root total

#### Scenario: A dormant child reported nothing in this process

- **GIVEN** a resumed session recovers a durable child whose work happened in
  an earlier process
- **WHEN** Smith reports delegated usage
- **THEN** it counts only what this process observed
- **AND** does not invent counters for the recovered child or count it as a
  contributor

#### Scenario: Delegated counters keep their categories

- **GIVEN** a child reports cache-read input, uncached input, and output
- **WHEN** Smith accumulates it into the delegated totals
- **THEN** each counter lands in its own category
- **AND** the delegated totals are priced by the same per-counter reference the
  root totals are

### Requirement: Cache re-billing is derived and non-overlapping

Smith SHALL accumulate canonical missed-cache tokens and miss count as derived
diagnostics separate from Agent Runtime's disjoint usage counters. Re-billed
tokens MUST NOT increase total token usage, enter a `CounterKind`, affect
budgets or routing, or reach the model. Failed provider attempts with canonical
miss evidence SHALL remain included and attributable because their usage was
billed.

#### Scenario: Missed tokens are already uncached input

- **GIVEN** an attempt reports 105,000 uncached input tokens and a canonical
  105,000-token cache miss
- **WHEN** Smith renders session usage
- **THEN** total input includes the 105,000 tokens exactly once
- **AND** cache re-billed reports 105,000 as a separate derived diagnostic

#### Scenario: Failed retry paid for a miss

- **GIVEN** a failed retry attempt and a later successful attempt both carry
  canonical miss evidence
- **WHEN** Smith computes the turn and session cache diagnostics
- **THEN** both attempts contribute to re-billed tokens and miss count
- **AND** each attempt's ordinary usage remains separately attributable

### Requirement: Cache re-billing cost preserves provenance

Smith SHALL derive extra cache-miss cost from the missed-token count, the
attempt's paid uncached/write mix, and the cache-read rate when a compatible
versioned price is available. The value MUST be labelled derived or estimated
according to all inputs. If any required rate, attempt attribution, or
expectation confidence is unavailable, cost MUST remain unknown while token
diagnostics remain visible.

#### Scenario: All price inputs are available

- **GIVEN** a canonical miss has attributed paid input/write counters
- **AND** the serving model's versioned price supplies compatible uncached,
  cache-write, and cache-read rates
- **WHEN** Smith computes extra miss cost
- **THEN** it reports the non-negative difference from the cache-read cost
- **AND** labels the result derived with the price and expectation provenance

#### Scenario: Cache-read price is unavailable

- **GIVEN** a canonical miss has a known derived token count
- **AND** the serving model has no cache-read price
- **WHEN** Smith renders the miss and session summary
- **THEN** it reports re-billed tokens without a dollar value
- **AND** it does not substitute another model's price or treat unknown as zero

### Requirement: Cache estimates are never actual usage

Smith SHALL keep cache estimates separate from actual usage. It MAY calculate
retention timing, predicted refill cost, or estimated savings for scheduling,
but those estimates MUST retain explicit derived or
estimated provenance. They MUST NOT be merged into provider-reported usage,
presented as verified cache reads/writes, used to fabricate a hit or expiry, or
reported as realized savings.

#### Scenario: Provider reports no cache counters

- **GIVEN** an eligible request to a provider without cache usage fields
- **WHEN** Smith renders accounting
- **THEN** cache-read and cache-write usage remain unknown
- **AND** no scheduling estimate is presented as provider-reported usage

#### Scenario: Scheduler estimates a warm refill saving

- **GIVEN** policy estimates that maintaining a prefix could avoid a future
  refill
- **WHEN** no future continuation or provider observation has occurred
- **THEN** the value remains a planning estimate
- **AND** session accounting reports no realized saving or cache hit

### Requirement: Synthetic cache budgets use actual attributed attempts

Smith SHALL enforce maintenance call, exact input/output, deadline, provider,
and session limits against the actual attributed synthetic attempts it dispatches,
including failed or cancelled attempts when the provider accepted work.
Provider-reported usage and cost remain recorded after an accepted attempt,
but calculated price or cost MUST NOT be a dispatch budget. Suppressed requests
that issue no provider I/O SHALL consume no provider tokens or cost but SHALL
retain a separate lifecycle disposition.

#### Scenario: Scheduled keepalive is suppressed locally

- **GIVEN** a real parent request makes a scheduled keepalive unnecessary
- **WHEN** Smith suppresses it before transport dispatch
- **THEN** maintenance call usage and provider spend remain unchanged
- **AND** the suppression event records the reason

#### Scenario: Provider accepts an attempt before cancellation

- **GIVEN** a synthetic request is dispatched and later cancelled during
  shutdown
- **WHEN** provider usage is available for the partial attempt
- **THEN** Smith counts and attributes that usage
- **AND** does not erase it because no summary or pong committed

### Requirement: Cost presentation does not authorize dispatch

Smith SHALL retain calculated or estimated price/cost with explicit
presentation provenance. Dispatch eligibility, host authority, suppression,
and maintenance limits MUST NOT read calculated price/cost; effective
synthetic dispatch instead requires the explicit host spend authority and the
ordinary call, exact input/output, deadline, provider, and session limits.

#### Scenario: Cost estimate cannot authorize or suppress maintenance

- **GIVEN** two otherwise identical maintenance plans have different
  calculated cost estimates
- **WHEN** Smith evaluates dispatch
- **THEN** the estimates do not change eligibility, authority, or ordinary
  maintenance limits
- **AND** any accepted attempt records actual provider usage/cost separately
  from the presentation estimate
