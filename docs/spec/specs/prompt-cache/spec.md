# prompt-cache Specification

## Purpose
TBD - created by archiving change add-smith-agent-harness. Update Purpose after archive.
## Requirements
### Requirement: Provider-declared cache capability

Each shared provider adapter SHALL declare a cache contract for every served
model. The contract MUST distinguish `unsupported`, `implicit-prefix`,
`explicit-breakpoint`, and `explicit-resource` behavior and SHALL include, when
applicable:

- whether stable-prefix reuse and independent ephemeral segments are supported;
- whether the provider accepts a stable cache key;
- maximum cache breakpoints;
- supported retention choices or explicit expiry metadata;
- whether a cache hit refreshes retention;
- whether reads and writes are observable;
- whether explicit create, extend, inspect, and delete operations exist;
- whether synthetic suffix removal has passed adapter conformance; and
- whether keepalive and handoff-checkpoint requests are safe.

Smith MUST map configuration to that shared capability and MUST NOT apply
retention, refresh, observation, or maintenance assumptions from one provider
adapter to another. The previous `automatic-prefix` spelling MAY be accepted
only as a bounded compatibility alias for `implicit-prefix` and MUST NOT carry
different semantics.

#### Scenario: Provider exposes no cache evidence

- **GIVEN** an OpenAI-compatible endpoint declares implicit prefix caching
- **AND** the adapter has no verified retention, observation, or suffix-removal
  contract
- **WHEN** Smith evaluates cache state and maintenance
- **THEN** it may report the matching prefix as structurally eligible
- **AND** provider cache status remains `unknown`
- **AND** Smith sends no synthetic keepalive or handoff checkpoint

#### Scenario: Explicit cache resource exposes typed expiry

- **GIVEN** an adapter exposes an explicit cache resource with an expiry
  timestamp
- **WHEN** Smith creates or inspects that resource and receives a typed
  upstream resource/expiry event
- **THEN** the matching lease records the provider-reported expiry
- **AND** Smith may schedule maintenance only within that declared contract

#### Scenario: Legacy automatic-prefix declaration loads

- **GIVEN** a compatible persisted model profile uses `automatic-prefix`
- **WHEN** Smith resolves it during the bounded migration window
- **THEN** it normalizes to `implicit-prefix` with the same behavior
- **AND** explain output reports the compatibility alias

### Requirement: Exact cache identity

Smith SHALL consume Agent Runtime's opaque context/cache plan identity for one
exact cache identity, adding only a Smith-owned endpoint partition when the
shared plan cannot represent it. Smith MUST NOT recompute an independent cache
fingerprint. The upstream identity MUST cover every input that can alter prefix
reuse, including:

- provider and endpoint identity;
- model and model-profile fingerprint;
- provider request-adapter and tokenizer revisions;
- provider cache-control contract, cache-key value/strategy, breakpoint
  identity, and explicit-resource identity/handle;
- stable system, profile, project-instruction, skill, and memory fragments;
- advertised tool names, descriptions, schemas, and ordering;
- registry snapshot, scoped view, and activation epoch; and
- ordered stable-history segment identities and content hashes.

A change to any identity input MUST create a different cache lease. Smith MUST
NOT transfer a warm, eligible, guaranteed, or suspended state from the prior
identity to the new identity.

#### Scenario: Tool schema changes

- **GIVEN** cache identity A has an observed hit
- **WHEN** an extension changes an advertised tool schema or ordering
- **THEN** the next context plan has cache identity B
- **AND** identity A remains historical only
- **AND** identity B begins with no provider-warmth claim

#### Scenario: Same session resumes on another model

- **GIVEN** a session persists canonical history and a previous cache plan
- **WHEN** it resumes using another model, endpoint, or request adapter
- **THEN** Smith restores compatible canonical state
- **BUT** treats the prior cache lease as inapplicable
- **AND** sends no request merely to establish the new identity

#### Scenario: Stable fragment revision changes

- **GIVEN** a later runtime resolves changed project instructions, profile
  instructions, skill content, or memory content
- **WHEN** it constructs the next exact plan
- **THEN** the changed fragment revision creates another cache identity
- **AND** unchanged fragment revisions remain independently attributable

#### Scenario: Cache key or resource changes

- **GIVEN** identity A uses a provider cache key, breakpoint, or explicit
  resource handle
- **WHEN** the upstream plan resolves a different value for any of them
- **THEN** Smith consumes a new opaque identity B
- **AND** A's warmth, guarantee, and maintenance budget do not transfer

