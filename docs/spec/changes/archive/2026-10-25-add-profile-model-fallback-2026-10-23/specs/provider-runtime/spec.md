## ADDED Requirements

### Requirement: Bounded request-level model fallback

The shared Agent Runtime SHALL select and replan an ordered fallback candidate
for ordinary provider-backed main and child requests on typed quota exhaustion,
exhausted transient network/timeout/server/rate-limit failures, or the configured
first-response deadline. Each binding SHALL be selected at most once per user
turn; the successful candidate SHALL serve subsequent steps in that turn.
Fallback MUST preserve original turn limits and cancellation, MUST NOT replay
completed tools, and MUST NOT advance after meaningful response progress or
when request replay is unsafe. New user turns SHALL start with the configured
primary. No fallback SHALL be inferred for unconfigured auxiliary/synthetic work.

#### Scenario: Transient retries precede failure fallback

- **GIVEN** the primary fails with transient server errors before response progress
- **WHEN** its retry allowance is exhausted while the turn still has time
- **THEN** the shared runtime activates the next compatible candidate and replans the pending request
- **AND** all failed attempts remain observable with their actual bindings

#### Scenario: Cancellation or hard failure

- **WHEN** the user cancels, the turn deadline expires, or the candidate fails with authentication rejection or a generic bad request
- **THEN** the runtime preserves that terminal outcome without dispatching another candidate

#### Scenario: Continuation preserves completed tool work

- **GIVEN** a tool completed and its canonical result is already in history
- **WHEN** the next model request fails safely before response progress
- **THEN** the next candidate receives a replanned continuation including that result
- **AND** the completed tool is not run again

#### Scenario: No candidate can serve the request

- **WHEN** every configured candidate fails or is incompatible with the current request
- **THEN** the turn fails once with a redaction-safe summary of attempted/skipped bindings and reasons
- **AND** it does not cycle back to an earlier candidate or reset the turn budgets

### Requirement: First-response timeout uses a safe progress fence

The shared runtime SHALL measure the first-response budget across candidate
dispatch, connection/open time, retry/backoff, and waiting on an empty stream.
Nonempty text, meaningful reasoning, tool-call or hosted-tool activity, and
successful completion SHALL end the startup window. Headers, heartbeats,
empty deltas, and usage-only events MUST NOT end or restart it. On expiry the
runtime SHALL cancel and retire the old attempt before advancing, fencing late
events by attempt identity. After stream acceptance, replay SHALL additionally
require an adapter contract proving pre-output replay safe. Requests allowing
provider-hosted side effects SHALL require proof those effects cannot already
have started or can be safely deduplicated after possible dispatch, even
before stream creation. Without the required proof, timeout SHALL fail
without replay.

#### Scenario: Provider never opens its stream

- **WHEN** a candidate cannot open a stream before its configured first-response deadline
- **THEN** its pending attempt is cancelled and recorded with reason `first_response_timeout`
- **AND** the next candidate starts only after local cancellation and retirement

#### Scenario: Headers and heartbeats without content

- **GIVEN** the adapter declares pre-output replay safe and returns an empty stream or heartbeat-only traffic
- **WHEN** the first-response deadline expires
- **THEN** the runtime retires that stream and advances to the next candidate
- **AND** late output from the retired attempt never enters the conversation or new attempt

#### Scenario: Retry does not restart the clock

- **GIVEN** a 30-second candidate budget has already consumed 20 seconds in an attempt and backoff
- **WHEN** a retry starts
- **THEN** it has at most the remaining 10 seconds to begin a meaningful response
- **AND** expiry advances directly instead of exhausting further same-model retries

#### Scenario: Response begins before the cutoff

- **WHEN** meaningful text, reasoning, or tool activity begins before the startup deadline
- **THEN** the first-response timer is disarmed for that request
- **AND** a later stall does not automatically switch models

#### Scenario: Hosted side-effect safety is unknown

- **GIVEN** a dispatched request may conceal provider-hosted side effects and the adapter cannot prove replay safe, whether or not a stream has opened
- **WHEN** its first-response timer expires before any progress is reported
- **THEN** the runtime cancels locally and reports the timeout without dispatching a fallback

### Requirement: Candidate activation replans model-bound state

Before provider I/O on a fallback, the shared planner SHALL apply the actual
candidate's model limits, reasoning, input modalities, output and context
budgets, tools, and cache identity. Activation SHALL preserve canonical history,
profile instructions, host authority, posture, approvals, and tool results.
Provider-private response handles and incompatible cache/continuation state
MUST NOT cross bindings. Reported usage and prices SHALL remain attributed to
the binding that produced them; absent timed-out usage SHALL remain unknown.

#### Scenario: Smaller fallback window

- **GIVEN** the next candidate has a smaller context window
- **WHEN** it is activated
- **THEN** normal context planning and compaction run against that window before sending
- **AND** a candidate that cannot fit protected history is skipped with a diagnostic

#### Scenario: Candidate cannot accept current input

