## Context

Smith already preserves stable prompt fragments, consumes provider cache usage,
injects terminal child outcomes through a protected must-deliver channel, and
persists canonical session state. It also specifies adapter-gated keepalives
and idle compaction. The current pieces stop short of a complete lifecycle:

- `CachePlanChanged` describes a structurally reusable prefix, while provider
  usage or cache APIs provide evidence about remote state. Those facts need
  separate names and clocks.
- `add-prompt-cache-miss-visibility-2026-08-08` adds explicit-zero/omission
  semantics and exact attempt correlation. Adaptive retention must consume
  that canonical evidence rather than introduce another miss detector, and
  the completed Smith change must be archived into truth specs before this
  change's composed delta is rebased.
- terminal child outcomes are losslessly injected, but injection alone does
  not admit a provider turn after the parent has become idle;
- `agent.wait` currently has no timeout in Smith's model-facing schema and can
  hold a parent tool call open for the entire child execution;
- the existing idle-compaction setting does not bound cache-maintenance calls,
  child-related holds, or useful handoff behavior; and
- canonical snapshots and protected checkpoints are durable, but there is no
  named cold-continuation projection that combines exact state, provenance,
  recent turns, and a bounded semantic summary.

The design separates three lanes:

```text
canonical durability    exact history/state/checkpoints; correctness source
active execution        user, parent, child, tool, and attributed internal work
provider cache lease    optional remote-performance evidence and maintenance
```

No transition in the provider-cache lane may be required for either of the
first two lanes to continue correctly.

## Goals / Non-Goals

### Goals

- Preserve an exact cache identity and evidence-bearing lease per provider
  cache candidate.
- Reduce avoidable refill cost while real delegated or persistent work creates
  a credible near-term continuation source.
- Bound every synthetic request by provider contract, explicit host authority,
  call count, exact input/model budget, output, deadline, cancellation, and
  lifecycle. Calculated cost remains presentation-only.
- End the parent's provider turn while a long child continues and resume once
  on terminal child evidence at an idle boundary.
- Make cold continuation correct and useful through existing durable state.
- Keep lifecycle events, usage, status, replay, and headless output
  attributable and redaction-safe.
- Keep provider-specific behavior in Agent Runtime adapters and Smith product
  policy in Smith.

### Non-Goals

- Promise or probe universal cache warmth.
- Preserve a provider stream merely to wait for a child.
- Create a hidden user or assistant message for parking, keepalive, checkpoint,
  or automatic child completion.
- Retry synthetic work until it succeeds.
- Turn a semantic summary into an authority for exact tool, validation, goal,
  child, approval, or artifact state.
- Replace Agent Runtime's canonical snapshot, checkpoint, context planning,
  delegation, usage, or admission contracts.

## Decisions

### 1. Agent Runtime owns the mechanism contract

The separately approved Agent Runtime change adds or revises shared types for
the mechanism. It MUST be landed at an immutable Git revision that Smith pins
in its manifest and lockfile; package publication is outside this change, and
a floating branch or local sibling path cannot satisfy the dependency. The
change adds or revises:

- a per-model provider cache contract;
- exact cache-plan identity and presence-aware observations;
- explicit-resource cache operations where supported;
- conformance-gated synthetic request construction;
- canonical cache operation/evidence events and attempt attribution;
- bounded delegation wait;
- conditional, idle-only internal-turn admission; and
- fake-clock/provider fixtures for cache and admission races.

Smith consumes those types through its existing provider registry, runtime
factory, delegation coordinator, event observer, session handle, and transport.
Agent Runtime owns canonical cache operation/evidence and admission event
payloads. Smith owns consumer lease, scheduler-intent, and capsule projections
keyed to those upstream identities; those projections are not RuntimeEvent
variants or a second normalized provider vocabulary. Smith MUST NOT define a
parallel provider trait or provider request builder. Smith may add endpoint
partition identity or host policy only where the shared plan explicitly cannot
represent Smith-owned inputs.