### Requirement: Evidence-based cache status

Smith SHALL project Agent Runtime's attributed cache evidence into an
evidence-bearing lease containing at least:

```text
identity
status
guaranteed_until
last_cache_touch_at
last_hit_at
last_write_at
last_miss_at
last_meaningful_activity_at
parked_interval_id
maintenance_calls
maintenance_input_tokens
maintenance_output_tokens
maintenance_cost
suspension_reason
```

`status` SHALL support at least `unsupported`, `unknown`, `eligible`,
`warm_observed`, `miss_observed`, `expired_observed`, and `suspended`. Only
canonical provider-reported usage or cache evidence MAY establish
`warm_observed` or `miss_observed`. `expired_observed` requires a typed
upstream resource/expiry event or typed provider error correlated through the
canonical shared event contract; elapsed time and omitted evidence never
establish it.

Smith MUST preserve the difference between an explicit zero and omitted cache
evidence, MUST correlate state by request, attempt, and exact cache identity,
and MUST NOT treat a first eligible request, a request with no comparable
predecessor, changed identity, omitted evidence, or elapsed time as a miss.
Omitted evidence remains `unknown`; `eligible` is reserved for a first request
or a request with no comparable predecessor. This requirement composes with the
canonical missed-token evidence
introduced by `add-prompt-cache-miss-visibility-2026-08-08`; Smith MUST NOT
derive a second conflicting miss value.

#### Scenario: Cache read tokens are reported

- **GIVEN** the current provider attempt matches cache identity A
- **WHEN** the provider reports a positive cache-read count
- **THEN** A becomes `warm_observed`
- **AND** `last_hit_at` is updated
- **AND** the attempt receives the reported read-token attribution
- **AND** provider-declared sliding retention is updated only according to the
  adapter contract

#### Scenario: Explicit zero follows a reusable plan

- **GIVEN** Agent Runtime reports a positive expected cache read for a
  comparable exact plan
- **AND** the provider explicitly reports zero cache-read tokens
- **WHEN** Smith reduces the canonical state event
- **THEN** it records `miss_observed` and the canonical missed-token count
- **AND** it does not reinterpret the zero as omitted data

#### Scenario: Cache evidence is omitted

- **GIVEN** provider caching is supported
- **AND** the completed response contains no cache observation
- **WHEN** Smith updates the lease
- **THEN** status is `unknown` rather than `miss_observed`
- **AND** Smith fabricates neither a zero read nor re-billed tokens

#### Scenario: First eligible request reports zero

- **GIVEN** a cache-capable first request has no comparable predecessor
- **AND** the provider explicitly reports zero
- **WHEN** Smith updates cache status
- **THEN** it remains `eligible`
- **AND** no miss notice, maintenance suspension, or re-billed total is added

#### Scenario: Guaranteed minimum retention passes

- **GIVEN** identity A has `guaranteed_until = T`
- **WHEN** the clock passes T without another provider observation
- **THEN** Smith no longer presents the cache as guaranteed
- **BUT** MUST NOT claim that the cache missed or expired
- **AND** the last observed evidence remains available for diagnostics

#### Scenario: Typed resource event reports deletion

- **GIVEN** identity A refers to an explicit provider cache resource
- **WHEN** a typed upstream resource event reports that the resource expired or
  was deleted
- **THEN** A becomes `expired_observed`
- **AND** no elapsed-time inference is required

#### Scenario: Provider or model identity changes

- **GIVEN** the next request resolves a different exact cache identity
- **WHEN** Agent Runtime reports no reusable expectation from the prior plan
- **THEN** Smith clears the prior identity's current hit indicator
- **AND** it does not count the non-transferable prefix as a cache miss

### Requirement: Configurable meaningful inactivity

Smith SHALL track the parent session's `last_meaningful_activity_at`, each
cache identity's `last_cache_touch_at`, and active continuation sources such as
children and persistent goals separately. The meaningful-inactivity duration
defaults to one hour.

Real user input or actual provider/tool work performed by the parent session
MUST reset the meaningful-activity clock. Only a provider request sent under
the exact parent cache identity counts as its cache touch. Synthetic
maintenance, passive monitor or child-progress delivery, and child provider or
tool work MUST NOT reset the parent inactivity clock or cache-touch clock.

An active child MAY make the parent eligible for a bounded cache-hold policy,
but MUST NOT extend the inactivity duration or `max_hold_while_child` by
remaining chatty.

