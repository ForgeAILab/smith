---
created_at: 2026-08-08T23:10:05Z
updated_at: 2026-08-09T02:38:40Z
completed_at:
---

## 1. Coordinate and pin Agent Runtime contracts

- [ ] 1.1 Approve, implement, and land
  `agent-runtime:add-adaptive-cache-parking-2026-08-08` at an immutable Git
  revision with per-model cache contracts, opaque exact identity, typed
  observations/resource evidence, explicit cache operations, conditional
  internal-turn admission, bounded delegation wait, Runtime-owned canonical
  operation/admission events, and fake-clock/provider seams. Package
  publication is outside this change; a floating branch or sibling path is
  insufficient.
- [ ] 1.2 Complete and archive `add-prompt-cache-miss-visibility-2026-08-08`,
  merge its deltas into truth specs, then rebase this change's composed
  `Evidence-based cache status` requirement on that archived truth without
  losing explicit-zero, omitted-evidence-unknown, first/no-comparable
  eligibility, attempt-correlation, or re-billed-token semantics.
- [ ] 1.3 Pin every Smith Agent Runtime workspace dependency and lockfile entry
  to the immutable Git revision and migrate exhaustive provider capability,
  runtime event, delegation, persistence, and usage matches without adding
  Smith-local compatibility traits or a second event vocabulary.
- [ ] 1.4 Add a bounded compatibility alias from `automatic-prefix` to
  `implicit-prefix` and reject ambiguous per-model cache declarations.

## 2. Resolve cache policy and provider capabilities

- [ ] 2.1 Add layered `[profiles.<name>.context.cache]` configuration with
  exact paths, defaults, ranges, zero/conflict semantics, source provenance,
  enum/value validation, and explain output; add the bounded
  `[profiles.<name>.child_agents]` wait defaults and maximum.
- [ ] 2.2 Treat `context.idle_compaction_ms` as a deprecated one-release alias
  for `context.cache.inactivity_limit_ms`; reject conflicting same-layer values
  and document migration.
- [ ] 2.3 Keep `cache.miss_notices` presentation-only and prove it cannot alter
  provider requests, maintenance scheduling, or cache state.
- [ ] 2.4 Resolve the requested policy against provider/model capability,
  adapter conformance, explicit host synthetic-spend authority, lifecycle
  state, and ordinary call/input/output/deadline/provider/session limits;
  calculated cost/price is presentation-only. Unsupported or invalid
  maintenance must fail closed while ordinary provider use remains available.
- [ ] 2.5 Extend built-in and fake provider model profiles with cache-contract
  fixtures for unsupported, implicit-prefix, explicit-breakpoint, and
  explicit-resource behavior.

## 3. Implement cache identity and lease lifecycle

- [ ] 3.1 Add an exact identity wrapper that consumes Agent Runtime's opaque
  context/cache plan identity, including provider cache key, breakpoint, and
  resource identity; add Smith-owned endpoint partition only when the upstream
  plan cannot represent that Smith input.
- [ ] 3.2 Add the evidence-bearing lease state, guaranteed-retention metadata,
  meaningful-activity/cache-touch clocks, parked-interval identity,
  maintenance counters, and suspension reason.
- [ ] 3.3 Consume Agent Runtime's canonical plan, attempt, usage, cache-state,
  admission, and explicit-resource events once; keep structural preservation,
  provider reads/writes, status, and guarantee fields separate. Smith emits
  projections only and defines no second event vocabulary.
- [ ] 3.4 Retire applicability on provider, endpoint, model/profile, adapter,
  tokenizer, prompt fragment, tool schema/order, registry/view/activation,
  stable history, cache-key, or breakpoint changes without transferring warmth.
- [ ] 3.5 Add fake-clock tests for guarantee passage without invented expiry,
  positive read, explicit-zero miss, omission as unknown, typed resource/
  expiry evidence, explicit resource deletion, identity invalidation, and
  process-resume unknown state.

## 4. Implement bounded adaptive maintenance

- [ ] 4.1 Add one cancellable scheduler per active lease that rechecks exact
  identity, continuation source, recent real activity, provider contract,
  explicit host authority, call/input/output/deadline/provider/session limits,
  inactivity/hold limits, suspension, and shutdown at dispatch; calculated
  price/cost must not participate.
- [ ] 4.2 Implement conformance-gated ephemeral keepalive requests with exact
  prefix identity, no tools, bounded suffix/output/deadline, jitter, no
  canonical history, separate purpose, and no automatic retry.
- [ ] 4.3 Implement the optional same-model handoff checkpoint, persist its
  bounded summary and provenance into the resume capsule, share
  `max_maintenance_calls` with keepalive, and count it as the parked
  interval's maintenance request.
- [ ] 4.4 Suppress scheduled maintenance after a real matching parent request;
  record a reason without provider I/O.
- [ ] 4.5 On an observed maintenance miss, typed upstream expiry/resource
  evidence, tool-call contract violation, or identity change, suspend further
  synthetic work and send no retry, prewarm, or rebuild request; elapsed time
  or omitted evidence is never expiry evidence.
- [ ] 4.6 Prove a different provider/model summary is ordinary compaction and
  never touches the parent lease.

## 5. Park parents and admit child completion safely

- [ ] 5.1 Extend the model-facing `agent.wait` schema with optional
  `timeout_ms`, exact `[profiles.<name>.child_agents]` default/max paths,
  5-second default, 30-second maximum, bounded ranges, zero immediate-check,
  rejection above maximum, and a successful `running` timeout result; update
  the description to promise automatic terminal delivery.
