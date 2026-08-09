---
created_at: 2026-08-08T23:10:05Z
updated_at: 2026-08-09T18:17:26Z
---

## Why

Long user pauses and delegated child work can let an otherwise reusable prompt
prefix go cold, forcing an expensive refill when the parent continues. Smith's
current specification permits generic adapter-gated keepalives and idle
compaction, but it does not define an evidence-bearing cache lease, a bounded
maintenance budget, a parked parent lifecycle, or a durable cold-resume
package. The missing contracts make it too easy to confuse structural prefix
reuse with a verified provider hit or to keep provider work alive indefinitely
while a child runs.

Smith needs one coordinated policy that can reduce avoidable cache refill cost
without making provider cache state a durability or correctness dependency.
Correct continuation must still work after a complete cache miss, provider or
process restart, model change, or eviction.

## What Changes

- Require a separately approved Agent Runtime mechanism change to land at an
  immutable Git revision, then pin that revision in Smith. Package publication
  is outside this change; a floating branch or sibling checkout is not an
  acceptable consumer dependency.
- Extend Agent Runtime's per-model provider capability contract with opaque exact
  cache identity, typed retention/resource evidence, explicit-resource
  operations, and conformance-gated synthetic-maintenance safety.
- Add an evidence-bearing cache lease that keeps structural eligibility,
  provider observations, guaranteed retention, meaningful activity, and cache
  touches distinct.
- Add adaptive, adapter-gated cache maintenance with one synthetic request per
  parked interval by default, bounded exact input/output/deadline and provider
  limits, no retry loop, and no rebuild after a miss, compaction, identity
  change, or cold resume.
- Prefer an optional same-provider-and-model handoff checkpoint that creates a
  useful durable continuation summary over a dummy keepalive when one bounded
  maintenance request is justified.
- Let a parent complete its provider turn and enter
  `parked-awaiting-child`; make `agent.wait` bounded and admit one attributed
  child-completion continuation only when the parent remains idle.
- Persist a versioned resume capsule through the existing canonical snapshot
  and protected-checkpoint paths, with exact structured state taking
  precedence over generated summary text.
- Add layered cache lifecycle configuration, redaction-safe lifecycle events,
  separate synthetic-usage attribution, status and machine-output projections,
  fake-clock/provider tests, and adapter conformance gates.
- Migrate the existing `context.idle_compaction_ms` setting to the new cache
  policy surface through a bounded compatibility alias rather than silently
  creating two inactivity clocks.

## Dependencies and Coordination

- Hard dependency:
  `agent-runtime:add-adaptive-cache-parking-2026-08-08`. Agent Runtime must
  approve, implement, and land this mechanism at an immutable Git revision
  before Smith implementation starts; Smith then pins that exact revision in
  every Agent Runtime workspace dependency. Package publication is not
  required by this change; a floating branch or sibling path is insufficient.
  Agent Runtime owns the shared
  provider cache contract, opaque exact plan identity, typed observations,
  canonical operation/admission events, safe-boundary inbox, conditional
  internal-turn admission, and canonical persistence primitives. Smith must
  not add consumer-local substitutes.
- Hard dependency: `add-prompt-cache-miss-visibility-2026-08-08`, whose Smith
  implementation is complete but whose change folder and deltas must be
  archived into the truth specs before this change is rebased. This proposal
  composes on the archived presence-aware explicit-zero, omitted-evidence,
  attempt-correlation, and re-billed-token semantics; its `Evidence-based cache
  status` delta must be merged first, then this change's composed requirement
  must be revalidated without weakening those semantics.
- The existing cache-miss notice setting remains presentation-only. The new
  `context.cache.*` settings control context/cache mechanism and do not change
  the meaning of `cache.miss_notices`.
- Existing Cargo, runtime, archived-change, benchmark, and truth-spec edits in
  the worktree are outside Stage 1 and are not modified by this proposal.

## Impact

- Affected specs: `prompt-cache`, `provider-runtime`, `configuration`,
  `child-agents`, `runtime-integration`, `session-recovery`,
  `usage-accounting`, `client-surfaces`.
- Affected Smith code: `crates/smith-runtime/src/{delegation,host,factory,
  prompt,chatgpt}.rs`, cache lifecycle modules added under `smith-runtime`,
  `crates/smith-config`, `crates/smith-cli/src/{headless,tui_driver,
  runtime_host}.rs`, `crates/smith-tui/src/{status,usage_log,app}.rs`, session
  persistence, and deterministic/runtime conformance fixtures.
- Upstream impact: Agent Runtime provider/model capabilities, context/cache
  plan identity and observations, delegation outcome delivery, conditional
  internal-turn admission, synthetic request purpose metadata, and testkit
  fake clock/provider support.
- Compatibility: additive session and machine-output fields; a coordinated
  pre-1.0 provider capability revision pinned to an immutable Git revision; a deprecated alias for
  `context.idle_compaction_ms`; no change to canonical conversation meaning.
- Network and cost: `adaptive` mode may issue at most the configured bounded
  synthetic provider request when explicit host synthetic-spend authority and
  adapter conformance allow it. `off` and `observe` issue no synthetic
  requests. Calculated price and cost remain presentation-only and never
  authorize dispatch.
- Security: synthetic requests preserve only tool schemas that are already
  identity-bound stable-prefix material, force tool choice to none, never
  execute returned tool calls or expose host side-effect capability, have
  bounded input/output/deadline/retry policy, and never persist raw
  credentials, private prompt bodies, or provider cache contents.

## Out of Scope

- Guaranteeing that a provider cache remains warm or durable.
- Inferring a hit, miss, or expiry from elapsed time or byte identity alone.
- Sharing cache state across provider, endpoint, model, profile, adapter,
  tokenizer, tool-schema, registry, or cache-key identities.
- Using a smaller or different model to refresh the parent's provider cache.
- Keeping a cache alive indefinitely because a child, monitor, or background
  process still exists.
- Prewarming a new prefix after compaction, miss, expiry, identity change, or
  process resume.
- Storing provider cache contents as canonical session state.
- Making child progress or synthetic cache request content part of canonical
  conversation history.
- Detached cache-maintenance daemons or provider work after Smith releases the
  session lifecycle lease.

## Approval Boundary

Approval authorizes the Smith-side policy, configuration, lifecycle,
persistence projection, UI/machine projection, and consumer migration defined
in these deltas, contingent on the separately approved Agent Runtime change
being landed at an immutable Git revision and the completed prompt-cache
miss change being archived into truth specs before this change is rebased. It
does not authorize implementation during Stage 1, a floating runtime
dependency, package publication, unbounded provider traffic, price/cost-based
dispatch, new provider-specific TTL guesses, edits to the sibling Agent
Runtime repository from this change, or provider cache state as a correctness
dependency.