#### Scenario: Tool work extends the active window

- **GIVEN** 55 minutes have passed since the parent's last meaningful activity
- **WHEN** the parent runs a real tool call
- **THEN** the parent inactivity clock restarts from that activity
- **AND** the tool call alone does not update the provider cache-touch clock

#### Scenario: Passive monitor does not pin cache forever

- **GIVEN** a monitor emits status lines but no parent turn or tool work follows
- **WHEN** the inactivity limit passes
- **THEN** Smith considers the parent inactive despite those lines
- **AND** sends no maintenance because of passive delivery alone

#### Scenario: Child remains busy

- **GIVEN** the parent entered a parked state 40 minutes ago
- **AND** a child has continued making provider and tool calls
- **WHEN** Smith evaluates the parent lease
- **THEN** child activity refreshes neither parent clock
- **AND** the original bounded child-hold deadline still applies

### Requirement: Adapter-gated ephemeral keepalive

Smith SHALL send an ephemeral keepalive only when it is enabled, authorized,
and explicitly supported by the resolved model's cache contract. An eligible
keepalive MAY use:

- the same provider, endpoint, model, cache key, and exact identity;
- the exact stable parent prefix;
- a minimal noncanonical suffix;
- any tool schemas already contained in the exact stable prefix, with tool
  choice forced to none and no execution or structured side effects;
- a bounded output limit and deadline;
- provider-selected retention controls; and
- jittered scheduling before a known lease boundary.

The adapter MUST have passed conformance proving that the prefix remains
reusable after synthetic suffix removal. Request and response MUST be excluded
from canonical conversation history, while attempt identity, usage, cache
evidence, error, latency, cost, and disposition are recorded separately.
Synthetic activity MUST NOT reset meaningful inactivity.

#### Scenario: Keepalive hits the old prefix

- **GIVEN** identity A is eligible for a conformance-approved keepalive
- **AND** its parked interval still has maintenance budget
- **WHEN** the keepalive reports positive cache-read usage
- **THEN** A records `warm_observed` and the declared retention effect
- **AND** maintenance usage is attributed to `cache_keepalive`
- **AND** canonical history contains neither ping nor pong

#### Scenario: Automatic-prefix behavior is unproven

- **GIVEN** an implicit-prefix adapter has not passed suffix-removal and
  retention conformance
- **WHEN** adaptive maintenance is evaluated
- **THEN** Smith remains observation-only
- **AND** sends no synthetic request

#### Scenario: Recent parent activity makes keepalive unnecessary

- **GIVEN** a keepalive is scheduled
- **WHEN** a real parent provider request touches the exact identity before
  dispatch
- **THEN** the scheduled keepalive is suppressed
- **AND** Smith records the suppression reason without provider I/O

### Requirement: No additional rebuild after miss

Smith SHALL, after an observed synthetic-maintenance miss or typed upstream
expiry/resource evidence correlated to the exact identity, record the evidence,
suspend further synthetic work for that identity, and MUST NOT send a second
prewarm, rebuild, or cache-only probe. The same
no-prewarm rule applies after idle compaction, cache identity change, and
process resume with unknown provider state. A later real user, parent, goal, or
child-result continuation MAY naturally create a new provider cache.

#### Scenario: Keepalive misses

- **GIVEN** a synthetic keepalive reports no cache read and reports uncached or
  cache-write input for identity A
- **WHEN** Smith updates the lease
- **THEN** A records `miss_observed` and transitions to `suspended`
- **AND** Smith sends no follow-up cache-only request

#### Scenario: Cold child-result continuation

- **GIVEN** the parent cache is absent or unknown
- **WHEN** a terminal child outcome wakes the idle parent
- **THEN** Smith sends one normal attributed continuation using durable state
- **AND** sends no prewarm request first

#### Scenario: Process resumes with a prior warm observation

- **GIVEN** persisted diagnostics show that identity A was previously warm
- **WHEN** a new process resumes without an explicit resource inspection
- **THEN** provider warmth starts unverified
- **AND** no provider request is sent merely to reconstruct the old claim

### Requirement: Idle-limit automatic compaction

At the configured inactivity limit, Smith SHALL wait for a safe boundary,
persist exact structured resume state, and invoke Agent Runtime's configured
semantic compactor at most once for the current idle interval. Compaction is
separate from the parked-interval synthetic maintenance-call budget, but its
single ordinary provider attempt remains subject to provider, session, model,
input/output, deadline, usage, and global limits and MUST NOT automatically
retry. Successful
compaction SHALL:

- replace only eligible old history with a durable semantic summary;
- preserve configured recent canonical turns;
- preserve source and coverage provenance;
- create a new context/cache identity;
- stop maintenance for the old identity; and
- avoid prewarming the new identity.

Compaction usage and outcome MUST remain attributed. A child that is still
running MUST NOT be interrupted merely because the parent's idle limit passed.

#### Scenario: One-hour inactivity expires

- **GIVEN** the default one-hour limit is reached at a safe parent boundary
- **WHEN** Smith processes the idle transition
- **THEN** it persists a resume capsule and attempts compaction once
- **AND** stops old-prefix maintenance
- **AND** waits for real continuation before any new-prefix cache creation

#### Scenario: Limit expires during a tool call

- **GIVEN** the inactivity deadline becomes due while a parent tool is
  executing
- **WHEN** the tool has not reached a safe boundary
- **THEN** Smith queues the one compaction attempt without interrupting the tool
- **AND** runs it only after the boundary remains eligible

#### Scenario: Inactivity limit expires while child is running

- **GIVEN** the parent is parked and a child remains active
- **WHEN** the inactivity limit is reached at a safe parent boundary
- **THEN** Smith persists exact resume state and may compact once
- **AND** does not interrupt the child
- **AND** performs no cache prewarm for the compacted parent prefix

#### Scenario: Automatic compaction fails

- **GIVEN** automatic compaction is attempted once
- **WHEN** the summary provider or persistence step fails
- **THEN** Smith records a visible failure with attributed usage
- **AND** preserves original canonical history
- **AND** sends no automatic retry loop
- **AND** stops synthetic cache maintenance at the idle limit

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

### Requirement: Structural reuse and provider warmth are distinct

Smith SHALL report structural cache planning separately from provider cache
evidence. At minimum, these values MUST remain distinct:

```text
structurally_preserved_prefix_tokens
provider_cache_read_tokens
provider_cache_write_tokens
provider_cache_status
provider_cache_guaranteed_until
```

Smith MUST NOT describe structurally preserved tokens as a verified cache hit,
write, guarantee, or saving.

#### Scenario: Byte-identical prefix after a long pause

- **GIVEN** the next request preserves 40,000 stable-prefix tokens
- **AND** the provider exposes no cache observation
- **WHEN** Smith renders cache status
- **THEN** it reports 40,000 structurally preserved tokens
- **AND** provider warmth is `unknown`
- **AND** no verified hit is reported

### Requirement: Adaptive cache-retention policy

Smith SHALL evaluate synthetic cache maintenance only when all of the following
are true:

- the host process, session, and lifecycle lease remain active;
- a real continuation source exists, such as a running child or active goal;
- the exact cache identity remains applicable;
- the provider adapter permits the proposed action;
- explicit host authority `synthetic_cache_spend = allow` authorizes it;
- no real parent activity has already refreshed the prefix;
- configured call, exact planned input/model, output, and deadline budgets
  remain, together with provider/session attempt and usage limits;
- inactivity and child-hold limits have not elapsed; and
- no miss, expiry, shutdown, or suspension prohibits maintenance.

A user-idle session with no continuation source SHALL receive no synthetic
maintenance by default.

#### Scenario: User leaves with no active work

- **GIVEN** the user has not sent input
- **AND** no child, goal, tool, or required continuation is pending
- **WHEN** the maintenance policy runs
- **THEN** Smith sends no keepalive or handoff checkpoint
- **AND** relies on durable cold-resume state for future continuation

#### Scenario: Child will finish inside guaranteed retention

- **GIVEN** the parent is parked with a running child
- **AND** the provider guarantee extends beyond the next evaluation window
- **WHEN** maintenance is evaluated
- **THEN** Smith sends no synthetic request

#### Scenario: Repository asks for spend the host forbids

- **GIVEN** project configuration requests adaptive maintenance
- **AND** user or host policy withholds synthetic provider spend
- **WHEN** Smith resolves the effective cache policy
- **THEN** maintenance is narrowed to `observe` or `off` with provenance
- **AND** repository text cannot authorize the request

### Requirement: Bounded maintenance budget

