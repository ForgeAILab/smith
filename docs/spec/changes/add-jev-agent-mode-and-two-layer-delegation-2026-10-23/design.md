---
created_at: 2026-10-23T21:44:00Z
updated_at: 2026-10-23T21:44:00Z
---

# Design B: bounded delegation and advisory SystemOne triage

## 1. Status, baselines, and normative boundaries

This is a proposed engineering design, not implemented APIs or a passing runtime
build. Structs and signatures below specify the intended interface; new symbols
are explicitly proposed. The source map identifies existing insertion points.
Stage 2 must rebase and re-check line numbers before editing.

Inspected Smith HEAD: `7baa1c7c352c43467ca82b7ce7c56dea9d4976e2`.
Inspected sibling `/Volumes/Data/codes/ai/agent-runtime` HEAD:
`b79134153ba95f8420c40fae6730cb80dd65fd4d`. Smith's working `Cargo.toml:69-77`
declares the `agent-runtime*` Git revision
`a5f1b1788e982c39e7c3a394c715a7d92cc7852b` at version 0.7.0. The sibling checkout
is a reference tree, not proof that the declared dependency already contains the
new API. Coordinate an upstream revision/release and consumer-conformance gate;
do not silently build against a sibling path or change pins during Stage 1.
Both worktrees contain existing user changes.

The complete child-1 research blueprint is retained in Appendix A. Sections 1-13
are the reconciled design and govern where an exploratory sketch differs:

- The current TUI queue is `pending_spawns` at reducer.rs:781, not the blueprint's
  `pending_agent_spawns` at its earlier line number.
- `ActiveTurns` has `current`/`count` bookkeeping, not a field named `active`.
  Do not implement `turns.active.is_some()` from the sketch.
- `take_ready_task_outcomes()` is currently an idempotent snapshot despite its
  name. Pending-work inspection must remain non-consuming, including after any
  refactor; it must include pending persistence/admission and timers, not just
  publicly visible durable ready outcomes.
- A `watch<Option<TurnHandle>>` containing only the latest turn can skip rapid
  continuations. Use a retained sequence-indexed episode ledger plus revision
  notification. A notification alone is not a lossless control channel.
- `SettledWithoutContinuation` after descendant failure is not success. Record
  failure/stopped outcomes and wake the lead; never publish its old parking text.
- `FileSessionStore::list()` already filters catalog-owned children. Extend/test
  that behavior for two layers and partial recovery; do not report the existing
  depth-one filter as absent or replace it with filename guessing.
- Lexical sorting alone is not numeric tree order (`child-10`, `child-2`). Parse
  ID segments for presentation and validate ownership separately from spelling.
- A unique questionnaire ID does not serialize modal presentation. Root's shared
  broker needs an explicit bounded presentation queue and exact response routing.