- **GIVEN** the pending request requires images or model-facing tools that a candidate cannot represent under the existing policy
- **WHEN** fallback prepares that candidate
- **THEN** it is skipped with an incompatibility reason
- **AND** protected history and required input are not silently removed

#### Scenario: Cross-provider activation preserves read-only policy

- **GIVEN** a plan-posture profile advances to another provider
- **WHEN** its request is rebuilt
- **THEN** the same read-only host authority and profile instructions apply
- **AND** credentials, provider handles, and cache baselines from the old binding are not reused for the new binding

#### Scenario: Timed-out attempt has unknown usage

- **GIVEN** the first provider times out without reporting usage and the fallback succeeds with reported usage
- **WHEN** the runtime records and prices the turn
- **THEN** both attempts retain distinct provider/model identities
- **AND** unreported usage is unknown and is never priced using the fallback model's rates

## MODIFIED Requirements

### Requirement: Usage-aware credential rotation

Smith's runtime SHALL offer rotation to the pool members not in cooldown,
rather than switching silently, when the active member fails with the typed
limit-exhaustion error before a response stream is accepted, because rotation
abandons the provider-side prompt cache and resubmits the whole context
uncached. On confirmation the runtime SHALL replay the attempt with the chosen
member, at most once per remaining eligible member within a single
user-visible attempt; on refusal, or on any surface that cannot answer, the
turn SHALL fail with the typed error. An exhausted member SHALL enter cooldown
until its reported reset time, or a bounded default when none was reported.
The runtime MUST NOT rotate or replay after a response stream has been
accepted, and the active member selection SHALL persist across sessions until
rotation or manual switch changes it.

For an ordinary request with an enabled profile model-fallback chain, reactive
quota rotation SHALL instead defer to the shared model-fallback policy after
recording the exhausted member's cooldown. It MUST NOT offer or silently spend
another pool member before that policy runs. This exception SHALL NOT change
manual rotation, proactive threshold offers, or requests without a chain.

#### Scenario: Exhaustion offers the next account

- **GIVEN** a two-member pool whose active member is exhausted and no enabled model-fallback chain
- **WHEN** an attempt fails with the typed limit-exhaustion error before any stream is accepted
- **THEN** the runtime places that member in cooldown until its reset time
- **AND** raises a rotation prompt naming the outgoing member, its reset time, and each eligible member with its usage meter
- **AND** the prompt states that switching resubmits the turn without the provider-side prompt cache

#### Scenario: Confirmed rotation replays the attempt

- **GIVEN** a rotation prompt offering the second member
- **WHEN** the user confirms the switch
- **THEN** the runtime replays the attempt once with the second member
- **AND** records a redaction-safe rotation event naming members by pool position or display name, never by credential value

#### Scenario: Declined rotation fails the turn

- **GIVEN** a rotation prompt offering the second member
- **WHEN** the user declines the switch
- **THEN** the turn fails with the typed limit-exhaustion error and the outgoing member's reset time
- **AND** the active member is unchanged
- **AND** the second member's usage is not spent

#### Scenario: All members exhausted

- **GIVEN** every pool member is exhausted or in cooldown and no enabled model-fallback chain
- **WHEN** an attempt fails with the typed limit-exhaustion error
- **THEN** the turn fails with a typed error reporting the earliest reset time across the pool
- **AND** no rotation prompt is raised, because no member could serve it
- **AND** no unbounded rotation loop occurs

#### Scenario: Proactive threshold offers rotation before exhaustion

- **GIVEN** a pool configured to rotate at 90% used, whose active member's latest snapshot reports 93%
- **WHEN** the next turn begins
- **THEN** the runtime offers rotation to an eligible member before spending the attempt
- **AND** declining keeps the active member and does not re-ask within the same turn

#### Scenario: Cooldown prevents reselecting an exhausted member

- **GIVEN** a member entered cooldown five minutes ago with a reset one hour away
- **WHEN** rotation looks for the next eligible member
- **THEN** the cooling member is skipped
- **AND** it becomes eligible again after its reset time passes

#### Scenario: No rotation after an accepted stream

- **GIVEN** a provider accepted a response stream that later reports limit exhaustion mid-stream
- **WHEN** the attempt ends
- **THEN** the runtime surfaces the failure without replaying with another member
- **AND** the exhausted member still enters cooldown for subsequent attempts

#### Scenario: Sticky member survives restart

- **GIVEN** rotation moved the pool to its second member yesterday
- **WHEN** Smith starts a new session for the same provider
- **THEN** the second member is the active member without re-testing the first

#### Scenario: Configured model chain takes precedence on quota exhaustion

- **GIVEN** the current ordinary request has an enabled profile model-fallback chain
- **WHEN** its active credential returns typed limit exhaustion
- **THEN** that member enters cooldown and the error reaches the shared fallback controller
- **AND** no reactive credential-rotation prompt or extra pool-member attempt occurs
- **AND** model fallback remains subject to its response-progress and replay-safety fences