Synthetic cache maintenance SHALL be bounded by configuration, provider
contract, and host policy. The default SHALL permit at most one synthetic
maintenance request per exact cache identity during one parked interval. A
handoff checkpoint and a keepalive share `max_maintenance_calls`; a handoff
checkpoint counts as that request, and Smith MUST NOT additionally send a
dummy keepalive unless separately configured and independently authorized.
The request MUST fit the exact resolved plan/model input budget, configured
output and deadline limits, and provider/session attempt and usage limits.
Calculated price or cost is recorded for presentation only and MUST NOT enter
dispatch eligibility or any maintenance budget.

Maintenance SHALL stop when the user submits input, the parent resumes real
work, all continuation sources stop, the identity changes, suspension occurs,
the inactivity or child-hold limit is reached, any exact call/input/output/
deadline/provider/session limit is exhausted, host authority is absent, or
shutdown begins. Idle compaction remains a separate once-per-idle-interval
ordinary attempt and never retries.

#### Scenario: Child runs indefinitely

- **GIVEN** a child remains alive beyond `max_hold_while_child_ms`
- **WHEN** the hold limit is reached
- **THEN** Smith stops maintaining the parent cache
- **AND** leaves the child subject to ordinary child policy
- **AND** relies on durable cold-continuation state when the child finishes

#### Scenario: Handoff already consumed the allowance

- **GIVEN** Smith completed a handoff checkpoint in the current parked interval
- **WHEN** a later keepalive boundary is evaluated
- **THEN** the default maintenance budget is exhausted
- **AND** no keepalive request is sent

### Requirement: Same-model handoff checkpoint

Smith SHALL create a handoff checkpoint only when it is enabled, authorized,
and adapter-approved. One eligible checkpoint MAY run before a cache guarantee
or bounded hold ends and SHALL:

- use the exact parent provider, endpoint, model, cache key, and cache identity;
- reuse the exact stable parent prefix;
- add a noncanonical instruction requesting a concise continuation summary;
- preserve any identity-bound stable tool schemas while forcing tool choice to
  none and disabling execution;
- use bounded output and deadline limits;
- exclude request and response from canonical history;
- persist summary text, route, source coverage, and provenance in the resume
  capsule;
- record cache reads/writes, usage, cost, latency, and outcome; and
- perform no automatic retry loop.

It MAY refresh or extend retention only when the provider contract explicitly
declares that behavior. The reported cost is presentation-only and cannot make
an otherwise ineligible checkpoint eligible.

#### Scenario: Warm checkpoint creates durable value

- **GIVEN** the parent is parked with a running child
- **AND** its exact prefix remains warm
- **WHEN** Smith creates a handoff checkpoint
- **THEN** the provider may read the stable prefix from cache
- **AND** Smith persists the attributed continuation summary
- **AND** later cold resume does not depend on the cache remaining present

#### Scenario: Handoff checkpoint misses

- **GIVEN** Smith sends one handoff checkpoint
- **WHEN** canonical provider evidence reports an observed miss
- **THEN** Smith may retain a valid returned summary
- **BUT** suspends further maintenance for the old identity
- **AND** sends no prewarm request

#### Scenario: Parent identity changes at the projection boundary

- **GIVEN** a handoff operation returned for cache identity A
- **AND** a real parent turn is ready to commit cache identity B
- **WHEN** Smith attempts to persist A's identity-bound handoff summary
- **THEN** it holds Runtime's current-identity lease through capsule
  persistence or discards the stale projection
- **AND** any later real identity change retires A's handoff metadata before
  the capsule is persisted again

#### Scenario: Provider emits a tool call

- **GIVEN** a checkpoint request forced tool choice to none and exposed no
  executable side-effect capability
- **WHEN** the provider emits a tool call anyway
- **THEN** Smith fails the checkpoint attempt and records a contract violation
- **AND** executes no tool

#### Scenario: Crash loses protected live summary

- **GIVEN** Runtime durably completed a handoff operation but Smith crashed
  before persisting its protected live-only text into the resume capsule
- **WHEN** the same operation is recovered
- **THEN** Smith accepts the persisted completion without a summary
- **AND** Runtime does not replay the provider request
- **AND** cold resume reconstructs from canonical state

### Requirement: Summary-model isolation

Smith SHALL treat a summary request using another model, provider, endpoint, or
cache partition as ordinary semantic compaction, not cache maintenance. It MAY summarize
a bounded delta plus exact structured state, but MUST NOT be described as
refreshing or preserving the parent model's cache.

#### Scenario: Smaller summary model

- **GIVEN** the parent uses model A
- **AND** semantic compaction uses model B
- **WHEN** model B creates a summary
- **THEN** Smith attributes usage and summary revision to model B
- **AND** does not update model A's lease or cache-touch timestamp