- An async method on `dyn ChildRuntimeFactory` needs an object-safe future (or the
  repository's async-trait convention), not an unadapted async-trait sketch.

## 2. Source map (existing inspected symbols)

All paths below are relative to the named repository. They are inspection anchors,
not immutable future offsets. New `jev.rs` / host-admission and depth-aware hook
symbols do not exist yet.

| Repository | Existing file:line | Insertion point |
|---|---|---|
| agent-runtime | `crates/agent-runtime/src/runtime/session/mod.rs:56` | SessionInner |
| agent-runtime | `crates/agent-runtime/src/runtime/session/mod.rs:131` | ActiveTurns |
| agent-runtime | `crates/agent-runtime/src/runtime/session/lifecycle.rs:50` | SessionHandle::parent |
| agent-runtime | `crates/agent-runtime/src/runtime/session/turns.rs:589` | conditional internal turn admission |
| agent-runtime | `crates/agent-runtime/src/runtime/engine.rs:1379` | Runtime::start_child_session |
| agent-runtime | `crates/agent-runtime/src/runtime/builder.rs:722` | allow_child_interaction |
| agent-runtime | `crates/agent-runtime/src/runtime/builder.rs:729` | return_child_interactions_to_parent |
| agent-runtime | `crates/agent-runtime/src/agent/driver/mod.rs:640` | interaction disposition |
| agent-runtime | `crates/agent-runtime/src/delegation/types.rs:3` | ChildRuntimeFactory |
| agent-runtime | `crates/agent-runtime/src/delegation/types.rs:55` | DelegationLimits |
| agent-runtime | `crates/agent-runtime/src/delegation/types.rs:81` | DelegationCapacity |
| agent-runtime | `crates/agent-runtime/src/delegation/types.rs:731` | ChildTaskOutcome |
| agent-runtime | `crates/agent-runtime/src/delegation/coordinator.rs:8` | DelegationCoordinator::new |
| agent-runtime | `crates/agent-runtime/src/delegation/coordinator.rs:430` | list |
| agent-runtime | `crates/agent-runtime/src/delegation/coordinator.rs:440` | status |
| agent-runtime | `crates/agent-runtime/src/delegation/coordinator.rs:701` | ready outcome snapshot |
| agent-runtime | `crates/agent-runtime/src/delegation/lifecycle.rs:786` | check_depth |
| agent-runtime | `crates/agent-runtime/src/delegation/lifecycle.rs:809` | mint_child_id |
| agent-runtime | `crates/agent-runtime/src/delegation/lifecycle.rs:1110` | build_and_start |
| agent-runtime | `crates/agent-runtime/src/delegation/monitor.rs:7` | episode collector insertion point |
| agent-runtime | `crates/agent-runtime/src/delegation/monitor.rs:90` | spawn_monitor |
| agent-runtime | `crates/agent-runtime/src/delegation/history.rs:56` | child_history |
| agent-runtime | `crates/agent-runtime/src/delegation/history.rs:66` | subscribe_child_history |
| agent-runtime | `crates/agent-runtime/src/delegation/persistence.rs:110` | recover |
| smith | `crates/smith-config/src/model.rs:233` | ConfigFile |
| smith | `crates/smith-config/src/model.rs:409` | ProfileSection |
| smith | `crates/smith-config/src/resolve/types.rs:1012` | ResolvedAgentProfile |
| smith | `crates/smith-config/src/resolve/agent.rs:363` | profile delegation resolution |
| smith | `crates/smith-runtime/src/delegation.rs:185` | SmithChildFactory |
| smith | `crates/smith-runtime/src/delegation.rs:203` | SmithChildRoute |
| smith | `crates/smith-runtime/src/delegation.rs:383` | child_builder |
| smith | `crates/smith-runtime/src/delegation.rs:722` | wire_delegation_with_wait_policy |
| smith | `crates/smith-runtime/src/delegation/lifecycle.rs:156` | nested lifecycle event reduction |
| smith | `crates/smith-runtime/src/factory/provider/children.rs:331` | child prompt is currently nondelegating |
| smith | `crates/smith-runtime/src/host.rs:197` | HostSession |
| smith | `crates/smith-runtime/src/host/child_history.rs:34` | child_transcript |
| smith | `crates/smith-runtime/src/host/child_history.rs:47` | subscribe_child_transcript |
| smith | `crates/smith-runtime/src/session.rs:455` | FileSessionStore |
| smith | `crates/smith-runtime/src/session.rs:480` | root session listing |
| smith | `crates/smith-client/src/agent_report.rs:123` | agent report projection |
| smith | `crates/smith-tui/src/app/reducer.rs:781` | pending-spawn guard |
| smith | `crates/smith-tui/src/render/composer.rs:848` | draw_agents |
| smith | `crates/smith-tui/src/render/composer.rs:991` | panel_row |
| smith | `crates/smith-cli/src/headless/run_flow.rs:114` | root input admission call |
| smith | `crates/smith-cli/src/headless/run_flow.rs:478` | has_required_child_work |
| smith | `crates/smith-cli/src/interaction.rs:45` | questionnaire dispatch |

Additional verified integration areas are
`crates/smith-runtime/src/factory/construction.rs` (root/default child routes,
security, factory ownership), `factory/capabilities.rs` (questionnaire eligibility),
`prompt.rs` and `delegation/tool.rs` (depth-aware guidance),
`crates/smith-cli/src/tui_driver/events.rs` (child subscription),
`crates/smith-cli/src/local_command/agent.rs` (`/agent`), and
`crates/smith-tui/src/app/state/children.rs` (navigation). Introduce proposed
`crates/smith-runtime/src/jev.rs` and `host/admission.rs` through this composition,
not a separate TUI-only implementation.

## 3. Topology, lineage, and authority

```text
Root session R: depth=0, parent=None, root_session=None, child_id=None
  child-1: session L1, depth=1, parent=R, root_session=R
    child-1.1: session L2a, depth=2, parent=L1, root_session=R
    child-1.2: session L2b, depth=2, parent=L1, root_session=R
  child-2: another Layer-1 session
```

Proposed fields added to `SessionInner` (existing fields retained):

```rust
pub(crate) root_session: Option<SessionId>,
pub(crate) delegation_depth: u8,
pub(crate) child_id: Option<ChildId>,
pub(crate) child_coordinator: Mutex<Option<Weak<CoordinatorInner>>>,
```

`parent: Option<SessionId>` remains the immediate parent. Proposed accessors on
`SessionHandle` are `delegation_depth() -> u8`,
`root_session() -> Option<&SessionId>`, `child_id() -> Option<&ChildId>`,
`child_coordinator() -> Option<DelegationCoordinator>`, and
`has_pending_delegated_work() -> bool`. Keep the coordinator's strong ownership in
the host lifecycle, with only a Weak back-reference from the session.

Proposed lineage input to the internal `Runtime::start_child_session`:

```rust
pub(crate) struct ChildLineage {
    pub parent: SessionId,
    pub root_session: SessionId,
    pub delegation_depth: u8,
    pub child_id: ChildId,
}
```

`build_and_start` computes the root as
`parent.root_session().cloned().unwrap_or_else(|| parent.id().clone())`, increments
validated depth with checked arithmetic, and passes lineage before driver ability
activation, checkpoint recovery, or the first child turn. `derive_session` and
`new_session_execution_context` retain immediate parent attribution and use the
resolved depth for capability narrowing; Root never becomes a child by display
name and a child never becomes Root by loading a snapshot in isolation.

Persist versioned lineage in protected recovery state, with only redaction-safe
identity mirrors in ordinary snapshots. The parent catalog, root identity,
checkpoint identity, profile/policy fingerprint, and depth must agree. Recover an
old verified direct child as depth 1; a legacy root remains depth 0. Do not infer
an orphan's root authority or Layer-2 identity from an ID string. Incompatible,
missing, cross-root, or authority-widening lineage fails closed for execution;
retained permitted history may still be inspected without rebinding a provider.

## 4. Depth limits, IDs, capacity, and budgets

Extend `DelegationLimits` with `max_depth: u8`. The neutral default is 1; supported
host values are 1 or 2 with a hard upper bound of 2 in this change. Smith's
`wire_delegation_with_wait_policy` explicitly uses 2 for Root and eligible
Layer-1 coordinators. It does not expose arbitrary recursion via `[jev]`.
Every spawn/rebind/follow-up/resume/admission guard uses validated lineage and:

```rust
if parent.delegation_depth() >= config.limits.max_depth {
    return Err(depth_violation(config.limits.max_depth));
}
```

Only a coordinator-owning parent may spawn. Preserve host authorization after tool
filtering: injected `agent` calls at depth 2 are rejected even when no schema was
presented. A Layer-1 route with `delegation = false` has no coordinator/tool and
cannot gain one from Jev output, a descendant profile, a model prompt, or recovery.

`mint_child_id` uses the parent's durable `child_id` and an incrementing local
counter: Root mints `child-{n}`, a lead mints `{parent_child_id}.{n}`. Counter
reservation and retained records survive restart; IDs are never reused after
failure or pruning within that session's lifetime. Validate bounded depth and
numeric suffixes, but authorize by catalog/lineage ownership, not prefix matching.

Create one process-scoped `Arc<DelegationCapacity>` and share that exact Arc with
all Root/Layer-1 coordinators, including separate roots in the same process.
Keep existing per-parent running/retained/queue caps. Root is not itself a child
slot; Layer-1 and Layer-2 live bindings both acquire leases. Parked leads do not
release and reacquire their lease to evade the cap. Defaults retain bounded reject
semantics at exhaustion; a queued policy must remain bounded/deadline-aware and
cannot strand all available slots in parents waiting for unadmittable children.
Test saturation and report capacity failure instead of parking indefinitely.

Apply parent/subtree turn, token, deadline, workspace, posture, approval, and tool
upper bounds before each descendant starts. DelegationAuthority authorizes
`agent.delegate` at both levels; it is not an exemption from the parent's
read-only scope. Descendant usage has one canonical source identity, attributed
both to that child's own counters and to ancestor budget enforcement without
being charged twice to run totals. Forwarding a lifecycle event is not a new usage
delta. Budget exhaustion cancels/settles affected descendants under existing policy.

## 5. Depth-aware factory and lifecycle wiring

Retain `ChildRuntimeFactory::child_builder` for existing consumers. Proposed
object-safe extension shapes are:

```rust
pub struct ChildRuntimeBuild {
    pub builder: RuntimeBuilder,
    pub delegation_slot: Option<Arc<OnceLock<DelegationCoordinator>>>,
}

pub type ChildStartFuture<'a> = Pin<Box<
    dyn Future<Output = Result<(), RuntimeError>> + Send + 'a
>>;

fn child_builder_at_depth(
    &self,
    spec: &ChildSpec,
    depth: u8,
    max_depth: u8,
) -> Result<ChildRuntimeBuild, RuntimeError>;

fn on_child_session_started<'a>(
    &'a self,
    parent: &'a SessionHandle,
    child: &'a ChildId,
    child_session: &'a SessionHandle,
    depth: u8,
    slot: Option<Arc<OnceLock<DelegationCoordinator>>>,
) -> ChildStartFuture<'a>;
```

Defaults call the old builder with `delegation_slot = None` and return a ready
successful no-op post-start future. This avoids forcing nested delegation on
neutral consumers. Equivalent object-safe async-trait signatures are acceptable
only through an explicit upstream compatibility review.

`SmithChildRoute` gains resolved `delegation: bool`, `direct_interaction: bool`,
and `wait_policy: DelegationWaitPolicy`, with sourced profile values preserved in
configuration. `SmithChildFactory` gains the optional root
`Arc<dyn InteractionBroker>`, shared capacity, the effective child profile roster,
and a Weak reference to a root-owned `NestedDelegationRegistry`.

For `depth == 1 && depth < max_depth && route.delegation`,
`SmithChildFactory::child_builder_at_depth` creates a fresh OnceLock slot and local
`AgentWaitTimers`, adds `AgentTool` with preflighted profiles, wait policy, and
transcript output, includes DELEGATION guidance, and installs
`DelegationAuthority` under the same narrowed `SecurityCheckSet` used at Root.
Existing provider pools, model fallback chains, harness TurnExecution, module
composition, capability limits, and read-only scope remain unchanged.

The proposed post-start sequence in `DelegationCoordinator::build_and_start` is:

1. Reserve ID/capacity and validate parent lineage and the selected route.
2. Obtain the depth-aware build and apply requested tool/workspace restrictions.
3. Strip `delegation_tool_names` if `child_depth >= max_depth` or delegation is
   disabled for the route; a profile cannot add a tool outside the upper bound.
4. Resolve child interaction disposition without clobbering explicit host-approved
   DirectHost; build the runtime and start/resume the lineage-bound session.
5. Invoke `on_child_session_started` **before any initial, resumed, or follow-up
   model turn**. For a lead, call `wire_delegation_with_wait_policy`, recover its
   local Layer-2 catalog/outcomes, set its slot, and start the ordinary
   `DelegationLifecycle` event/admission tasks.
6. Publish ready lifecycle state and dispatch the task only after wiring succeeds.
   Hook failure must cancel/drain the partial session, release leases once, and
   report a typed error; never run a child with an uninitialized agent tool.

The root-owned registry retains each lead's lifecycle/coordinator handles and
shuts them down in descendant-first bounded order. Factory references back to it
are Weak so `factory -> registry -> coordinator -> factory` is not a strong Arc
cycle. Release locks before awaiting recovery, nested lookups, or shutdown.

## 6. Logical task episodes and the parking barrier

### 6.1 Current trap

The existing collector at `delegation/monitor.rs:7-85` awaits exactly one
`TurnHandle`. Simply enabling nested tools lets a lead spawn `child-1.1`, finish
its first turn to park, and have that interim text recorded as final completion
by Root. Later internal lead turns are not followed by that one-shot collector.
This is a correctness defect in a naive depth-only implementation, not evidence
that an unmodified depth-one runtime currently runs grandchildren.

### 6.2 Episode identity and reliable turn following

One spawn/follow-up/resume creates a logical `TaskEpisodeId { child, generation }`.
Its initial turn plus all attributed descendant-completion and wait-timeout turns
belong to that episode until completion, escalation, or a terminal error. A later
explicit follow-up creates a new generation; an old monitor cannot complete it.

Add a retained, sequence-indexed turn-admission/outcome ledger keyed by episode
and turn identity. Proposed `subscribe_task_episode_turns` and
`wait_next_turn_or_delegation_settled` APIs expose a cursor/subscription to the
collector; names and exact retained record representation are upstream APIs to
finalize, not assumed existing methods. Register before the initial send and
record a turn before execution can finish. A watch/Notify revision signals changes
but is never the sole copy of a handle/outcome. Recover terminal evidence from
protected checkpoints rather than replaying provider execution. Bound ledger
retention by active episode readers and existing retained-task limits.

### 6.3 Pending-work predicate and terminal commit

`DelegationCoordinator::has_pending_child_work()` and
`SessionHandle::has_pending_delegated_work()` are observational. Pending includes:
queued/admitted spawns, active descendant episodes, uncommitted terminal
persistence, protected outcomes ready for local delivery, continuation admission
in flight, accepted/queued episode continuations, and relevant non-stale wait
wakeups. A completed current turn must not keep itself pending forever merely
because its generic active-turn counter has not yet been decremented.

Do not consume or acknowledge outcomes in this predicate. Do not conflate
ancestor observability of grandchildren with ownership of their ready outcomes.
Use the local admission gate plus ledger revisions to subscribe-before-check and
atomically validate quiescence before committing the final episode result. A
racing terminal outcome or continuation must either be observed by the collector
or serialized after a committed terminal generation and rejected as stale.

The collector loop has these semantics:

```text
await initial/current episode turn outcome
  NeedsInput -> persist exact returned request to immediate parent; end episode
  Failed/Cancelled/Shutdown -> persist terminal outcome; bounded subtree cleanup
  LimitReached -> enforce existing limit; never extend episode budget
  Completed + pending delegated work -> retain Running; follow next episode turn
  Completed + atomically verified quiescence -> transfer final result once; Idle
```

A completed parked turn emits ordinary per-turn observability, but not the
ancestor's final `ChildCompleted` task outcome. In Root's coordinator `child-1`
remains `Running`; it may have no active provider call while parked. Once Layer-2
finishes, Layer-1 integrates it in an internal turn, and only Layer-1's final
quiescent answer is transferred to Root with the exact final turn ID. Preserve
artifact ownership transfer, excerpt/reference rules, and commit-time outcome
acknowledgement. `Completed` is a task outcome; the existing terminal-success
lifecycle state remains `Idle`.

A NeedsInput boundary suspends automatic lead continuations until the exact
interaction is resolved or explicitly abandoned. Other worker outcomes remain
protected, not discarded or allowed to run the lead ahead of the user's answer.
An authorized follow-up binds those pending outcomes to the new task generation;
stale collectors and timers from the earlier generation cannot complete it.

### 6.4 Failure/stopped wakeup

Extend the versioned protected outcome identity/value ledger with failure and
stopped/cancelled outcomes, identified by child, task generation, and terminal
turn/control identity. Exact errors stay protected; model projections are bounded
and redacted. Add `ChildTaskOutcome::Failed` / `Stopped` (or an equivalent typed
terminal sum) and corresponding `ChildOutcomeIdentity` cases upstream; update
serialization, restore, deterministic order, inspection, and acknowledgement.

A Layer-2 failure or stop is admitted to its immediate Layer-1 parent through
`try_admit_child_completion_if_idle`, subject to real-user priority, cancellation,
expected cursor, and existing Busy/Stale/Shutdown/Conflict handling. It must wake
that lead even if no successful result exists. The lead can recover or report
failure; Root does not receive the old parking prose as a successful synthesis.
Root's task episode remains running while this integration is pending. Multiple
failure/progress broadcasts cannot create duplicate continuations.

## 7. Interaction: DirectHost versus ReturnToParent

`RuntimeBuilder::allow_child_interaction(true)` already exists. Today the
coordinator subsequently calls `return_child_interactions_to_parent`, resetting
that choice. Replace the unconditional reset with an explicit effective
interaction policy that respects a host-approved depth-1 route and supplied
broker; preserve ReturnToParent by default. A builder flag alone, repository
profile text, or absent broker must not grant user interaction.

- `DirectHost`: only designated depth-1 clarifier/lead, effective
  `direct_interaction = true`, an available root broker, and host/tool policy all
  agree. `SmithChildFactory` receives `request.interaction.clone()` through root
  factory construction and supplies the same broker plus
  `.allow_child_interaction(true)` to that route.
- `ReturnToParent`: all ordinary Layer-1 and Layer-2 workers. Exact `ask_user`
  requests return as protected `NeedsInput` to the immediate parent.
- Unavailable/headless direct surface: use typed escalation/input-required
  semantics; never wait forever or create an invisible TUI modal. Layer 2 remains
  ReturnToParent even if it inherits `direct_interaction = true`.

Layer-2 -> Layer-1 -> Root escalation is explicit. Layer-1 may answer a worker
from known task context, or ask its own parent and later use `agent follow_up` to
send the answer down. DirectHost on a lead shortcuts only presentation; it does
not change child ownership or grant approval. Include depth-aware QUESTIONNAIRE
and DELEGATION prompt fragments and remove root-only escalation language in
`prompt.rs`, `delegation/tool.rs`, and capability eligibility decisions.

Root serializes all authorized prompts through a bounded modal queue. Identity is
`(origin session, turn, tool call, InteractionRequestId)`, not the currently
selected child row. Show the originating `child-1` label, preserve the composer
and transcript focus, close/cancel the exact request on stop/shutdown, and restore
pending prompts through existing checkpoint rules. A task answer is never tool
approval, workspace trust, or permission to widen a scope. Distinct simultaneous
requests must not overwrite one another merely because IDs are unique.

## 8. Event forwarding, authorized tree reads, and persistence

`spawn_monitor` forwards the six canonical ChildSpawned/Progress/NeedsInput/
Completed/Stopped/Failed variants from a lead's stream to Root. Preserve original
child/session/turn ownership and a source event identity for idempotent replay;
if canonical event additions are needed, add them upstream with version gates.
Do not invent Smith-local RuntimeEvent variants. Root projection can see the
worker's activity without copying its full tool transcript into Root's model
history, and without putting its outcome into Root's automatic delivery queue.
The worker's terminal outcome belongs to Layer-1; only Layer-1's integrated
outcome belongs to Root. Apply redaction at the same child-history boundary.

`list()` returns authorized descendant snapshots, while internal `list_direct()`
(or equivalent private iteration) preserves per-parent capacity/mutation
semantics. `status()`, `child_events()`, `with_child_history()`, `child_history()`,
and `subscribe_child_history()` resolve the owning coordinator, clone handles out
of locks, and recurse at most to the configured depth. Root can inspect its
subtree, a lead only its own workers, and no session can cross roots. Live reads
join canonical retained history without gaps/duplicates; dormant reads traverse
validated durable catalogs and stores **without** starting a runtime/provider.
Unknown, pruned, or unavailable history is explicit. Pagination and artifact
ownership/redaction retain their current contracts.

Spawning and model follow-up/resume remain direct-parent control. Root host
operator stop/shutdown may explicitly traverse its subtree through each owning
coordinator, never by moving entries into Root's catalog. A Layer-1 request for a
sibling's `child-2.1` is refused even if its prefix is syntactically valid.

Keep `agent-runtime.delegation.children` and
`agent-runtime.delegation.child-outcomes` partitioned within each owning parent's
snapshot; no namespace renaming is required solely to avoid collisions. Version
new lineage/episode/failure records and fingerprint execution/interaction policy.
Recover Root -> eligible Layer-1 -> its catalog before admitting lead work.
Preserve encrypted exact questionnaires and checkpoints; ordinary journals remain
redaction-safe. Extend `FileSessionStore::list` catalog ownership filtering for
nested and partially recovered trees; children are reached through their parent,
not exposed as independent root resume candidates.

## 9. Smith client, TUI, CLI, and headless behavior

`smith-client` agent report projection and `/agent` consume the same authorized
hierarchical status/history APIs. Preserve existing lowercase lifecycle words and
machine-readable headless contracts; extend versioned identity metadata rather
than replacing status with presentation strings.

The delegated-work panel groups numeric hierarchical IDs in parent-before-child
order and indents depth-2 rows beneath their lead. Profile/workspace/activity facts
come from each child's own identity, not the root's pending spawn arguments.
In `app/reducer.rs:770-829`, pop `pending_spawns` only for an event correlated to a
root-owned pending call. For this ID scheme the minimum guard excludes IDs with
`.`; prefer explicit source parent/call identity so an out-of-order event cannot
steal another pending spawn. Preserve child navigation, clocks, paging, reasoning
visibility, exact tool pairing, and root composer state.

`has_required_child_work` must include the full task-episode barrier, queued or
pending descendants, in-flight terminal admission, and final delivery cursor.
It remains true while Layer-1 is parked for Layer-2 and until final Root delivery
settles. A stale wait timer, cache-retention policy, retained idle child record,
or forward-only lifecycle event must not keep a completed headless run alive.
Shutdown cancels admission, questionnaires, timers, and descendants in bounded
order and releases shared capacity exactly once. External-agent/harness profiles
obey the same two-layer authorization and internal-turn execution contracts.

## 10. Configuration and SystemOne contract

Proposed config shapes in `smith-config` are `JevSection`, `ResolvedJevConfig`, and
`ProfileSection::direct_interaction: Option<bool>`. Extend
`ResolvedAgentProfile` with `direct_interaction: Sourced<bool>`; keep sourced
`delegation` and carry both through inheritance, profile revision/fingerprint,
preflighted `SmithChildRoute`, config explain/provenance, and safe reconfiguration.
Jev settings are top-level run policy, not model-generated parameters.

Defaults/validation: disabled; endpoint
`http://127.0.0.1:8765/v1/systemone`; timeout 1500 ms, allowed 1..=10,000;
actionability threshold 0.5 and normalized choice-confidence threshold 0.8, both
finite [0,1]; clarifier profile `clarifier`; optional `bearer_credential_ref`.
Omitted DirectHost opt-in is false. Profile delegation still defaults true but is
intersected with depth/host/tool scope. A Layer-1 delegation-disabled route stays
a leaf. Static invalid configuration is a preflight diagnostic, not a reason to
silently ignore typos; transient service failures are runtime fail-open events.

Only user-owned effective configuration may enable an endpoint, select a bearer
credential, or authorize non-loopback prompt disclosure. Repository config can
narrow/disable approved behavior, not self-authorize new network destinations.
No redirects; bounded connect/read/total timeout, request/response size, and
cancellation. Proposed initial body cap is 64 KiB; reject/fallback rather than
silently truncate a user message. Send only the bounded user text and necessary
role descriptions; not credentials, hidden instructions, whole transcripts,
repository contents, or image assets. Unsupported multimodal inputs retain their
attachments through ordinary root fallback. Hosted official `jev` is opt-in and
must pass adapter contract tests; it was not exercised by the local benchmark.

### 10.1 Request

`POST /v1/systemone`, `Content-Type: application/json`, optional
`Authorization: Bearer <resolved secret>`. This is not `/v1/chat/completions`.
The following is the proposed combined request. The inverse head's exact wording
comes from the successful follow-up and the other heads from primary D. **This
new combination has not been benchmarked end to end**; it needs held-out tests.
Profile criteria at runtime are drawn only from effective child-enabled,
preflighted, host-eligible profiles, with bounded one-line descriptions.

```json
{
  "state": "In crates/smith-config/src/model.rs, change only the Display string for ProfileUse::Child from \"child\" to \"subagent\". Do not change the variant name or parsing behavior.",
  "questions": {
    "is_actionable": {
      "type": "noul",
      "instructions": "Is this user request sufficiently specific to begin useful task-specific work without asking the user a clarifying question? Judge only this message; you may inspect the repository but have no previous conversation.",
      "criteria": {
        "true": "Clear and actionable: an exact edit or test command, a concrete symptom to investigate, or a research topic with an objective.",
        "false": "Underspecified: a missing target or outcome, an unresolved it/this, or vague fix/refactor/cache/optimization request without necessary scope."
      }
    },
    "complexity": {
      "type": "score",
      "instructions": "Estimate the intended engineering scope, not prompt length or safety risk. Use the ordered 1..5 rubric: 1 is one literal edit or focused test command; 2 is a small edit/test or bounded inspection; 3 is a bounded feature/debugging task or substantive research; 4 is a cross-component refactor or coordinated audit; 5 is new multi-agent/distributed architecture. If ambiguous, estimate the likely intended scope provisionally; do not call every unclear request trivial.",
      "criteria": [
        {
          "label": "1_trivial",
          "description": "One explicit literal/text change or focused verification command; no design."
        },
        {
          "label": "2_simple_edit",
          "description": "A small well-specified edit or unit test, or a bounded read-only inspection."
        },
        {
          "label": "3_moderate_feature",
          "description": "A bounded feature/debugging task or substantive investigation needing multiple steps and tests."
        },
        {
          "label": "4_complex_refactor",
          "description": "A cross-component refactor, difficult debugging, or broad audit needing coordinated reasoning."
        },
        {
          "label": "5_multi_agent_architecture",
          "description": "A new multi-agent or distributed architecture with multiple independently built subsystems."
        }
      ]
    },
    "needs_subagents": {
      "type": "noul",
      "instructions": "Should the lead agent spawn multiple independent worker agents now? True requires adequately specified work with separable substantial workstreams or an explicit request to delegate parallel tasks. False for a single focused task, ordinary repository exploration, or a request blocked on essential clarification. Complexity or the mere word agent alone is insufficient.",
      "criteria": {
        "false": "Do not spawn now: a single coherent task, lightweight work, or blocked clarification.",
        "true": "Spawn workers now: sufficiently specified independent substantial workstreams or explicit parallel delegation."
      }
    },
    "target_profile": {
      "type": "choice",
      "instructions": "Select the best lead profile using the actual Smith role definitions. code: small well-specified edits and focused verification commands. sol: substantial implementation, debugging, refactoring, architecture. research: read-only investigations, comparisons, and explanations. visual: images/screenshots, UI layout, rendering, asset appearance. security: read-only vulnerability audit and threat modeling. Choose the requested kind of work, not a keyword: implementing authentication is sol; auditing authentication vulnerabilities is security. For ambiguous tasks choose the likely specialist; clarification is scored separately.",
      "criteria": {
        "code": "Simple coder: small, well-specified code edits, focused tests, and explicit verification commands.",
        "sol": "Substantial implementation, debugging, refactoring, and architecture; use for complex coding changes.",
        "research": "Read-only investigation of code/docs/options, explanations, and comparative research; not a direct run-tests command.",
        "visual": "Work depending on seeing screenshots/images, layout, rendering, visual regressions, or asset appearance.",
        "security": "Read-only vulnerability audits, exploit tracing, threat modeling, and finding triage; not general auth implementation."
      }
    }
  }
}
```

Do not request a specific model and assume the server enforces it: local 0.5.0
accepted `model = nonexistent-model-name` and returned its loaded model. Validate
response model/backend identity against the endpoint's negotiated/advertised
configuration and record what actually answered. Reject a fake/untrained local
engine for production triage. Unknown/incompatible service contracts fall back;
no stock-Ollama chat/logit heuristic is substituted.

### 10.2 Response and numeric interpretation

This response is **illustrative arithmetic, not a captured benchmark observation**:

```json
{
  "model": "jev-style-2b-decision-v3",
  "backend": "mlx",
  "answers": {
    "is_actionable": {
      "type": "noul",
      "noul": 0.94
    },
    "complexity": {
      "type": "score",
      "score": 0.41,
      "confidence": 0.6583333333333333,
      "legend": {
        "0": "1_trivial",
        "1": "2_simple_edit",
        "2": "3_moderate_feature",
        "3": "4_complex_refactor",
        "4": "5_multi_agent_architecture"
      },
      "probabilities": {
        "0": 0.72,
        "1": 0.19,
        "2": 0.06,
        "3": 0.02,
        "4": 0.01
      }
    },
    "needs_subagents": {
      "type": "noul",
      "noul": 0.04
    },
    "target_profile": {
      "type": "choice",
      "choice": "code",
      "confidence": 0.825,
      "probabilities": {
        "code": 0.86,
        "sol": 0.08,
        "research": 0.03,
        "visual": 0.02,
        "security": 0.01
      }
    }
  },
  "usage": {
    "output_tokens": 0
  }
}
```

Required answers and types must match the four requested IDs. Validate finite
probabilities in [0,1], nonempty expected option sets, probability sums within a
specified floating tolerance (initially 1e-5), recognized model identity, exact
five-level legend/order, consistency of choice with argmax, and score arithmetic.
Optional diagnostics/usage fields may be accepted without affecting authority;
malformed required fields or materially inconsistent confidence fail open.

For `noul`, the scalar is P(true); infer its complementary probability as 1-p.
For inverse actionability use `p_clarify = 1 - answers.is_actionable.noul`.
Clarification is suggested when `P(actionable) <= actionability_threshold`;
exact threshold ties therefore prefer clarification.
It is not selected by interpreting API confidence as P(true).

For complexity with ordered probabilities p0..p4:

```text
raw API score = sum(i * p_i), i=0..4
expected complexity on 1..5 = raw API score + 1
exact class = first argmax(p_i) + 1
choice confidence = clamp((k * p_max - 1) / (k - 1), 0, 1)
```

With k=1 define choice confidence as 1. With five options and pmax=.86, confidence
is .825, not .86. Validate the returned confidence against that rule. Score
confidence is the adapter's ordinal-spread metric, not P(exact class): for n=5,
mode m, spread=sum(p_i*abs(i-m)), even_spread=sum(abs(i-2))/5=1.2, confidence is
clamp(1-spread/even_spread,0,1). Noul has no separate API confidence field.
Do not round the raw score to implement a 1..5 class; the benchmark's architecture
example had raw score 3.257994, expected 4.257994, and argmax class 5.

`JevTriageClient` returns proposed typed `JevDecision { model, p_actionable,
p_clarify, complexity_expected, complexity_class, complexity_probabilities,
p_subagents, target_profile, profile_probabilities, profile_confidence,
wall_latency, service_latency }`. These are advisory data; never deserialize a
child specification, tool invocation, permission, workspace, or credential from
model output. Low profile confidence falls back even if argmax names an eligible
profile. Complexity and sub-agent need are handoff hints, not worker-spawn grants.

## 11. Shared pre-turn admission and fail-open state machine

Place triage in proposed `HostSession::submit_user_input` / `host/admission.rs`,
consumed by native interactive, headless, and supported embedded Smith surfaces.
Direct `SessionHandle::send` remains the lower-level runtime API; migrate stock
user-submission call sites (including headless `run_flow.rs:114`) to the shared
host admission path. Never put this only in the TUI reducer or child factory.

A real-user submission gets one host admission identity, cancellation token,
canonical input payload, and immutable config/profile revision. The optional
Jev HTTP request precedes provider dispatch. Successful host routing needs an
upstream/host-supported **provider-free, authorized control admission** that
records the real user message once and starts a normal delegated episode under
the same `agent.delegate` workspace/approval checks as the tool path. Add a typed
runtime host-control admission API if existing hooks cannot provide this; do not
mutate SessionState directly, fabricate assistant tool calls, or invoke the raw
coordinator behind the security boundary. This is a Stage-2 upstream gate.

```text
Disabled / ineligible input -> OrdinaryRoot
Eligible real-user root input -> Querying(request_id, input fingerprint, config revision)
Querying -> cancelled/shutdown: Cancelled (no fallback execution)
Querying -> timeout/unavailable/HTTP/shape/identity error: OrdinaryRoot(reason)
Valid -> P(actionable) at or below threshold: authorized Clarifier, else OrdinaryRoot
Valid -> actionable + eligible confident profile: authorized Layer1Lead
Valid -> low confidence / missing route / disabled delegation: OrdinaryRoot(reason)
Authorized dispatch rejected by host policy: normal denial/approval path, not a bypass
Clarifier -> exact task answers + concise handoff -> authorized Layer1Lead or OrdinaryRoot
Layer1Lead -> park while descendants run -> final integrated outcome -> Root delivery
```

Fallback must preserve the original input/attachments exactly once. Re-check the
admission identity, cancellation, and config revision before committing either
path; a late HTTP response cannot spawn another child after fallback. No
unbounded retry, recursive triage, or silent polling. Bounded optional diagnostics
identify disabled/timeout/invalid/low-confidence/fallback without logging secrets
or adding fabricated model prose to the root transcript.

Do not triage internal child-completion/timer turns, child inputs, goal turns,
questionnaire answers as standalone submissions, or local slash/shell commands.
A clarifier completion may produce at most one explicit bounded handoff using the
original request plus exact task answers; it does not recursively trigger new
clarifiers. The clarifier is a depth-1 leaf with a narrowing posture; after it
settles, Root dispatches a separate depth-1 lead so that lead still has depth-2
workers available. Settle/retire the earlier binding under normal retention policy
rather than leaking slots across successive tasks. Final Root delivery may use
its existing internal synthesis turn, but detailed implementation history stays
in inspectable children. Root remains the canonical user/authorization anchor.

Fail-open means bypassing advisory triage when it is unavailable or uncertain.
It never means bypassing approval, creating a child after cancellation, widening
a workspace, interpreting a questionnaire answer as consent, or sending prompts
to a newly discovered remote service. A low-confidence route returns to the
configured Root profile, not an arbitrary stronger/privileged model.

## 12. Tests, rollout, and acceptance gates

Use deterministic fake providers/HTTP endpoints, shared runtime testkit clocks,
barriers, journals, and protected stores. Required interleavings include:

- depth 0->1->2 succeeds only when eligible; 2->3 rejects even via injection;
  disabled Layer-1 delegation omits both tool and prompt;
- unique hierarchical IDs and retained counters across two leads/restart;
  shared process-wide capacity under two simultaneous roots;
- lead turn-1 parks; worker succeeds; lead turn-2 integrates; exactly one final
  result reaches Root; no premature completion/exit or repeated provider turn;
- worker completes before lead's parking event, immediately after subscription,
  during terminal persistence, and in multiple rapid internal turns; no lost wake;
- sole/all workers fail/stop; lead wakes once; cancellation/budget/shutdown settle
  the tree without using interim parking prose as success;
- exact NeedsInput at both levels, direct clarifier modal, concurrent modals,
  cross-request responses, headless unavailable broker, resume, and cancellation;
- descendant list/status/history reads across live/dormant records, denied sibling
  roots, pagination/redaction, artifact ownership, and no provider execution;
- forwarded events are attributed once, usage totals are not doubled, pending
  root-spawn correlation survives interleaved grandchild events, numeric tree
  ordering handles child-2/child-10, and headless waits for final Root delivery;
- HTTP disabled/timeout/malformed/non-finite/wrong-model/fake/low-confidence/401/
  unavailable/oversized/redirect/cancel cases; every fallback preserves one input
  and existing approval checks; no raw secrets or unsupported asset disclosure;
- zero-based score/ordinal confidence arithmetic and actual eligible profile
  descriptions/roster; singleton versus combined-request fixture validation.

Release upstream runtime APIs plus shared consumer conformance first, then update
all Smith runtime crate pins together and run Smith consumer tests. Rebase this
change on immediate-return wait monitoring and existing fallback/harness work.
Keep `[jev].enabled = false` by default. Validate the newly combined request on a
held-out suite with more clarification and delegation positives before any
rollout; report recall, false positives, calibration, coverage, and latency, not
only overall accuracy. No runtime test or deployment is claimed by Stage 1.

## 13. Alternatives and remaining limits

Depth-only enablement is rejected: it loses multi-turn lead synthesis. A separate
Smith scheduler/child outcome loop is rejected: it duplicates runtime durability
and authorization. DirectHost for every worker is rejected: it loses parent
ownership and creates competing user interactions. Text-generating Ollama chat
as a probability endpoint is rejected without an independently tested adapter.
Unbounded delegation is outside scope.

The original experiment was prompt-only, manually labeled, and reused cases for
phrasing selection; the better inverse result has only eight positives. Longer
criteria improved some heads and worsened others. The selected combined request,
concurrent load, official hosted services, GGUF, and actual two-layer execution
were not validated by that experiment. These remain explicit Stage-2 gates.

## Appendix A. Complete child-1 engineering blueprint (preserved research)

The following is the complete final child-1 blueprint recovered from its local
retained session, not a claim that these changes already exist. Earlier line
numbers and sketches are preserved as evidence; the reconciliations and normative
constraints above take precedence. In particular, do not copy its missing
`turns.active` field, latest-turn-only watch sketch, or settled-after-failure
success path into an implementation.

Blueprint SHA256: `462ca007b3a21c224fbed189d7715c283e24c0dfc12ce488d7eb31be926985ce`.

````markdown

Here is the complete, line-by-line engineering blueprint for **Design B (2-layer delegation: `User -> Root / Jev -> Layer-1 Agent -> Layer-2 Sub-Agents`)** across `agent-runtime` (`/Volumes/Data/codes/ai/agent-runtime`) and `smith` (`crates/smith-config`, `crates/smith-runtime`, `crates/smith-client`, `crates/smith-tui`, `crates/smith-cli`).

---

# Section 1: `agent-runtime` — Every Depth-1 Assumption, Storage Key, and the Layer-1 / Layer-2 Parking Trap

## 1.1 `SessionHandle` & `SessionInner` Lineage and Delegation Depth

### Current State
1. **`SessionInner`** (`/Volumes/Data/codes/ai/agent-runtime/crates/agent-runtime/src/runtime/session/mod.rs:56-62`):
   ```rust
   pub struct SessionInner {
       pub(crate) shared: Arc<RuntimeShared>,
       pub(crate) id: SessionId,
       /// The parent session, when this session is a delegated child. A child
       /// session must never spawn children of its own (depth-one enforcement).
       pub(crate) parent: Option<SessionId>,
   ```
2. **`SessionHandle::parent(&self)`** (`/Volumes/Data/codes/ai/agent-runtime/crates/agent-runtime/src/runtime/session/lifecycle.rs:50-52`):
   - Returns `Option<&SessionId>` (only the immediate parent).
3. **`Runtime::start_child_session` & `start_session_with_parent`** (`/Volumes/Data/codes/ai/agent-runtime/crates/agent-runtime/src/runtime/engine.rs:1379-1393`):
   - Takes `parent: Option<SessionId>` with no depth or root-ancestor tracking.
4. **`LiveAbilitiesRuntime::derive_session`** (`/Volumes/Data/codes/ai/agent-runtime/crates/agent-runtime/src/agent/driver/mod.rs:664-710`):
   - Takes `parent: Option<SessionId>` to scope live ability activation.

### Required Changes for Design B
1. **Add `delegation_depth: u8` and `root_session: Option<SessionId>` to `SessionInner`** (`crates/agent-runtime/src/runtime/session/mod.rs:59-62`):
   ```rust
   pub(crate) parent: Option<SessionId>,
   pub(crate) root_session: Option<SessionId>,
   pub(crate) delegation_depth: u8,
   /// Weak or Arc handle to this session's own DelegationCoordinator when it
   /// is a Layer-1 parent managing Layer-2 children (used by the turn-completion
   /// hold barrier in Section 1.4).
   pub(crate) child_coordinator: Mutex<Option<Weak<CoordinatorInner>>>,
   ```
   - Root session: `parent = None`, `root_session = None`, `delegation_depth = 0`.
   - Layer-1 child: `parent = Some(root_id)`, `root_session = Some(root_id)`, `delegation_depth = 1`.
   - Layer-2 grandchild: `parent = Some(layer1_id)`, `root_session = Some(root_id)`, `delegation_depth = 2`.
2. **Expose on `SessionHandle`** (`crates/agent-runtime/src/runtime/session/lifecycle.rs:50-55`):
   - `pub fn parent(&self) -> Option<&SessionId>` (immediate parent).
   - `pub fn root_session(&self) -> Option<&SessionId>` (`self.inner.root_session.as_ref()`).
   - `pub fn delegation_depth(&self) -> u8` (`self.inner.delegation_depth`).
3. **Update `Runtime::start_child_session`** (`crates/agent-runtime/src/runtime/engine.rs:1379-1393`):
   - Accept `(request: StartSession, parent: SessionId, root_session: SessionId, delegation_depth: u8)` (or a `ChildLineage { parent: SessionId, root: SessionId, depth: u8 }` struct).
   - In `DelegationCoordinator::build_and_start` (`crates/agent-runtime/src/delegation/lifecycle.rs:1177-1179`), compute:
     ```rust
     let parent_id = self.inner.parent.id().clone();
     let root_id = self.inner.parent.root_session().cloned().unwrap_or_else(|| parent_id.clone());
     let child_depth = self.inner.parent.delegation_depth().saturating_add(1);
     let handle = runtime.start_child_session(start, parent_id, root_id, child_depth).await?;
     ```

---

## 1.2 `DelegationLimits::max_depth`, `check_depth()`, and `ChildId` Hierarchy

### Current State
1. **`DelegationLimits`** (`/Volumes/Data/codes/ai/agent-runtime/crates/agent-runtime/src/delegation/types.rs:55-63, 477-485`):
   - Fields: `max_running_children: usize`, `max_retained_children: usize`, `retention_ms: Option<u64>`. No `max_depth`.
2. **`DelegationCoordinator::new` & `check_depth`** (`crates/agent-runtime/src/delegation/coordinator.rs:13-15`, `lifecycle.rs:777-782`, `monitor.rs:557-562`):
   - Hardcodes `if parent.parent().is_some() { return Err(depth_violation()); }`.
   - Called at:
     - `DelegationCoordinator::new` (`coordinator.rs:13`)
     - `DelegationCoordinator::spawn` (`coordinator.rs:333`)
     - `DelegationCoordinator::try_admit_child_completion_if_idle` (`coordinator.rs:836`)
     - `DelegationCoordinator::follow_up` (`lifecycle.rs:137`)
     - `DelegationCoordinator::resume` (`lifecycle.rs:239`)
     - `DelegationCoordinator::stop` (`lifecycle.rs:366`)
3. **Flat `ChildId` Minting** (`crates/agent-runtime/src/delegation/lifecycle.rs:800-803`):
   ```rust
   pub(super) fn mint_child_id(&self) -> ChildId {
       let n = self.inner.next_child.fetch_add(1, Ordering::SeqCst) + 1;
       ChildId::new(format!("child-{n}"))
   }
   ```
   - If Root mints `child-1` and `child-1` also mints `child-1`, their `ChildId`s collide on any root-aggregated event stream, usage map, or TUI panel!
4. **Tool Stripping in `DelegationCoordinator::build_and_start`** (`crates/agent-runtime/src/delegation/lifecycle.rs:1125-1129`):
   - Always strips `self.inner.config.delegation_tool_names` (`"agent"`) from every child regardless of depth.

### Required Changes for Design B
1. **Add `max_depth: u8` to `DelegationLimits`** (`crates/agent-runtime/src/delegation/types.rs:55-63, 477-485`):
   - Default `max_depth: 1` for backward compatibility in neutral `agent-runtime`, configurable to `2` (or `HARD_MAX_DELEGATION_DEPTH = 2`).
   - Optionally add `child_id_prefix: Option<ChildId>` to `DelegationConfig` (`types.rs:406-425`) or derive it from the parent's `ChildId`.
2. **Replace `parent.parent().is_some()` in `DelegationCoordinator::new` and `check_depth()`**:
   - In `DelegationCoordinator::new` (`coordinator.rs:13-15`) and `check_depth(&self)` (`lifecycle.rs:777-782`):
     ```rust
     if parent.delegation_depth() >= config.limits.max_depth {
         return Err(depth_violation(config.limits.max_depth));
     }
     ```
   - Register the coordinator on the parent session: `parent.set_child_coordinator(Arc::downgrade(&self.inner))` so the parent session knows whether it owns active sub-agents.
3. **Hierarchical `ChildId` Minting** (`crates/agent-runtime/src/delegation/lifecycle.rs:800-803`):
   - Store `parent_child_id: Option<ChildId>` on `SessionInner` (passed via `start_child_session`).
   - In `mint_child_id(&self)`:
     ```rust
     pub(super) fn mint_child_id(&self) -> ChildId {
         let n = self.inner.next_child.fetch_add(1, Ordering::SeqCst) + 1;
         match self.inner.parent.child_id() {
             Some(prefix) => ChildId::new(format!("{prefix}.{n}")), // e.g., "child-1.1", "child-1.2"
             None => ChildId::new(format!("child-{n}")),           // e.g., "child-1", "child-2"
         }
     }
     ```
   - Why `child-1.1` (or `child-1/child-1`) is critical:
     - Within Layer-1's own coordinator (`children: BTreeMap<ChildId, ChildEntry>`), the key is `"child-1.1"`.
     - When Layer-1's lifecycle events are forwarded/projected to Root TUI (`App.children`, `App.child_conversations`, `App.child_clocks`, `App.child_counts`), `"child-1.1"` is globally unique across the entire session tree and naturally sorts right after `"child-1"` in `BTreeMap<String, ...>`!
4. **Conditional Tool Stripping in `build_and_start`** (`crates/agent-runtime/src/delegation/lifecycle.rs:1123-1130`):
   - Strip `delegation_tool_names` (`"agent"`) **only** when `child_depth >= self.inner.config.limits.max_depth` (i.e. always strip at Layer 2 so Layer-2 sub-agents cannot spawn Layer 3, and strip at Layer 1 only if the Layer-1 profile or spawn spec does not allow delegation).
5. **Shared Process-Wide Capacity Pool Across Both Layers**:
   - Pass a shared `Arc<DelegationCapacity>` (`types.rs:81-131`) in `DelegationConfig::shared_capacity` across Root's coordinator and every Layer-1 coordinator so total live Layer-1 + Layer-2 children cannot cause a fork bomb (e.g., `max_running_children = 4` per parent, `shared_capacity = 8` total).

---

## 1.3 `CHILD_CATALOG_NAMESPACE` & `CHILD_OUTCOME_CURSOR_NAMESPACE` in `persistence.rs`

### How Extension State Storage Works Today
1. **Namespaces** (`/Volumes/Data/codes/ai/agent-runtime/crates/agent-runtime/src/delegation/mod.rs:84-94`):
   - `pub const CHILD_CATALOG_NAMESPACE: &str = "agent-runtime.delegation.children";`
   - `pub const CHILD_OUTCOME_CURSOR_NAMESPACE: &str = "agent-runtime.delegation.child-outcomes";`
2. **Where they are stored** (`crates/agent-runtime/src/delegation/persistence.rs:836-853`, `coordinator.rs:38, 140`):
   - `self.inner.parent.persist_with_extension_state([(CHILD_CATALOG_NAMESPACE, ...), (CHILD_OUTCOME_CURSOR_NAMESPACE, ...)])` writes into **`self.inner.parent`'s own `SessionSnapshot.extension_state`** (which is persisted under `parent.id()` in `SessionStore` and `CheckpointStore`).
   - Because Root (`session-<root-uuid>`) and a durable Layer-1 child (`session-<layer1-uuid>`) have **distinct `SessionId`s** (`crates/agent-runtime/src/delegation/lifecycle.rs:942`), each session has its own `SessionSnapshot` in `SessionStore` and its own `TurnCheckpoint` in `CheckpointStore`!
   - Furthermore, both `ChildSessionRecord` (`types.rs:640`: `pub parent_session: SessionId`) and `ProtectedChildOutcomeState` (`types.rs:367`: `pub(super) parent: SessionId`) and `ChildOutcomeCursor` (`types.rs:203`: `parent: SessionId`) already validate `persisted.parent == *parent.id()` on load (`coordinator.rs:62-63, 155-162`)!

### Where Collisions / Bugs *Actually* Happen in `persistence.rs` & `smith-runtime` for Layer-2
1. **Smith's `FileSessionStore` Only Persists the Root Session Today!**
   - Inspect `crates/smith-runtime/src/host/startup.rs:144-169` and `crates/smith-runtime/src/host/persistence.rs`:
     - `SessionPaths` (`crates/smith-runtime/src/journal.rs`) maps a `SessionId` to `~/.smith/sessions/<project-hash>/<session-id>.snapshot.json` and `<session-id>.checkpoints/`.
     - When `SmithChildFactory` is constructed (`crates/smith-runtime/src/factory/construction.rs:703-707`), `session_store` is `request.session_store` (`RedactingSessionStore` wrapping `FileSessionStore`) and `checkpoint_store` is `durability.child_store` (`SqliteCheckpointStore` or `FileCheckpointStore`).
     - Because `FileSessionStore::save` keys files by `snapshot.id` (`session-<uuid>.snapshot.json`), a durable Layer-1 child **writes a top-level `session-<layer1-uuid>.snapshot.json` file into `~/.smith/sessions/<project-hash>/`**!
     - **Session Cleanliness Bug**: `FileSessionStore::list()` scans `~/.smith/sessions/<project-hash>/*.snapshot.json` to populate `/resume` and session listings! Currently, `SmithChildFactory::child_builder` (`crates/smith-runtime/src/delegation.rs:644-649`) attaches `self.session_store` and `self.checkpoint_store` to children. If `SessionSnapshot` for a child or grandchild is written to the same directory without parent-scoping or filtering, or if Layer-1 and Layer-2 children are durable, their snapshots must not pollute root session listings.
2. **Recursive Recovery Order in `DelegationCoordinator::recover()`** (`crates/agent-runtime/src/delegation/persistence.rs:110-260`):
   - Today, `DelegationCoordinator::new(&parent, ...)` reads `CHILD_CATALOG_NAMESPACE` and `CHILD_OUTCOME_CURSOR_NAMESPACE` from `parent.extension_state(...)` (`coordinator.rs:38, 140`).
   - When a Layer-1 child is rebound on cold resume (`DelegationCoordinator::bind_child` in `crates/agent-runtime/src/delegation/lifecycle.rs:458-738`), `build_and_start` resumes the Layer-1 `SessionHandle` from `SessionStore`, which loads Layer-1's `extension_state` containing Layer-1's own `CHILD_CATALOG_NAMESPACE` and `CHILD_OUTCOME_CURSOR_NAMESPACE` (its Layer-2 children!).
   - **Required Hook**: When `bind_child` / `start_child` starts a Layer-1 `SessionHandle` that is delegation-eligible (`child_depth < max_depth`), the child factory / post-start hook must wire a child `DelegationCoordinator` + `DelegationLifecycle` onto that Layer-1 `SessionHandle` and call `layer1_coordinator.recover().await?` **before** Layer-1's resumed turn or follow-up turn is dispatched!
   - Because `CHILD_CATALOG_NAMESPACE` (`"agent-runtime.delegation.children"`) and `CHILD_OUTCOME_CURSOR_NAMESPACE` (`"agent-runtime.delegation.child-outcomes"`) live inside each session's own `extension_state` map (and validate `record.parent_session == *parent.id()`), **keeping the exact namespace strings `"agent-runtime.delegation.children"` and `"agent-runtime.delegation.child-outcomes"` inside each parent's own `SessionSnapshot.extension_state` is 100% backward-compatible with existing depth-1 session recovery** while naturally partitioning Root's children (in Root's `extension_state`) from Layer-1's children (in Layer-1's `extension_state`).