- [ ] 5.2 Record `parked-awaiting-child` only when at least one direct child
  remains nonterminal/pending, without retaining a provider stream, tool call,
  or synthetic transcript message; terminal outcomes awaiting delivery alone
  do not create parked state.
- [ ] 5.3 Preserve terminal child outcomes through the protected lossless
  channel with deterministic ordering; progress may remain coalescable.
- [ ] 5.4 After terminal injection, call Agent Runtime's
  `try_admit_child_completion_if_idle` with the expected
  `ChildOutcomeCursor`; handle `Accepted`, `Busy`, `Stale`, `Shutdown`, and
  `Conflict` without loss, and drain all ready outcomes in deterministic order
  through ordinary runtime policy and accounting.
- [ ] 5.5 Serialize user, child-completion, and goal admission so real user
  input wins, no competing provider turns start, and outcomes are neither lost,
  duplicated, nor reordered.
- [ ] 5.6 Ensure child provider/tool/progress activity does not reset the parent
  inactivity or cache-touch clocks and cannot extend
  `max_hold_while_child_ms`.
- [ ] 5.7 Add deterministic parking, wait-timeout, multi-child ordering,
  user-race, goal-race, live-child restart reconciliation/no-auto-restart,
  terminal-delivery-once, cold child-result continuation, and bounded-shutdown
  tests.

## 6. Persist cold-continuation state and compact once

- [ ] 6.1 Add the versioned resume-capsule projection through existing
  snapshot/extension state and protected checkpoints; keep live child
  execution process-local, reconcile running/uncommitted children to
  `interrupted_by_process_exit` on restart, and add no sidecar database or
  project file.
- [ ] 6.2 Persist exact session/turn, goal/plan, child, mutation, validation,
  artifact, interaction/approval, constraint, summary, provenance, and recent
  canonical-turn projections at the specified commit boundaries.
- [ ] 6.3 Make the highest compatible committed protected/canonical watermark
  authoritative, with protected exact state ahead of same-boundary projections,
  journal replay presentation-only, and semantic text never overriding exact
  state or scheduling work; emit a redaction-safe diagnostic for conflicts.
- [ ] 6.4 Update semantic summaries from bounded deltas plus the previous
  summary and exact projection; retain same-model handoff and independent
  summary routes separately.
- [ ] 6.5 At the inactivity limit, persist exact state and attempt compaction
  once per idle interval at a safe boundary; preserve original history on
  failure and stop old-identity maintenance either way.
- [ ] 6.6 Prove cold process resume restores canonical state, starts provider
  warmth unknown, reconciles live children to interrupted without auto-restart,
  preserves committed terminal outcomes for at-most-once delivery, sends no
  prewarm, and lets the next real continuation create cache naturally.
- [ ] 6.7 Add schema migration, corruption, protection-key, watermark,
  summary-conflict, sensitive-content, and repeated-compaction fixtures.

## 7. Project lifecycle, usage, and status consistently

- [ ] 7.1 Consume Agent Runtime's redaction-safe plan, observation, admission,
  `CacheOperationPrepared`, `CacheOperationRejected`,
  `CacheOperationStarted`, `CacheOperationCompleted`,
  `CacheAvailabilityEvidenceRecorded`, and `CacheOperationSuspended` events.
  Keep Smith lease/scheduler/capsule lifecycle as consumer projections keyed to
  upstream identity; do not add RuntimeEvent variants or a second canonical
  provider vocabulary.
- [ ] 7.2 Preserve `cache_keepalive` and add separately attributed
  `cache_handoff_checkpoint` and `cache_idle_compaction` purposes with provider,
  model, cache identity, disjoint counters/provenance, cost, latency, and
  outcome.
- [ ] 7.3 Count synthetic attempts toward provider/session totals and limits
  without presenting them as user, parent, or child turn usage.
- [ ] 7.4 Add lease status, structural preserved tokens, provider reads/writes,
  guarantee timestamp, budget, last/scheduled action, suppression/suspension
  reason, and synthetic totals to status, final JSON, and stream JSON.
- [ ] 7.5 Keep estimated scheduling economics and calculated price/cost
  separate from provider-reported usage, verified hits, actual savings, and
  dispatch eligibility/authority.
- [ ] 7.6 Prove live, journal replay, resumed snapshot, TUI status, text
  diagnostics, final JSON, and stream JSON derive equivalent bounded state.

## 8. Security, shutdown, documentation, and verification

- [ ] 8.1 Add adapter conformance proving the opaque exact identity and
  key/breakpoint/resource fields, exact wire prefix, suffix exclusion from
  later canonical requests, presence-aware observations, typed expiry/resource
  evidence, tools disabled, deadlines/cancellation, and no duplicate
  maintenance retry.
- [ ] 8.2 Reject any synthetic response tool call without execution and record
  the provider contract violation.
- [ ] 8.3 Stop scheduling first on shutdown, cancel in-flight synthetic work,
  freeze internal admission, apply child shutdown policy, persist the latest
  capsule, release explicit resources, drain journals, and release the session
  lease within existing bounds.
- [ ] 8.4 Document configuration, authority narrowing, lease/evidence
  vocabulary, parking, automatic child continuation, capsule precedence,
  synthetic purposes/cost, provider conformance, and the cold-cache invariant.
- [ ] 8.5 `cargo fmt --all --check`.
- [ ] 8.6 `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] 8.7 `cargo test --workspace`.
- [ ] 8.8 Run strict spec validation for
  `add-adaptive-cache-parking-2026-08-08` and re-run the cache-miss change's
  strict validation after composing the landing order.