The cache contract kind is normalized as `unsupported`, `implicit-prefix`,
`explicit-breakpoint`, or `explicit-resource`. The existing
`automatic-prefix` spelling receives a bounded compatibility alias during the
pre-1.0 capability revision; two spellings cannot describe different behavior
for the same resolved model.

### 2. Cache identity is exact; cache state is evidence-bearing

One cache identity covers every request input that can affect prefix reuse:

- provider and endpoint identity;
- model and resolved model-profile fingerprint;
- adapter, tokenizer, and cache-control revisions;
- stable system, agent-profile, project-instruction, skill, and memory
  fragments;
- advertised tool names, descriptions, schemas, and order;
- registry snapshot, scoped view, and activation epoch;
- ordered stable-history segment identities and hashes; and
- provider cache key, breakpoint, or explicit-resource identity.

Changing any input retires the identity from current use. Historical evidence
may remain inspectable, but warmth, eligibility, guarantees, and maintenance
budget never transfer to the new identity.

Smith's lease controller stores, at minimum:

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

The identity is an opaque upstream identity consumed by Smith, not a Smith-local
recomputed hash. It MUST include the provider cache key, breakpoint identity,
and explicit-resource handle whenever the upstream plan exposes them; Smith may
append only a documented endpoint partition when that input is absent upstream.
Changing any of those values retires the lease without transferring warmth.

`guaranteed_until` is only provider-declared minimum-retention evidence. After
that time, the guarantee becomes unavailable; the status does not
automatically become miss or expired. `warm_observed` and `miss_observed`
require canonical provider usage/cache evidence; `expired_observed` requires a
typed upstream resource/expiry event or typed provider error correlated to the
exact identity. Elapsed time and an omitted observation never establish any of
those states.

The active cache-miss visibility change remains authoritative for present
zero, omitted data, expected/observed/missed tokens, confidence, and attempt
correlation. This change extends its composed `Evidence-based cache status`
requirement with lease timing and maintenance suspension; it does not rederive
misses. Omitted cache evidence remains `unknown`; `eligible` is reserved for a
first request or a request with no comparable predecessor.

### 3. Structural reuse and remote warmth are different projections

Context planning reports `structurally_preserved_prefix_tokens`. Usage reports
provider cache reads/writes. Lease status reports provider evidence and any
guaranteed-until timestamp. No UI or machine field may relabel the structural
count as an actual hit.

This distinction also applies after process resume. A saved plan can be used as
a comparison baseline, but provider warmth starts unknown unless an explicit
resource API supplies current evidence.

### 4. Meaningful activity and cache touches use separate clocks

The parent session's meaningful-activity clock resets for real user input or
actual provider/tool work performed by that parent session. Passive monitor
delivery, child progress, synthetic cache maintenance, and child provider/tool
work do not reset it.

The lease's cache-touch clock changes only when a provider request is sent
under that exact identity. A parent tool call is meaningful activity but not a
cache touch. Child activity uses the child's identity and cannot touch the
parent lease.

An active child or active persistent goal is a continuation source that may
make one bounded hold useful. It does not extend either the parent's inactivity
limit or `max_hold_while_child` indefinitely.

### 5. Adaptive maintenance is conditional and budgeted

Smith evaluates a maintenance action only when all of these are true:

1. the process, session, and lifecycle lease remain active;
2. a real continuation source exists;
3. the exact cache identity remains current;
4. the adapter declares the proposed action safe;
5. user/parent activity has not already made it unnecessary;
6. explicit host synthetic-spend authority permits it;
7. the configured call, exact planned input/model, output, and deadline
   budgets remain, together with provider/session attempt and usage limits;
8. inactivity and child-hold limits have not elapsed; and
9. no miss, expiry, shutdown, or suspension prohibits it.

The default permits one synthetic request per identity per parked interval. A
handoff checkpoint consumes that allowance. A scheduled request is rechecked
at dispatch so recent real activity suppresses it without provider I/O.

Unknown or merely compatible OpenAI-style endpoints remain observation-only
until their exact adapter and request shape pass conformance. A provider's
published cache support alone does not authorize suffix removal, retention
refresh, or synthetic spend. Calculated price or cost is never an input to
dispatch eligibility, host authority, or any maintenance budget.