### Requirement: Cache lifecycle observability

Smith SHALL consume Agent Runtime's canonical redaction-safe plan,
observation, admission, `CacheOperationPrepared`, `CacheOperationRejected`,
`CacheOperationStarted`, `CacheOperationCompleted`,
`CacheAvailabilityEvidenceRecorded`, and `CacheOperationSuspended` events.
Smith MAY project bounded identifiers, upstream revisions, counts, timestamps,
reasons, attempts, usage, and dispositions into status, replay, and headless
results. Smith lease, scheduler-intent, policy-suppression, capsule, and
next-action state MUST remain consumer projections keyed to upstream identity;
they MUST NOT become RuntimeEvent variants or a second normalized provider
event vocabulary, and MUST NOT expose raw system instructions, private profile
text, credentials, or cache contents.

#### Scenario: Suppressed maintenance is explainable

- **GIVEN** a keepalive was scheduled
- **WHEN** it is suppressed because real activity occurred
- **THEN** Smith records a bounded consumer policy-suppression projection
- **AND** no provider I/O is issued

#### Scenario: Runtime rejects a prepared operation at dispatch

- **GIVEN** Runtime prepared a bounded cache operation
- **AND** identity, authority, budget, cancellation, or shutdown invalidates it
- **WHEN** Runtime rejects dispatch
- **THEN** Smith projects the canonical `CacheOperationRejected` reason
- **AND** no provider attempt is fabricated

#### Scenario: Runtime event is projected without a second vocabulary

- **GIVEN** Agent Runtime emits one canonical maintenance completion event
- **WHEN** Smith updates live status and replays the same event
- **THEN** both projections retain the upstream event identity and disposition
- **AND** Smith creates no replacement canonical cache event

### Requirement: Cache-maintenance security boundary

Every synthetic cache request SHALL preserve only tool schemas already bound
into the exact stable prefix, force tool choice to none, and expose no tool
execution, mutation, process, network, delegation, interaction, or approval
capability. It SHALL use a separately attributed purpose; fit the exact
resolved plan/model input budget; have bounded output, deadline,
provider/session attempt, usage, and no-retry policy; require explicit host
synthetic-spend authority; remain cancellable during shutdown; and stop after
the session releases its lifecycle lease.
Calculated price or cost MUST remain presentation-only. Redaction-safe state
MUST NOT persist raw credentials, provider cache contents, or private
instruction bodies.

#### Scenario: Shutdown starts before dispatch

- **GIVEN** maintenance is scheduled
- **WHEN** the host begins shutdown before provider dispatch
- **THEN** the request is cancelled without network I/O
- **AND** no later scheduler task can revive it after lease release

#### Scenario: Synthetic response contains private prompt text

- **GIVEN** a provider error or summary echoes private stable-prefix content
- **WHEN** Smith records diagnostics and status
- **THEN** ordinary redaction and bounded-projection rules remove that content
- **AND** only safe identity, purpose, usage, and outcome metadata persists

### Requirement: Deterministic cache lease tests

Smith SHALL provide fake-clock and fake-provider conformance tests covering at
least:

1. structural eligibility without observed warmth;
2. a positive read observation;
3. an explicit-zero observed miss and maintenance suspension;
4. typed explicit expiry/resource observation;
5. a guaranteed-retention deadline passing without an invented miss;
6. identity invalidation after tool-schema, profile, model, endpoint, cache-key,
   breakpoint, or resource change;
7. keepalive suppression after real activity;
8. no second maintenance request after a miss;
9. one maintenance call per parked interval by default;
10. idle compaction exactly once, separately from the synthetic-call budget and
    without retry;
11. no prewarm after compaction or cold resume; and
12. shutdown cancellation of scheduled and in-flight maintenance;
13. omitted evidence remaining `unknown`, with `eligible` reserved for a first
    request or no-comparable predecessor; and
14. calculated price/cost never changing dispatch eligibility.

The tests MUST assert canonical events, provider attempt counts, usage purpose,
lease state, and absence of synthetic conversation messages rather than relying
only on rendered text.

#### Scenario: Deterministic lease matrix runs

- **GIVEN** a controllable clock and scripted provider/cache observations
- **WHEN** the cache lifecycle conformance suite advances each boundary
- **THEN** every listed state transition and provider-call count is
  deterministic
- **AND** elapsed time alone never creates hit, miss, or expiry evidence
