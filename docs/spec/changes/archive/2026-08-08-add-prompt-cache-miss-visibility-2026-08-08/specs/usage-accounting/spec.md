## ADDED Requirements

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