---

## 1.4 CRITICAL TRACE: How Layer-1 Parks for Layer-2, and Why Root Prematurely Thinks Layer-1 Is Done Today!

Let's trace `spawn_returned_input_collector` (`crates/agent-runtime/src/delegation/monitor.rs:7-85`), `spawn_monitor` (`monitor.rs:90-309`), and `wait_task_outcome` (`coordinator.rs:776-816`) step by step when **Root spawns Layer-1 (`child-1`)**, and **Layer-1 spawns Layer-2 (`child-1.1`) and ends its turn to park**.

### Step-by-Step Trace of the Bug in Current `agent-runtime`
1. **Root spawns Layer-1 (`child-1`)**:
   - `root_coordinator.start_child("child-1", ...)` (`lifecycle.rs:1077`) calls `let turn = handle.send(spec.task)?;` (this is Layer-1's `turn-1`).
   - Immediately at `lifecycle.rs:1106`, Root calls:
     ```rust
     self.spawn_returned_input_collector(child.clone(), handle.clone(), turn);
     ```
2. **Layer-1 (`child-1`) runs `turn-1` and spawns Layer-2 (`child-1.1`)**:
   - During `turn-1`, Layer-1 calls `agent` `action: "spawn"` -> `layer1_coordinator.spawn(...)` starts `child-1.1`.
   - Following Smith's delegation instructions (*"After spawning, continue independent work or finish your turn and let the session park: completed child outcomes wake you automatically"*), Layer-1 emits text `"I have spawned child-1.1 to investigate X; parking until it completes."` and finishes `turn-1` with `FinishReason::Stop`.
3. **What happens when Layer-1's `turn-1` finishes to park?**
   - Layer-1's `TurnHandle` (`turn-1`) resolves `turn.outcome().await` with `(Some(TurnFinish::Completed), None)`!
   - In Root's `spawn_returned_input_collector` (`crates/agent-runtime/src/delegation/monitor.rs:16-50`):
     ```rust
     let (finish, returned) = turn.outcome().await;
     // ...
     (Some(TurnFinish::Completed | TurnFinish::LimitReached { .. }), None) => {
         match transfer_completed_result(
             &coordinator,
             &child,
             &handle,
             &turn_id,
             last_assistant_text(&handle),
         )
         .await
         {
             Ok(result) => {
                 record_completed_outcome(&coordinator, &child, turn_id, result).await
             }
             Err(error) => Err(error),
         }
     }
     ```
   - **DISASTER**:
     1. Root's `spawn_returned_input_collector` sees `turn-1` finish with `TurnFinish::Completed`!
     2. `record_completed_outcome` (`persistence.rs:1420, 1561-1636`) immediately transitions `child-1` in Root's coordinator from `ChildState::Running` to **`ChildState::Idle`**, records Layer-1's interim parking message as `child-1`'s final `ChildTaskOutcome::Completed`, emits **`RuntimeEvent::ChildCompleted { child: "child-1", ... }`** on Root's event stream, and wakes Root with an automatic child-completion turn (`"child child-1 completed: I have spawned child-1.1..."`) — **while `child-1.1` is still running and `child-1` hasn't even received `child-1.1`'s result yet!**
     3. Even worse: when `child-1.1` later finishes and wakes `child-1` via `layer1_coordinator.try_admit_child_completion_if_idle(...)` (which starts an **internal turn** `turn-2` on `child-1`), **nothing in Root's coordinator is watching `turn-2` on `child-1`**! Because `spawn_returned_input_collector` only awaited the single `turn: TurnHandle` from `handle.send(spec.task)` (`turn-1`) and then exited its `tokio::spawn` task (`monitor.rs:14-84`)! So `child-1`'s actual final synthesis in `turn-2` is **completely lost and never delivered to Root**!

### Exact Engineering Fix for Multi-Turn / Parked Layer-1 Children (`monitor.rs`, `coordinator.rs`, `persistence.rs`)

A delegated task on `child-1` is **not** a single `TurnHandle` when `child-1` has its own `DelegationCoordinator`; a delegated task on `child-1` is a **logical task episode** that spans:
- Initial `turn-1` (from `spawn` or `follow_up` or `resume`), **plus**
- Any subsequent internal continuation turns (`InternalTurnStarted` from `delegation.child-completion` or `smith.delegation.wait-timeout`) until `child-1` finishes a turn at an idle boundary where **`child-1` has no pending/running Layer-2 work and no undelivered ready Layer-2 outcomes**!

#### Step 1: Add `has_pending_delegated_work(&self) -> bool` on `SessionHandle` / `DelegationCoordinator`
In `/Volumes/Data/codes/ai/agent-runtime/crates/agent-runtime/src/delegation/coordinator.rs`:
```rust
impl DelegationCoordinator {
    /// Whether this coordinator still has running children, queued spawns,
    /// in-flight outcome admissions, or ready outcomes not yet consumed by its parent.
    pub fn has_pending_child_work(&self) -> bool {
        if self.inner.outcome_admission_in_flight.load(Ordering::Acquire) {
            return true;
        }
        let has_queued = !self.inner.queue.lock().expect("delegation queue poisoned").is_empty();
        if has_queued {
            return true;
        }
        let has_running = self.alive_children_running();
        if has_running {
            return true;
        }
        !self.take_ready_task_outcomes().is_empty()
    }
}
```
where `alive_children_running(&self)` checks:
```rust
self.inner
    .children
    .lock()
    .expect("delegation children poisoned")
    .values()
    .any(|entry| {
        matches!(entry.status.borrow().state, ChildState::Running)
            && matches!(entry.binding, ChildBinding::Live { .. })
    })
```
And on `SessionHandle` (`crates/agent-runtime/src/runtime/session/lifecycle.rs`):
```rust
pub fn has_pending_delegated_work(&self) -> bool {
    let coordinator = self
        .inner
        .child_coordinator
        .lock()
        .expect("child coordinator lock poisoned")
        .as_ref()
        .and_then(Weak::upgrade);
    if let Some(inner) = coordinator {
        if DelegationCoordinator { inner }.has_pending_child_work() {
            return true;
        }
    }
    // Also check if an internal continuation turn is already active or queued in turns
    let turns = self.inner.turns.lock().expect("session turns poisoned");
    turns.active.is_some()
}
```
*(Note: also include Smith's `AgentWaitTimers` if a Layer-1 child armed a `wait` timer with `timeout_ms > 0` while its Layer-2 child was running, though `alive_children_running()` is already `true` whenever a Layer-2 child is `Running`!)*

#### Step 2: Add a Session-Level Turn Outcome Channel / Notifier on `SessionInner` for Internal Continuation Turns
When `child-1` wakes from parking via `layer1_coordinator.try_admit_child_completion_if_idle` (`coordinator.rs:1006`), `child-1` starts a **new** `TurnHandle` (`turn-2`).
In `crates/agent-runtime/src/runtime/session/mod.rs` (`ActiveTurns` / `SessionInner`):
- Add `pub(crate) latest_turn_handle: Mutex<Option<TurnHandle>>` and `pub(crate) turn_started_notify: Notify` (or a `tokio::sync::watch::Receiver<Option<TurnHandle>>` updated whenever `reserve_turn` / `try_send_internal_if_idle_with_state_and_hook` accepts a new turn on `SessionHandle`).

#### Step 3: Upgrade `spawn_returned_input_collector` in `crates/agent-runtime/src/delegation/monitor.rs:7-85` into a Multi-Turn Task Episode Loop
Replace the single `let (finish, returned) = turn.outcome().await;` in `spawn_returned_input_collector` (`monitor.rs:15-52`) with a loop that follows `handle` across Layer-2 parking boundaries:
```rust
pub(super) fn spawn_returned_input_collector(
    &self,
    child: ChildId,
    handle: SessionHandle,
    initial_turn: TurnHandle,
) {
    let coordinator = self.inner.clone();
    tokio::spawn(async move {
        let mut current_turn = initial_turn;
        let mut turn_rx = handle.subscribe_turns(); // watch::Receiver<Option<TurnHandle>>
        let ( turn_id, finish, returned ) = loop {
            let turn_id = current_turn.id().clone();
            let (finish, returned) = current_turn.outcome().await;

            // 1. If the turn finished with NeedsInput, Cancelled, Failed, or None,
            //    it does NOT park for sub-agents — break immediately!
            let Some(TurnFinish::Completed | TurnFinish::LimitReached { .. }) = &finish else {
                break (turn_id, finish, returned);
            };
            if returned.is_some() {
                break (turn_id, finish, returned);
            }

            // 2. Normal turn completion on `child`. Check if `child` is a Layer-1
            //    parent that still has live Layer-2 children or ready Layer-2 outcomes!
            if !handle.has_pending_delegated_work() {
                break (turn_id, finish, returned);
            }

            // 3. `child` is parked awaiting its own Layer-2 sub-agents!
            //    Keep `child` in `ChildState::Running` in Root's coordinator.
            //    Wait for `child` to either start its next internal continuation turn
            //    (when Layer-2 completes or times out) OR for `child` to be cancelled/shut down
            //    OR for all of `child`'s Layer-2 children to fail/stop without producing
            //    a continuation turn.
            match handle.wait_next_turn_or_delegation_settled(&turn_id, &mut turn_rx).await {
                NextChildEpisodeStep::NextTurn(next_turn) => {
                    current_turn = next_turn;
                    continue;
                }
                NextChildEpisodeStep::SettledWithoutContinuation => {
                    // e.g., all Layer-2 children were stopped or failed without
                    // waking Layer-1, or Layer-1 was cancelled.
                    break (turn_id, finish, returned);
                }
            }
        };
        // ... existing emission of ChildProgress::TurnFinished and
        // record_returned_input / record_completed_outcome ...
    });
}
```
**Why this solves the Root/Layer-1/Layer-2 parking problem completely:**
- While `child-1` is parked waiting for `child-1.1`:
  - `handle.has_pending_delegated_work()` is `true` (because `child-1.1` is `ChildState::Running` or has a ready outcome awaiting `child-1`'s admission worker).
  - Root's `spawn_returned_input_collector` stays in the loop waiting for `child-1`'s continuation turn (`turn-2`).
  - Therefore, in Root's `DelegationCoordinator`, `child-1`'s `ChildStatus.state` **remains `ChildState::Running`**!
  - Because `child-1` remains `ChildState::Running` in Root's coordinator, Root's own `DelegationLifecycle` (`crates/smith-runtime/src/delegation/lifecycle.rs:160-166`) sees `child-1` as `Running` when Root's turn completes, so **Root automatically enters `ParentParkingState::ParkedAwaitingChild`**!
  - And in `wait_task_outcome` (`crates/agent-runtime/src/delegation/coordinator.rs:776-816`), `self.take_task_outcome("child-1")` remains `None` and `status.state` remains `Running` until `child-1.1` finishes -> wakes `child-1` (`turn-2`) -> `child-1` synthesizes the result and finishes `turn-2` with `has_pending_delegated_work() == false` -> Root's `spawn_returned_input_collector` exits the loop and calls `record_completed_outcome` with `turn-2`'s final answer -> wakes Root!

#### Edge Case in the Loop: What if Layer-2 (`child-1.1`) Fails or Is Stopped While Layer-1 Is Parked?
- Notice that in `agent-runtime` today, `ChildTaskOutcome` only has `Completed` and `NeedsInput` (`types.rs:728-743`). When a child fails (`TurnFinish::Failed`) or is stopped (`ChildStopped`), `spawn_monitor` emits `RuntimeEvent::ChildFailed` / `ChildStopped`, updates `ChildStatus.state = ChildState::Failed`, and **does not** insert into `ready_task_outcomes`!
- In a 1-layer setup, if `child-1` fails while Root is parked, Root's `DelegationLifecycle` (`crates/smith-runtime/src/delegation/lifecycle.rs:174-184`) calls `state.child_terminal(child)` which transitions Root's `DelegationParking` from `ParkedAwaitingChild` to `Idle` (`delegation_parking.rs:297`), and the human user in TUI sees the red error notice `"sub-agent child-1 failed: ..."`!
- **In a 2-layer setup**, there is no human sitting inside Layer-1 (`child-1`)! If `child-1` parked waiting for `child-1.1`, and `child-1.1` fails (`ChildFailed`), `child-1` **must be woken up** so `child-1` knows `child-1.1` failed and can either retry, spawn another sub-agent, or report the failure to Root!
- **Required Change in `agent-runtime` (`types.rs`, `persistence.rs`, `coordinator.rs`, `monitor.rs`)**:
  - Either add `ChildOutcomeIdentity::Failed(TurnId)` / `ChildTaskOutcome::Failed { child: ChildId, turn: TurnId, error: RuntimeError }` to the protected outcome ledger so `ChildFailed` wakes a parked parent via `try_admit_child_completion_if_idle` (`"- child child-1.1 failed: <error>"`), **OR** have `DelegationLifecycle` (in `smith-runtime/src/delegation/lifecycle.rs:174`) emit an internal turn when a child fails while the parent is `ParkedAwaitingChild`.
  - Adding `ChildTaskOutcome::Failed` (or waking on `ChildFailed` in `coordinator.rs`) is much cleaner because it guarantees a parked Layer-1 agent is never stranded in `ParkedAwaitingChild` if all its Layer-2 children fail!

---

# Section 2: `ask_user` and `needs_input` Across 2 Layers (`DirectHost` vs `ReturnToParent`)

In Design B (`User -> Root -> Layer-1 -> Layer-2`), there are two distinct interaction patterns to support:
1. **A designated "Clarifier" (or interactive Layer-1) agent** that is allowed to ask the user directly via the Root TUI questionnaire modal (`InteractionDisposition::DirectHost`).
2. **Standard Layer-1 and Layer-2 worker sub-agents** whose `ask_user` calls return `needs_input` up the chain (`InteractionDisposition::ReturnToParent`).

## 2.1 How `InteractionDisposition::DirectHost` (`allow_child_interaction`) Works for a Clarifier Agent
Let's check what `agent-runtime` already has:
- `RuntimeBuilder::allow_child_interaction(mut self, allow: bool)` exists at `/Volumes/Data/codes/ai/agent-runtime/crates/agent-runtime/src/runtime/builder.rs:738-741`!
- `Driver::new_session_execution_context` (`/Volumes/Data/codes/ai/agent-runtime/crates/agent-runtime/src/agent/driver/mod.rs:676-691`) already evaluates:
  ```rust
  let interaction_disposition = if parent.is_none() {
      InteractionDisposition::DirectHost
  } else if self.return_child_interactions_to_parent {
      InteractionDisposition::ReturnToParent
  } else if self.allow_child_interaction {
      InteractionDisposition::DirectHost
  } else {
      InteractionDisposition::Unavailable
  };
  ```
- **Why `allow_child_interaction` is currently overridden**:
  1. In `DelegationCoordinator::build_and_start` (`crates/agent-runtime/src/delegation/lifecycle.rs:1155`), it unconditionally calls:
     ```rust
     builder.return_child_interactions_to_parent();
     ```
     which sets `self.return_child_interactions_to_parent = true; self.allow_child_interaction = false;` (`builder.rs:745-748`).
  2. In `SmithChildFactory::child_builder` (`crates/smith-runtime/src/delegation.rs:568-625`), `SmithChildFactory` does **not** hold `interaction: Option<Arc<dyn InteractionBroker>>` and never calls `.interaction_broker(...)` on the child's `RuntimeBuilder`!

### Exact Changes to Enable `DirectHost` for a Clarifier Profile (or `allow_child_interaction`)
1. **In `agent-runtime` (`crates/agent-runtime/src/runtime/builder.rs:745-748` & `crates/agent-runtime/src/delegation/lifecycle.rs:1152-1156`)**:
   - In `RuntimeBuilder`: if the host's `ChildRuntimeFactory::child_builder` explicitly set `.allow_child_interaction(true)` (and supplied an `.interaction_broker(broker)`), `build_and_start` must **not** clobber it with `return_child_interactions_to_parent()`!
   - Specifically, change `builder.rs:745-748` to:
     ```rust
     pub(crate) fn return_child_interactions_to_parent_Unless_allowed(&mut self) {
         if !self.allow_child_interaction {
             self.return_child_interactions_to_parent = true;
         }
     }
     ```
2. **In `smith-config` & `smith-runtime`**:
   - If a profile (e.g., `[profiles.clarifier]` or `[profiles.<name>]`) declares `direct_interaction = true` (or if `SmithChildFactory` passes the root `interaction: Option<Arc<dyn InteractionBroker>>` when `allow_child_interaction` is enabled for a Layer-1 profile):
   - Add `pub(crate) interaction: Option<Arc<dyn InteractionBroker>>` to `SmithChildFactory` (`crates/smith-runtime/src/delegation.rs:185-199`, populated from `request.interaction.clone()` in `crates/smith-runtime/src/factory/construction.rs:699`).
   - In `SmithChildFactory::child_builder` (`crates/smith-runtime/src/delegation.rs:617-625`), if the route's profile opts into direct host interaction (and `child_depth == 1` and `self.interaction` is `Ready`):
     ```rust
     builder = builder
         .interaction_broker(interaction.clone())
         .allow_child_interaction(true);
     ```
   - **Concurrency guard in `InteractionSurface` (`crates/smith-cli/src/interaction.rs:45-76`)**:
     - `InteractionSurface` already keys active prompts by `request_id` in `prompts: BTreeMap<String, InteractionPrompt>`, and `InteractionOrigin` (`crates/agent-runtime-core/src/interaction.rs:42-81`) already carries `session: SessionId, turn: TurnId, call: ToolCallId`!
     - Because `InteractionRequestId` is fingerprinted over `(session_id, turn_id, call_id)` (`crates/agent-runtime/src/harness/questionnaire.rs:104-110`), a child's `InteractionRequest` has a globally unique `request_id` and flows through the exact same `InteractionNotice::Present(prompt)` channel to the TUI modal without any conflict!

## 2.2 How `ReturnToParent` (`needs_input`) Works Across 2 Layers (`Layer-2 -> Layer-1 -> Root`)
When `allow_child_interaction` is `false` (the default for all Layer-1 and Layer-2 sub-agents):

### Case A: Layer-2 Calls `ask_user` (`ReturnToParent` to Layer-1)
1. **Layer-2 finishes its turn with `TurnFinish::NeedsInput { request }`**:
   - Layer-2's `InteractionDisposition` is `ReturnToParent`.
   - Layer-2 commits the synthetic tool result `{"outcome": "needs_input", "request_id": ...}` (`crates/agent-runtime/src/agent/driver/tools.rs:405-411`) and finishes its turn without touching any UI broker.
2. **Layer-1's `DelegationCoordinator` collects the `NeedsInput` outcome**:
   - Layer-1's `spawn_returned_input_collector` (`monitor.rs:27-31`) records `ChildTaskOutcome::NeedsInput { child: "child-1.1", request }` into Layer-1's ledger and transitions `child-1.1` to `ChildState::Idle`.
   - Layer-1's `DelegationLifecycle` (`crates/smith-runtime/src/delegation/lifecycle.rs:236-263`) admits an internal child-completion turn on Layer-1 (`turn-2`).
   - Meanwhile, as designed in **Section 1.4**, Root's `spawn_returned_input_collector` sees that Layer-1 (`child-1`) had pending work and is now running `turn-2`, so Root stays parked waiting for `child-1`.
3. **Layer-1 (`turn-2`) Decides Whether It Can Answer Layer-2 Itself or Must Escalate to Root**:
   - **Subcase A1 — Layer-1 already knows the answer from the codebase or its task context**:
     - Layer-1 calls `agent` `action: "result", child_id: "child-1.1"` (if it needs the full `public_questionnaire` text), then calls `agent` `action: "follow_up", child_id: "child-1.1", task: "<answer>"`!
     - `child-1.1` resumes work in its `follow_up` turn, Layer-1 parks again, and **Root is never disturbed at all**!
   - **Subcase A2 — Layer-1 cannot answer without the human user (Escalation to Root)**:
     - Layer-1 calls its own `ask_user` tool with the question(s) it needs answered!
     - Because Layer-1's own `InteractionDisposition` is also `ReturnToParent` (unless Layer-1 is a `DirectHost` Clarifier):
       - Layer-1's `ask_user` call immediately returns `{"outcome": "needs_input", ...}` and finishes Layer-1's `turn-2` with `TurnFinish::NeedsInput { request: layer1_request }`.
       - In Root's `spawn_returned_input_collector` (Section 1.4 loop), `current_turn.outcome().await` returns `(Some(TurnFinish::NeedsInput { request: layer1_request }), Some(exact))`!
       - Because `finish` is `TurnFinish::NeedsInput`, the loop in Root's `spawn_returned_input_collector` **breaks immediately** (even if Layer-1 has idle children) and calls `record_returned_input` on Root's coordinator!
       - `child-1` transitions to `ChildState::Idle` (`NeedsInput`) in Root's coordinator, and Root wakes up with `ChildTaskOutcome::NeedsInput { child: "child-1", request: layer1_request }`.
       - Root invokes its root `ask_user` tool (opening the TUI modal), gets the user's answer, and sends `agent` `action: "follow_up", child_id: "child-1", task: "<user answer>"`.
       - Layer-1 wakes up in `turn-3` and forwards the answer via `agent` `action: "follow_up", child_id: "child-1.1", task: "<answer>"`!

### Prompt Update for Layer-1 Agents (`crates/smith-runtime/src/prompt.rs`)
- Currently, `QUESTIONNAIRE` (`crates/smith-runtime/src/prompt.rs:134-139`) and `DELEGATION` (`lines 141-160`) say:
  - *"If the parent chooses to ask the user, invoke root ask_user and then send the answer back to that child with an explicit agent follow_up."*
- And in `crates/smith-runtime/src/delegation/tool.rs:275-278`:
  ```rust
  "next_action": {
      "ask": "decide whether to invoke root ask_user",
      "return": "send the answer with agent follow_up"
  }
  ```
- **Update for Design B**:
  - In `crates/smith-runtime/src/factory/capabilities.rs:188-190`, `questionnaire_eligible(request)` is currently `!matches!(request.surface, HostSurface::Child)`, even though `SmithChildFactory::child_builder` (`delegation.rs:490`) *does* register `SmithQuestionnaireTool` (`ask_user`) on children!
  - For a Layer-1 agent that can delegate, include the `DELEGATION` prompt fragment and update the wording in `prompt.rs:145` and `tool.rs:276` from `"invoke root ask_user"` to `"answer from your own context if possible, or invoke ask_user to escalate the question, then send the answer with an explicit agent follow_up"`.

---

# Section 3: How `smith-config`, `smith-runtime`, `smith-client`, `smith-tui`, and `smith-cli` Wire and Project 2 Layers

## 3.1 `smith-config`: Profile Configuration for Layer-1 vs Layer-2
Let's check `crates/smith-config/src/model.rs:425-430` and `crates/smith-config/src/resolve/agent.rs:363-365`:
- Every profile in `config.toml` **already** has `delegation: Option<bool>` (defaulting to `true` in `resolve/agent.rs:364`) and `use = ["main", "child"]`!
- Today, the doc comment on `ProfileSection::delegation` (`model.rs:427-428`) says: *"Child runtimes never delegate regardless of this value."*
- Under Design B:
  - A profile with `use = ["main", "child"]` and `delegation = true` (e.g., `orchestrator` or a Layer-1 lead profile) can be spawned as a **Layer-1 child** and **will receive the `agent` tool** so it can spawn Layer-2 sub-agents!
  - A profile with `delegation = false` (or any agent spawned at `delegation_depth == 2`) **will not receive the `agent` tool** (acting as a leaf worker).
  - Or if you want explicit placement control without changing existing child profiles that defaulted `delegation` to `true`:
    - Either check `route.delegation && child_depth < max_depth` in `SmithChildFactory::child_builder` (so any child profile with `delegation = true` can spawn Layer-2 children when running at `depth == 1`),
    - Or set `delegation = false` on leaf profiles (`code`, `sol`, `visual`, `research`, `security`) and `delegation = true` on Layer-1 lead profiles.

## 3.2 `smith-runtime`: Wiring `AgentTool` and `DelegationLifecycle` Inside Layer-1 Children
Currently, `SmithChildFactory` (`crates/smith-runtime/src/delegation.rs:185-199, 375-670`) builds a child `RuntimeBuilder` in `child_builder(&self, spec: &ChildSpec)`, and `DelegationCoordinator::build_and_start` (`crates/agent-runtime/src/delegation/lifecycle.rs:1110-1186`) calls `builder.build()` and `runtime.start_child_session(...)`.

Look at what happens when a Layer-1 child is built and started:
1. **In `SmithChildRoute` (`crates/smith-runtime/src/delegation.rs:203-235`)**:
   - Add `pub(crate) delegation: bool` (from `agent_profile.delegation.value` in `crates/smith-runtime/src/factory/provider/children.rs:354` and `factory/construction.rs:667`) and `pub(crate) wait_policy: DelegationWaitPolicy`.
2. **In `ChildRuntimeFactory` trait (`crates/agent-runtime/src/delegation/types.rs:3-51`)**:
   - Add two hooks (with default no-op implementations so existing tests/implementors don't break):
     ```rust
     pub trait ChildRuntimeFactory: Send + Sync + fmt::Debug {
         fn child_builder(&self, spec: &ChildSpec) -> Result<RuntimeBuilder, RuntimeError> { ... }

         /// Depth-aware builder hook. Defaults to calling `child_builder(spec)`.
         fn child_builder_at_depth(
             &self,
             spec: &ChildSpec,
             depth: u8,
             max_depth: u8,
         ) -> Result<(RuntimeBuilder, Option<Arc<OnceLock<DelegationCoordinator>>>), RuntimeError> {
             Ok((self.child_builder(spec)?, None))
         }

         /// Called by `DelegationCoordinator::build_and_start` immediately after
         /// `start_child_session` succeeds and before the first turn is sent.
         /// Allows the host to wire a nested `DelegationCoordinator` + `DelegationLifecycle`
         /// onto a Layer-1 child session.
         async fn on_child_session_started(
             &self,
             _parent: &SessionHandle,
             _child: &ChildId,
             _child_session: &SessionHandle,
             _depth: u8,
             _slot: Option<Arc<OnceLock<DelegationCoordinator>>>,
         ) -> Result<(), RuntimeError> {
             Ok(())
         }
     ```
3. **In `SmithChildFactory::child_builder_at_depth` (`crates/smith-runtime/src/delegation.rs:375-670`)**:
   - When `depth < max_depth && route.delegation`:
     1. Create `let slot = Arc::new(OnceLock::new());`
     2. Create `let wait_timers = Arc::new(AgentWaitTimers::new(route.agent_profile_name.clone()));`
     3. Build `AgentTool::new(slot.clone()).with_profiles(self.agent_tool_profiles.clone()).with_wait_policy(route.wait_policy).with_wait_timers(wait_timers.clone()).with_transcript_output(...)` and push it to `tools` (`delegation.rs:490`)!
     4. Register `DelegationAuthority::new()` on `builder.security_check(...)` (`crates/smith-runtime/src/factory/construction.rs:605-614`) so `agent.delegate` permission is covered in the Layer-1 child's `SecurityCheckSet`!
     5. Include the `DELEGATION` prompt fragment in `route.prompt_contributor` when `depth < max_depth && route.delegation`.
4. **In `SmithChildFactory::on_child_session_started` (`crates/smith-runtime/src/delegation.rs`)**:
   - When `slot` is `Some(slot)` (i.e. Layer-1 child with delegation enabled):
     1. Construct a `SmithDelegation { factory: self.clone_for_child(), slot, wait_timers }`.
     2. Call `wire_delegation_with_wait_policy(child_session, &child_delegation, route.wait_policy).await?`!
     3. Attach the resulting `DelegationLifecycle` to `child_session` (or store it in a map on `SmithChildFactory` keyed by `child_session.id()`, and shut it down when `child_session` emits `SessionShutdown`).
   - Look how clean this is: **Layer-1 gets the exact same `DelegationCoordinator` and `DelegationLifecycle` (`event_task` + `admission_task`) that Root uses!** When a Layer-2 grandchild (`child-1.1`) completes or returns `needs_input`, Layer-1's own `DelegationLifecycle` automatically wakes Layer-1 at its idle boundary via `try_admit_child_completion_if_idle`!

---

## 3.3 Event Propagation & Projection Across `agent-runtime`, `smith-runtime`, `smith-client`, `smith-tui`, and `smith-cli`

How does the Root session (and Root TUI / Headless CLI) see and inspect Layer-2 grandchildren (`child-1.1`)?

### 1. Forwarding Layer-2 Lifecycle Events to the Root Event Stream (`agent-runtime/src/delegation/monitor.rs`)
Look at `DelegationCoordinator::spawn_monitor` in `/Volumes/Data/codes/ai/agent-runtime/crates/agent-runtime/src/delegation/monitor.rs:116-244`:
- Currently, `spawn_monitor` subscribes to `child`'s `RuntimeEventStream` and matches on `TurnStarted`, `Error`, `Usage`, `TurnCompleted`, and `SessionShutdown`, with `_ => {}` at `line 242`.
- When Layer-1 (`child-1`) spawns or manages a Layer-2 child (`child-1.1`), Layer-1's coordinator emits on **Layer-1's `emitter`**:
  - `RuntimeEvent::ChildSpawned { child: "child-1.1", ... }`
  - `RuntimeEvent::ChildProgress { child: "child-1.1", phase: ... }`
  - `RuntimeEvent::ChildNeedsInput { child: "child-1.1", ... }`
  - `RuntimeEvent::ChildCompleted { child: "child-1.1", ... }`
  - `RuntimeEvent::ChildStopped { child: "child-1.1", ... }`
  - `RuntimeEvent::ChildFailed { child: "child-1.1", ... }`
- **In `spawn_monitor` (`monitor.rs:242`)**:
  - Add a match arm for those six `RuntimeEvent::Child*` variants!
  - When Root's `spawn_monitor` (which is watching Layer-1 `child-1`'s event stream) observes a `RuntimeEvent::Child*` event emitted by Layer-1 for `child-1.1`:
    1. It re-emits that `RuntimeEvent::Child*` event onto `parent_emitter` (Root's event stream)!
    2. And when `child-1.1` emits `RuntimeEvent::Usage` into `child-1`, `child-1`'s `spawn_monitor` updates `child-1.1`'s `tokens_used` AND rolls `child-1.1`'s token delta up into `child-1`'s `tokens_used` (or emits a delegated usage record) so parent token budgets (`max_tokens`) apply across the subtree!

### 2. Coordinator Tree Lookup for Status, Counts, and Transcript Inspection (`agent-runtime` & `smith-runtime`)
In `DelegationCoordinator` (`/Volumes/Data/codes/ai/agent-runtime/crates/agent-runtime/src/delegation/coordinator.rs` & `history.rs`):
- Because `SessionInner` now holds `child_coordinator: Mutex<Option<Weak<CoordinatorInner>>>` (from Section 1.1):
  - Given a live `ChildEntry` for `child-1` (`ChildBinding::Live { handle, .. }`), Root's `DelegationCoordinator` can access `child-1`'s `DelegationCoordinator` via `handle.child_coordinator()`!
  - Update the following read/inspection methods on `DelegationCoordinator` to traverse into child coordinators when `child` is not a direct key (or when listing all descendants):
    1. **`list_all(&self) -> Vec<ChildStatus>`** (or `list(&self)`):
       - For each direct `entry` in `self.inner.children`: push `entry.status.borrow().clone()`, and if `entry` has a live `handle` with a `child_coordinator`, extend with `child_coord.list()`!
       - Because `ChildId`s are `"child-1"`, `"child-1.1"`, `"child-1.2"`, `"child-2"`, lexical order automatically groups each Layer-1 agent followed immediately by its Layer-2 sub-agents!
    2. **`status(&self, child: &ChildId)`** (`coordinator.rs:440-450`):
       - First check `self.inner.children.get(child)`. If not found, check any child coordinator (`for entry in children.values() { if let Some(sub) = entry.child_coordinator() { if let Ok(s) = sub.status(child) { return Ok(s); } } }`).
    3. **`child_history(&self, child: &ChildId)` and `subscribe_child_history(&self, child: &ChildId)`** (`crates/agent-runtime/src/delegation/history.rs:58-90`):
       - If `self.inner.children` does not contain `child` directly, delegate `child_history(child)` / `subscribe_child_history(child)` to the owning Layer-1 child's coordinator!
       - **Impact on `smith-runtime/src/host/child_history.rs:34-75`**: `HostSession::child_transcript("child-1.1")` and `HostSession::subscribe_child_transcript("child-1.1")` will **work out of the box with zero changes** because they call `coordinator.child_history(child)` and `coordinator.subscribe_child_history(child)`!
       - **Impact on `smith-cli/src/tui_driver/events.rs:256-315`**: When `RuntimeEvent::ChildSpawned { child: "child-1.1", .. }` arrives on Root's event stream, `TuiLoop::ensure_child_subscription("child-1.1")` calls `host.subscribe_child_transcript("child-1.1")`, which subscribes directly to `child-1.1`'s live `SmithEventStream` and streams `child-1.1`'s text deltas, tool calls, and usage into `App.child_conversations["child-1.1"]`!

### 3. Headless Exit Condition (`crates/smith-cli/src/headless/run_flow.rs:478-498`)
- In `has_required_child_work(host: &HostSession, observed_delivery: u64)` (`crates/smith-cli/src/headless/run_flow.rs:478-498`):
  - It checks:
    ```rust
    coordinator
        .list()
        .into_iter()
        .any(|status| matches!(status.state, ChildState::Running))
        || !coordinator.take_ready_task_outcomes().is_empty()
        || coordinator.child_outcome_cursor().revision() > observed_delivery
    ```
  - Because our fix in **Section 1.4** keeps Layer-1 (`child-1`) in `ChildState::Running` in Root's coordinator for the entire time `child-1` is parked waiting for Layer-2 (`child-1.1`), `coordinator.list().any(|status| status.state == ChildState::Running)` **automatically stays `true`** in headless mode until Layer-2 finishes, wakes Layer-1, Layer-1 finishes its synthesis turn, and Root finishes its delivery turn!

### 4. `smith-client` & `smith-tui`: Rendering 2 Layers in the Delegated-Work Panel & `/agent` Report
Let's check every place in `smith-client` and `smith-tui` that touches `ChildId` or `ChildSummary`:
1. **`crates/smith-tui/src/app/reducer.rs:804-1045`**:
   - Handles `ChildSpawned`, `ChildProgress`, `ChildNeedsInput`, `ChildCompleted`, `ChildStopped`, `ChildFailed`.
   - Keys `self.children: BTreeMap<String, ChildSummary>`, `self.child_conversations`, `self.child_clocks`, `self.child_counts` by `child.to_string()`.
   - With hierarchical `ChildId`s (`"child-1"`, `"child-1.1"`, `"child-1.2"`):
     - In `RuntimeEvent::ChildSpawned` (`reducer.rs:804-837`), `self.pending_agent_spawns.pop_front()` only correlates root `agent` tool calls. Guard `self.pending_agent_spawns.pop_front()` with `if !child.as_str().contains('.')` so a Layer-2 spawn (`"child-1.1"`) does not steal a pending root spawn correlation entry from `"child-2"`!
     - In `RuntimeEvent::TurnCompleted` (`reducer.rs:1083`), `let pending_child = self.children.values().any(|child| child.state.is_running());` sets `Activity::ParkedAwaitingChild`. This works automatically for 2 layers.
2. **`crates/smith-tui/src/render/composer.rs:848-1021` (`draw_agents` & `agent_row`)**:
   - Currently renders each visible child row as `"  ○ child-1  <profile> · <activity>    0:12"`.
   - For a Layer-2 child (`child_id.contains('.')`, e.g. `"child-1.1"`), indenting the row by 2 extra spaces in `panel_row` (`composer.rs:1008-1010`):
     ```rust
     let indent = if identity.contains('.') { "    " } else { "  " };
     let left = clip_line(
         format!("{indent}{marker} {identity}  {activity}"),
         width.saturating_sub(reserved),
     );
     ```
     visually nests Layer-2 sub-agents under their Layer-1 parent in the bottom panel (`main` -> `child-1` -> `child-1.1`) while preserving keyboard navigation (`inspect_next_child` / `inspect_previous_child` in `crates/smith-tui/src/app/state/children.rs:401-481`).
3. **`crates/smith-client/src/agent_report.rs:58-120` & `crates/smith-cli/src/local_command/agent.rs:22-69`**:
   - `/agent` (`AgentAction::List`, `AgentAction::Inspect("child-1.1")`, `AgentAction::Next`, `AgentAction::Previous`) reads `coordinator.list()`. Once `coordinator.list()` includes active Layer-2 children (with their `parent` session ID already on `ChildStatus.parent`), `/agent` and `/agent child-1.1` work seamlessly.

---

# Summary Checklist of Files & Line Numbers to Modify for Design B

| Crate | File & Lines | Change Summary |
|---|---|---|
| `agent-runtime` | `crates/agent-runtime/src/runtime/session/mod.rs:56-100` | Add `root_session: Option<SessionId>`, `delegation_depth: u8`, `child_id: Option<ChildId>`, `child_coordinator: Mutex<Option<Weak<CoordinatorInner>>>`, and turn-watch channel to `SessionInner`. |
| `agent-runtime` | `crates/agent-runtime/src/runtime/session/lifecycle.rs:49-55` | Expose `delegation_depth()`, `root_session()`, `child_id()`, `has_pending_delegated_work()` on `SessionHandle`. |
| `agent-runtime` | `crates/agent-runtime/src/runtime/engine.rs:1379-1393` | Pass lineage (`parent`, `root_session`, `delegation_depth`, `child_id`) through `start_child_session`. |
| `agent-runtime` | `crates/agent-runtime/src/delegation/types.rs:3-63, 406-485` | Add `max_depth: u8` to `DelegationLimits`; add `child_builder_at_depth` and `on_child_session_started` hooks to `ChildRuntimeFactory`. |
| `agent-runtime` | `crates/agent-runtime/src/delegation/coordinator.rs:8-15, 333, 430-450, 836` | Check `parent.delegation_depth() >= config.limits.max_depth` instead of `parent.parent().is_some()`; register `Weak<CoordinatorInner>` on `parent`; traverse child coordinators in `list()` / `status()`. |
| `agent-runtime` | `crates/agent-runtime/src/delegation/lifecycle.rs:777-803, 1106-1186` | Prefix `mint_child_id()` with parent `child_id` (`child-1.1`); strip `delegation_tool_names` only when `child_depth >= max_depth`; invoke `child_builder_at_depth` & `on_child_session_started`; respect `allow_child_interaction`. |
| `agent-runtime` | `crates/agent-runtime/src/delegation/monitor.rs:7-85, 116-244` | **Critical**: Upgrade `spawn_returned_input_collector` to loop across Layer-1 parking boundaries while `handle.has_pending_delegated_work()` is true; forward Layer-2 `RuntimeEvent::Child*` events to Root emitter in `spawn_monitor`. |
| `agent-runtime` | `crates/agent-runtime/src/delegation/history.rs:58-90` | Delegate `child_history` and `subscribe_child_history` to child coordinators for `"child-1.1"`. |
| `smith-runtime` | `crates/smith-runtime/src/delegation.rs:185-235, 375-745` | Implement `child_builder_at_depth` and `on_child_session_started` on `SmithChildFactory` to register `AgentTool`, `DelegationAuthority`, and `wire_delegation_with_wait_policy` for Layer-1 (`depth == 1`) profiles with `delegation = true`; set `max_depth: 2`. |
| `smith-runtime` | `crates/smith-runtime/src/factory/provider/children.rs:315-396` | Carry `delegation: agent_profile.delegation.value` and `wait_policy` onto `SmithChildRoute`; enable `DELEGATION` prompt fragment for Layer-1 delegating routes. |
| `smith-tui` | `crates/smith-tui/src/app/reducer.rs:804-837` & `render/composer.rs:1008-1011` | Guard root `pending_agent_spawns.pop_front()` with `!child.as_str().contains('.')`; indent `child-1.1` rows in `panel_row`. |
````
