## ADDED Requirements

### Requirement: Normalized provider rate-limit snapshots

Direct provider adapters SHALL parse provider-reported rate-limit and usage
headers into one normalized, redaction-safe snapshot observation carrying, per
reported window: used percentage, window duration when reported, reset time
when reported, and the provider's limit identifier when reported. Snapshots
are server-reported only: an adapter MUST NOT estimate or fabricate usage, and
absent data MUST surface as absent rather than zero. Snapshots flow through
the versioned runtime event/observation contract without exposing credential
material.

#### Scenario: Response carries rate-limit headers

- **GIVEN** a provider response reports a primary window at 82% used with a
  reset timestamp
- **WHEN** the adapter completes the attempt
- **THEN** a normalized snapshot observation records 82%, the window, and the
  reset time for the active credential
- **AND** the snapshot contains no authorization or header secret material

#### Scenario: Provider reports no usage headers

- **GIVEN** a provider response carries no recognized rate-limit headers
- **WHEN** the adapter completes the attempt
- **THEN** no snapshot is fabricated
- **AND** any usage surface continues to show the member's state as unknown

### Requirement: Typed limit-exhaustion classification

The shared transport/adapter error classification SHALL distinguish a
usage-limit exhaustion rejection from other rate or authentication failures as
a distinct typed error carrying the server-reported reset time when present.
A transient throttle that the existing retry discipline may safely retry MUST
NOT be classified as exhaustion.

#### Scenario: Usage limit rejection carries reset time

- **GIVEN** the provider rejects an attempt because the account's usage window
  is exhausted and reports when it resets
- **WHEN** the adapter classifies the failure
- **THEN** the attempt fails with the typed limit-exhaustion error including
  that reset time
- **AND** the classification is redaction-safe in events and journals

#### Scenario: Transient throttle is not exhaustion

- **GIVEN** the provider returns a momentary throttle with a short retry hint
- **WHEN** the adapter classifies the failure
- **THEN** the failure keeps its existing transient classification
- **AND** no limit-exhaustion handling or credential rotation is triggered

### Requirement: Usage-aware credential rotation

Smith's runtime SHALL offer rotation to the pool members not in cooldown,
rather than switching silently, when the active member fails with the typed
limit-exhaustion error before a response stream is accepted — because rotation
abandons the provider-side prompt cache and resubmits the whole context
uncached. On confirmation the runtime SHALL replay the attempt with the chosen
member, at most once per remaining eligible member within a single
user-visible attempt; on refusal, or on any surface that cannot answer, the
turn SHALL fail with the typed error. An exhausted member SHALL enter cooldown
until its reported reset time, or a bounded default when none was reported.
The runtime MUST NOT rotate or replay after a response stream has been
accepted, and the active member selection SHALL persist across sessions until
rotation or manual switch changes it.

#### Scenario: Exhaustion offers the next account

- **GIVEN** a two-member pool whose active member is exhausted
- **WHEN** an attempt fails with the typed limit-exhaustion error before any
  stream is accepted
- **THEN** the runtime places that member in cooldown until its reset time
- **AND** raises a rotation prompt naming the outgoing member, its reset time,
  and each eligible member with its usage meter
- **AND** the prompt states that switching resubmits the turn without the
  provider-side prompt cache

#### Scenario: Confirmed rotation replays the attempt

- **GIVEN** a rotation prompt offering the second member
- **WHEN** the user confirms the switch
- **THEN** the runtime replays the attempt once with the second member
- **AND** records a redaction-safe rotation event naming members by pool
  position or display name, never by credential value

#### Scenario: Declined rotation fails the turn

- **GIVEN** a rotation prompt offering the second member
- **WHEN** the user declines the switch
- **THEN** the turn fails with the typed limit-exhaustion error and the
  outgoing member's reset time
- **AND** the active member is unchanged
- **AND** the second member's usage is not spent

#### Scenario: All members exhausted

- **GIVEN** every pool member is exhausted or in cooldown
- **WHEN** an attempt fails with the typed limit-exhaustion error
- **THEN** the turn fails with a typed error reporting the earliest reset time
  across the pool
- **AND** no rotation prompt is raised, because no member could serve it
- **AND** no unbounded rotation loop occurs

#### Scenario: Proactive threshold offers rotation before exhaustion

- **GIVEN** a pool configured to rotate at 90% used, whose active member's
  latest snapshot reports 93%
- **WHEN** the next turn begins
- **THEN** the runtime offers rotation to an eligible member before spending
  the attempt
- **AND** declining keeps the active member and does not re-ask within the
  same turn

#### Scenario: Cooldown prevents reselecting an exhausted member

- **GIVEN** a member entered cooldown five minutes ago with a reset one hour
  away
- **WHEN** rotation looks for the next eligible member
- **THEN** the cooling member is skipped
- **AND** it becomes eligible again after its reset time passes

#### Scenario: No rotation after an accepted stream

- **GIVEN** a provider accepted a response stream that later reports limit
  exhaustion mid-stream
- **WHEN** the attempt ends
- **THEN** the runtime surfaces the failure without replaying with another
  member
- **AND** the exhausted member still enters cooldown for subsequent attempts

#### Scenario: Sticky member survives restart

- **GIVEN** rotation moved the pool to its second member yesterday
- **WHEN** Smith starts a new session for the same provider
- **THEN** the second member is the active member without re-testing the first