### 6. Prefer one useful same-model handoff over a dummy ping

When enabled and conformance-approved, Smith may send one ephemeral handoff
checkpoint before a declared guarantee or bounded hold ends. It uses the exact
parent provider, endpoint, model, cache key, and stable prefix; advertises no
tools; asks for one bounded continuation summary; and has a short deadline and
no automatic retry.

The request and response are never canonical messages. The response is stored
as semantic text in the resume capsule with model, purpose, source coverage,
usage, cache evidence, and revision. A valid summary may still be retained when
the checkpoint reports a miss, but that miss suspends further maintenance for
the old identity.

If a useful checkpoint is disabled or inappropriate, a minimal ephemeral
keepalive may be used only when the adapter separately declares it safe. Smith
never sends both by default.

A summary on another provider, model, endpoint, or partition is ordinary
semantic compaction. It cannot refresh the parent's lease and is attributed to
its actual model.

### 7. A maintenance miss ends synthetic work for that identity

An observed miss records canonical cache evidence and moves the lease to
synthetic-maintenance suspension. An expiry or resource deletion may do so
only when a typed upstream cache/resource event or typed provider error is
correlated to the exact identity; elapsed time and a missing observation are
never expiry evidence. Smith does not retry, prewarm, rebuild, or probe the
old prefix. The next real continuation is the natural request that may create
a new provider cache.

The same no-prewarm rule applies after idle compaction, identity change, and
process resume. This intentionally accepts a possible cold first real request
in exchange for bounded traffic and simple cost attribution.

### 8. Parent parking ends the turn without ending delegated work

After spawning a child, the parent may continue independent work. Once its
provider/tool turn reaches a normal terminal boundary and at least one child
outcome remains nonterminal/pending, Smith records
`parked-awaiting-child` without holding a provider stream or tool call open.
The pending child remains process-local work: if the process restarts, any
running or otherwise uncommitted child is reconciled to
`interrupted_by_process_exit` and is never auto-restarted. A terminal outcome
committed before restart remains terminal and may be delivered once through
the protected channel.

`agent.wait` accepts `timeout_ms`, defaults to 5 seconds, and is capped at 30
seconds unless host policy narrows those values. Timeout is a successful
`running` result; the child continues and its terminal outcome remains
must-deliver. The tool description explicitly says completion is delivered
automatically.

Terminal outcomes enter the protected lossless channel with deterministic
ordering keys. The coordinator coalesces only passive progress. After injection,
Smith asks Agent Runtime to admit one internal continuation attributed as
`delegation.child-completion`. Admission succeeds only if the session remains
idle at the serialized boundary. It drains all ready terminal outcomes in
deterministic order and runs through ordinary context, provider, tool,
approval, cancellation, checkpoint, retry, usage, and cache planning.

Real user input always wins. If a user turn is ready, the child outcome joins
that turn at the next safe boundary or remains protected for the immediately
following continuation. A goal continuation and child continuation use the
same idle-only admission arbitration and cannot create concurrent parent
turns.

### 9. The resume capsule is a projection over canonical durability

The resume capsule is a logical versioned package, not a new database or
source of truth. Its bounded redaction-safe projection lives in the existing
snapshot or versioned extension state; sensitive exact pending state,
protected child state, and resource handles remain in the authenticated
protected checkpoint. Existing watermarks and atomic commit rules determine
which version is authoritative: the highest compatible committed protected or
canonical watermark wins, journal replay is presentation-only, and semantic
summary prose never overrides exact state or schedules work.

The capsule contains exact structured projections where available: session
and turn identity, goal/plan generation, children and outcomes, changed-file
and validation evidence, artifacts, unresolved interactions/approvals,
constraints, recent canonical turns, semantic summary, summary route and
revision, and source coverage.

Canonical history, protected exact state, tool/validation evidence, goal
state, child state, and artifact metadata always outrank summary prose. A
conflict is diagnostic evidence; it is not resolved in favor of the model's
text.

