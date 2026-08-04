## ADDED Requirements

### Requirement: Provider-declared cache capability

Each shared provider adapter SHALL declare caching as unsupported,
automatic-prefix, explicit-breakpoint, or explicit-resource and MUST describe
observable cache-read/write fields, supported retention choices, refresh
behavior, and synthetic-keepalive safety. Smith MUST map configuration to that
shared capability and MUST NOT apply one provider's TTL assumptions to another.

#### Scenario: Provider exposes no cache evidence

- **GIVEN** an OpenAI-compatible endpoint declares automatic caching but no
  usage fields or cache API
- **WHEN** Smith sends a matching request
- **THEN** cache status remains `unknown`
- **AND** Smith does not report a verified hit from elapsed time alone

### Requirement: Exact cache identity

Smith SHALL use Agent Runtime's context/cache plan fingerprint for the exact
stable prefix, provider, model, tool schemas, system content, adapter/sizer
revisions, and cache controls, adding only Smith-owned endpoint identity where
the shared plan cannot represent it. Any change to those inputs MUST create a
distinct cache identity.

#### Scenario: Tool schema changes

- **GIVEN** a session has an observed warm cache
- **WHEN** an extension changes an enabled tool schema
- **THEN** Smith marks the prior identity inapplicable
- **AND** does not attribute future cache usage to the old identity

### Requirement: Evidence-based cache status

Smith SHALL derive at least `unsupported`, `unknown`, `eligible`,
`warm_observed`, `miss_observed`, and `suspended`. Only provider-reported usage
or a provider cache API exposed through shared events MAY establish
`warm_observed` or `miss_observed`.

#### Scenario: Cache read tokens are reported

- **GIVEN** a provider response reports a positive cache-read count
- **WHEN** the adapter normalizes usage
- **THEN** Smith records `warm_observed` for the matching identity
- **AND** attributes the cache-read tokens to that request

### Requirement: Configurable meaningful inactivity

Smith SHALL target cache retention for a configurable inactivity duration,
defaulting to one hour. The clock MUST reset on a real user message or actual
agent/provider/tool activity. Synthetic keepalives and passive monitor delivery
MUST NOT reset it; agent or tool work caused by a notification MUST reset it.

#### Scenario: Tool work extends the active window

- **GIVEN** 55 minutes have passed since the user's last message
- **WHEN** the active agent runs a real tool call
- **THEN** the session's inactivity clock restarts from that activity

#### Scenario: Passive monitor does not pin cache forever

- **GIVEN** a monitor emits status lines but no agent turn or tool work follows
- **WHEN** one hour passes since meaningful activity
- **THEN** Smith considers the session inactive despite those lines

### Requirement: Adapter-gated ephemeral keepalive

When enabled and supported, Smith SHALL send the exact stable cache anchor plus
a minimal ephemeral ping and bound the expected pong response. The adapter MUST
select a provider-appropriate interval with jitter and suppress a scheduled
ping when recent real activity already refreshed the prefix. Ping and pong MUST
be excluded from the canonical transcript, while their usage and cache
observation MUST be recorded.

#### Scenario: Keepalive hits the old prefix

- **GIVEN** the adapter declares a safe keepalive strategy and the inactivity
  limit has not been reached
- **WHEN** the scheduled keepalive reports cache-read usage
- **THEN** Smith records the observed hit and keepalive tokens
- **AND** future canonical requests contain neither ping nor pong

#### Scenario: Automatic-prefix behavior is unproven

- **GIVEN** an automatic-prefix adapter has not passed conformance showing that
  its anchor remains reusable after ephemeral suffix removal
- **WHEN** keepalive policy is evaluated
- **THEN** Smith remains observation-only and sends no synthetic ping

### Requirement: No additional rebuild after miss

After an observed keepalive miss, Smith MUST record `miss_observed`, suspend
further keepalives for that identity, and MUST NOT send a second prewarm or
rebuild request. A later real user/agent turn MAY naturally create a new cache.

#### Scenario: Keepalive misses

- **GIVEN** a synthetic keepalive reports no cache read and reports uncached or
  cache-write input
- **WHEN** Smith updates cache state
- **THEN** the identity becomes miss-observed or suspended
- **AND** no follow-up cache-only request is sent

### Requirement: Idle-limit automatic compaction

At the configured inactivity limit, Smith SHALL wait for a safe boundary and
invoke Agent Runtime's configured semantic compactor once, persist the resulting
shared summary/manifest/usage, stop keeping the old identity alive, and MUST NOT
prewarm the compacted prefix.

#### Scenario: One-hour inactivity expires

- **GIVEN** the default one-hour limit is reached with no in-flight turn
- **WHEN** Smith processes the idle transition
- **THEN** it compacts once and persists the canonical summary
- **AND** stops old-prefix keepalives
- **AND** waits for a real turn before any new-prefix cache creation

#### Scenario: Limit expires during a tool call

- **GIVEN** the inactivity deadline becomes due while a tool is executing
- **WHEN** the tool has not reached a safe boundary
- **THEN** Smith queues compaction without interrupting the tool
- **AND** runs it once after the boundary

#### Scenario: Automatic compaction fails

- **GIVEN** the provider fails the scheduled compaction
- **WHEN** the attempt reaches its retry/limit policy
- **THEN** Smith records a visible compaction failure and its usage
- **AND** does not enter an automatic retry loop or continue pinging forever
