# prompt-cache Specification

## Purpose
TBD - created by archiving change add-smith-agent-harness. Update Purpose after archive.
## Requirements
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

### Requirement: Independently revisioned project-instruction fragment

Smith SHALL contribute a present project-instruction snapshot as one required,
provenance-bearing developer-instruction fragment whose revision derives from
its exact source identity and content. It MUST remain distinct from Smith's
stable product-policy sections, optional retrieval-style project context,
canonical conversation history, and executable skill activation. Exact prompt
and cache identity MUST change when a newly constructed runtime captures
different project instructions.

#### Scenario: The same snapshot is reused by a child

- **GIVEN** a root runtime and direct child use the same captured project
  instruction snapshot
- **WHEN** Smith constructs their prompt and child-policy fingerprints
- **THEN** both identify the same project-instruction revision
- **AND** child construction performs no second filesystem read

#### Scenario: Instructions change before a later runtime

- **GIVEN** one runtime was built from project-instruction revision A
- **WHEN** a later runtime captures changed content as revision B
- **THEN** the project fragment and exact full prompt/cache identity differ
- **AND** unchanged Smith product fragments retain their own prior revisions
- **AND** Smith does not report the old exact cache identity as applicable

#### Scenario: File changes without runtime reconstruction

- **GIVEN** an active runtime has already planned with a captured project
  instruction revision
- **WHEN** the underlying file changes
- **THEN** the runtime's fragments and cache identity remain unchanged
- **AND** no provider request is sent merely because of the filesystem change

#### Scenario: Complete host prompt override is supplied

- **GIVEN** a direct embedder supplies Smith's complete system-prompt override
- **WHEN** the factory composes the runtime
- **THEN** the override retains its existing complete-replacement semantics
- **AND** Smith does not append an implicit project-instruction fragment

### Requirement: Exact agent profile prompt identity

Smith SHALL derive a deterministic revision for the effective agent-profile
fragment from its resolved behavior, instructions, placement, and source
identity, and SHALL include that revision in root prompt plans and child policy
fingerprints. Smith MUST keep stable host, project-instruction, skill, memory,
and profile revisions independently attributable.

#### Scenario: Reuse an unchanged profile
- **GIVEN** two equivalent compositions resolve the same effective profile and
  instruction bytes
- **WHEN** Smith plans their prompt and policy identities
- **THEN** the profile fragment has the same exact revision
- **AND** unrelated stable Smith fragments retain their own revisions

#### Scenario: Profile instructions change
- **GIVEN** a newly constructed runtime resolves changed profile instructions
- **WHEN** Smith plans provider context
- **THEN** the profile revision and exact full prompt identity change
- **AND** Smith does not claim reuse under the prior exact identity

#### Scenario: Debug profile identity
- **GIVEN** a profile contains private or sensitive instruction text
- **WHEN** status, debug, journal, or compatibility diagnostics render it
- **THEN** they show bounded name, revision, placement, and provenance only
- **AND** do not copy the raw instruction body into canonical user history
