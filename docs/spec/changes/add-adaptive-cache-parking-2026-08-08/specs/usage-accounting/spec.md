## MODIFIED Requirements

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

## ADDED Requirements

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