Exact state updates at existing meaningful commit boundaries. Semantic
summaries may update less often from the previous summary plus a bounded delta
and exact projection. This avoids repeatedly summarizing the whole transcript
while retaining cold-resume correctness.

### 10. Idle compaction is once-per-interval and creates a new identity

At the inactivity limit, Smith waits for a safe boundary, persists exact resume
state, and attempts automatic compaction once for that idle interval.
Compaction is separate from the parked-interval synthetic maintenance-call
budget, but it is still one ordinary provider attempt subject to provider,
session, model, input/output, deadline, usage, and global limits, with no
automatic retry. Success
replaces eligible old history through Agent Runtime's normal summary/manifest
contract, retains configured recent canonical turns and provenance, creates a
new context/cache identity, and stops the old lease.

Failure retains original canonical history, records attributed usage and a
visible failure, sends no retry loop, and stops synthetic maintenance at the
limit. A running child is neither interrupted nor used to extend the deadline.

### 11. Configuration distinguishes request, capability, and authority

The mechanism surface is:

```toml
[profiles.<name>.context.cache]
maintenance = "adaptive"             # off | observe | adaptive
inactivity_limit_ms = 3600000
max_hold_while_child_ms = 3600000
max_maintenance_calls = 1
max_maintenance_input_tokens = 0      # 0 = exact resolved model/plan budget
max_maintenance_output_tokens = 256
maintenance_deadline_ms = 30000
keepalive_margin_ms = 120000
keepalive_jitter_percent = 10
handoff_checkpoint = true
idle_compaction = true
resume_capsule = true

[profiles.<name>.child_agents]
wait_default_timeout_ms = 5000
wait_max_timeout_ms = 30000
```

`off` disables synthetic maintenance; `observe` records plans and evidence but
sends nothing; `adaptive` allows only actions that also pass adapter,
lifecycle, budget, and host authority. Unsupported or invalid combinations
fail closed without making the provider unusable for ordinary turns.

The resolved numeric policy is bounded and source-explainable. The accepted
ranges are `inactivity_limit_ms: 1_000..=86_400_000` (zero is invalid),
`max_hold_while_child_ms: 0..=86_400_000` (zero disables the child hold),
`max_maintenance_calls: 0..=8` (zero disables synthetic maintenance),
`max_maintenance_input_tokens: 0..=resolved_model_input_limit` (zero uses the
exact resolved plan/model input budget), `max_maintenance_output_tokens:
1..=4096`, `maintenance_deadline_ms: 1..=120_000`,
`keepalive_margin_ms: 0..=inactivity_limit_ms` (zero means no early margin),
and `keepalive_jitter_percent: 0..=50` (zero is deterministic). The wait
default is `0..=30_000` milliseconds, the wait maximum is `1..=30_000`, the
default is 5,000, the maximum is 30,000, and the default MUST not exceed the
maximum. A requested `agent.wait.timeout_ms` above the resolved maximum is
rejected before waiting; a zero wait is an immediate status check.

The existing `profiles.<name>.context.idle_compaction_ms` is accepted as a
deprecated alias for `profiles.<name>.context.cache.inactivity_limit_ms` for
one transition release. Within one layer, conflicting declarations fail
preflight; across layers, the normal layer precedence selects one winner and
the losing alias remains visible in provenance. Equal-precedence aliases from
different sources fail as ambiguous. `cache.miss_notices` remains a separate
presentation setting from the implementing cache-miss change.

Project configuration may narrow or disable maintenance but cannot grant
provider-spend authority withheld by user or host policy. The effective
synthetic dispatch path requires an explicit host authority value
`synthetic_cache_spend = allow`; its default is deny, and no project or
repository setting can produce that value. Explain output must show both the
requested setting and any effective narrowing reason, including missing host
authority.

### 12. Events and usage are canonical, separate, and redaction-safe

Agent Runtime's canonical stream provides plan/observation/admission evidence
and `CacheOperationPrepared`, `CacheOperationRejected`,
`CacheOperationStarted`, `CacheOperationCompleted`,
`CacheAvailabilityEvidenceRecorded`, and `CacheOperationSuspended`. Smith
consumes those events and projects their redaction-safe identity/revision,
counts, timestamps, reasons, usage, and dispositions into TUI, machine, and
session views. Smith's lease creation/retirement, scheduler intent, local
policy suppression, capsule persistence, and next-action status remain
consumer projections keyed to upstream identity; they are not RuntimeEvent
variants or a competing provider event vocabulary. Neither lane exposes raw
prompts, credentials, private instructions, or cache contents.

Stable request purposes retain the existing `cache_keepalive` ID and add
`cache_handoff_checkpoint` and `cache_idle_compaction`. Every attempt preserves
provider, model, cache identity, disjoint usage counters, counter provenance,
cost, latency, and outcome. Synthetic usage counts toward total spend and
provider/session limits but remains distinct from user, parent, and child
turns. Calculated cost and price provenance are presentation-only; neither
authorizes or suppresses dispatch.

Economic scheduling estimates never become provider-reported cache hits,
usage, or savings. Status and headless output expose structural prefix tokens,
provider evidence, guarantee timestamp, maintenance budget/disposition, and
synthetic usage as separate fields.

### 13. Shutdown cancels maintenance before draining work

Shutdown first prevents new maintenance, cancels any in-flight synthetic
request, and freezes child-completion admission. It then applies existing
child stop/wait/durability policy, commits the latest compatible resume state,
releases explicit cache resources according to adapter policy, drains and
syncs persistence, and releases the lifecycle lease.

A headless process may remain alive for required child completion under its
ordinary policy. It never remains alive solely to preserve a provider cache
after required work is complete.

## Risks / Trade-offs

- Provider cache guarantees and observation fields vary. The conservative
  contract leaves some adapters in `observe`, reducing optimization but
  avoiding false claims and surprise traffic.
- One maintenance call may not bridge a very long child task. The fixed call,
  token, output, deadline, provider/session, and host-authority bounds keep
  traffic predictable; cold continuation remains correct.
- A same-model handoff consumes output tokens. It is preferred only because it
  creates durable semantic value; deployments can disable it or all synthetic
  work.
- Automatic child-completion turns add provider spend without a new user
  message. They use existing internal-turn attribution, user-priority
  admission, host policy, limits, and machine-visible accounting.
- Resume capsules duplicate projections of canonical facts. Watermarks,
  versioning, exact-state precedence, and no new storage authority prevent the
  projection from becoming a conflicting source of truth.
- The active cache-miss change touches the same status requirement and client
  projections. Landing order and a composed delta are required to prevent an
  archive-time loss of explicit-zero or omission semantics.

## Migration Plan

1. Approve, implement, and land the coordinated Agent Runtime cache/admission
   contract at an immutable Git revision, including conformance fixtures and
   the compatibility alias for `automatic-prefix`; package publication is out
   of scope, and a floating branch or sibling path is not a landing.
2. Complete and archive `add-prompt-cache-miss-visibility-2026-08-08`, merge
   its approved deltas into truth specs, then rebase this change's composed
   `Evidence-based cache status` delta on that archived truth.
3. Pin every Smith Runtime workspace dependency and lockfile entry to the
   landed immutable Git revision, then add configuration parsing in
   `off`/`observe` behavior first.
4. Add the lease controller, events, accounting, resume capsule, and fake-clock
   tests without enabling synthetic dispatch.
5. Add bounded wait, parking, and child-completion admission with deterministic
   race tests.
6. Enable `adaptive` dispatch only for adapters that pass synthetic suffix,
   tool-disable, evidence, deadline, cancellation, and no-duplicate conformance.
7. Retain the deprecated `context.idle_compaction_ms` alias for one transition
   release, then remove it through a separately approved change.
8. Run full cross-surface replay, cold-resume, shutdown, and provider-adapter
   conformance before release.

## Open Questions

None for the approval boundary. Provider-specific retention choices and cache
resource APIs remain adapter declarations; they do not change the shared
correctness invariant or authorize a provider-specific Smith shortcut.
