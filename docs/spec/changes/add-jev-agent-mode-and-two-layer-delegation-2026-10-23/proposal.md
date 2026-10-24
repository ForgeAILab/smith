---
created_at: 2026-10-23T21:44:00Z
updated_at: 2026-10-23T21:44:00Z
---

# Jev Agent Mode and bounded two-layer delegation

Status: Stage 1 proposal; implementation and rollout require approval. No runtime,
configuration schema, permission, or default behavior is changed by these documents.

## Why

Design B separates user-facing orchestration from implementation:

```text
User -> Jev / Root (depth 0)
          -> Layer-1 Clarifier or Lead (depth 1; sol, code, etc.)
               -> Layer-2 worker sub-agents (depth 2; research, code, etc.)
```

The root remains the durable user-session and host-authority anchor, not another
large implementation conversation. A short SystemOne `/v1/systemone` decision pass
can suggest clarification, implementation scope, an appropriate lead profile, and
whether decomposition may help. A clarifier obtains missing task information
before expensive implementation starts. A Layer-1 lead can keep detailed tool work
and Layer-2 synthesis in its own conversation, returning a final report to Root.
A clean root does not mean erasing canonical user messages, exact interaction
provenance, approvals, or inspectable child history.

The live local experiment recorded **448 requests: 192 four-question primary
requests, 192 clarification-only follow-up requests, and 64 diagnostics**. This is
24 distinct prompts (16 controlled and eight genuine local-session messages), not
448 independent tasks; protocol validations and the initial smoke are additional.
The primary/follow-up requests scored 960 decisions. Results justify testing this
architecture, not treating small-model predictions as established ground truth:

- On `2b-v3`, inverse actionability (`P(clarify) = 1 - P(is_actionable)`) reached
  **22/24 accuracy and 8/8 must-clarify recall**, with two false positives. It was
  selected in-sample after poor direct-question recall; it is not held-out proof.
- With descriptive roles, profile routing reached **18/24**. The **10/24 bare-name
  comparison is specifically the 0.8B A/B conditions**, not a universal 2B result;
  2B A was 13/24. Role descriptions must be explicit and profile-specific.
- Detailed-rubric complexity was within **±1 on 23/24** cases for both releases;
  exact accuracy was 16/24 (0.8B) and 17/24 (2B).
- `0.8b-v3` scored **24/24 on sub-agent need** with the detailed rubric, but there
  were only four positive cases. 2B D scored 21/24 and missed three positives.
- Direct `needs_clarification` wording with the detailed four-question rubric
  achieved 17/24 accuracy for both models while detecting only **1/8** ambiguous
  positives. A majority-class baseline already achieves 16/24. Calibration and
  phrasing sensitivity rule out an unchecked autonomous dispatch gate.
- Four-question p50/p95 wall latency was 437.8/828.2 ms (0.8B D) and
  494.9/515.6 ms (2B D). The stronger 2B inverse question alone averaged 99.9 ms;
  that is not a measured latency for a new combined four-head router.

`evidence.md` permanently embeds the complete captured report, calibration,
confusion matrices, per-case results, and validation caveats. Locally tested
serving was `jev-style` 0.5.0 with cached MLX BF16 `0.8b-v3` and `2b-v3`; Ollama's
store was empty and no usable `jev-score`/decision GGUF installation was found.
An official `jev` or other SystemOne service is eligible only through a verified
compatible contract and explicit user-owned disclosure settings; stock chat
completions and an arbitrary model name are not interchangeable with this API.

Jev is therefore **advisory triage with fail-open fallback to ordinary Smith
handling**, never fail-open authorization. Unavailability, malformed probabilities,
low routing confidence, or a bad route must not lose or block a user turn.

## What Changes

**Compatibility-sensitive behavior change:** Smith's former root-only delegation
ceiling becomes a bounded two-layer policy. Explicit child resume must retain
parent lineage rather than promote a child to Root. Neutral runtime defaults
remain depth one; protected recovery and consumer APIs require coordinated
versioning and migration.

### 2-Layer Bounded Delegation Hierarchy (`max_depth = 2`)

In `agent-runtime` and `smith-runtime`:

- Add protected session lineage `delegation_depth: u8`,
  `root_session: Option<SessionId>`, and `child_id: Option<ChildId>` to
  `SessionInner`, exposed by `SessionHandle`; immediate `parent` remains intact.
  Depths are Root=0, Layer-1=1, Layer-2=2. Legacy neutral-runtime consumers retain
  `DelegationLimits::max_depth = 1`; Smith explicitly composes a maximum of 2.
- Replace `if parent.parent().is_some()` with
  `if parent.delegation_depth() >= config.limits.max_depth` at every host/runtime
  delegation boundary. A missing tool is not the only depth/security check.
- Mint `child-1`, `child-2`, then `child-1.1`, `child-1.2`; keep counters and
  ownership parent-scoped and persistent. Share one process-wide
  `Arc<DelegationCapacity>` across Root and Layer-1 coordinators, preserving
  per-parent caps, budgets, deadlines, and bounded shutdown.
- Preserve the `agent` tool only for an eligible Layer-1 route with effective
  `delegation = true`. Strip `delegation_tool_names` at
  `child_depth >= max_depth` and on `delegation = false`; Layer 2 is always a leaf.
- Extend `ChildRuntimeFactory` with compatible depth-aware builder and post-start
  hooks. `SmithChildFactory::child_builder_at_depth` prepares the tool and slot;
  `on_child_session_started` wires and recovers Layer-1 delegation before its first
  task/follow-up/resume turn. Nested lifecycle ownership must not create Arc cycles.

### Multi-turn Task Episode Collector

Replace the single-turn `spawn_returned_input_collector` completion assumption
with a lossless logical-task episode loop. While
`handle.has_pending_delegated_work()` is true, Root keeps `child-1` in
`ChildState::Running` across Layer-1's parked `turn-1` and subsequent
`delegation.child-completion` / `smith.delegation.wait-timeout` continuation turns.
Only a final completed turn with no pending Layer-2 work produces the parent's
`ChildTaskOutcome::Completed` and the existing idle lifecycle state. The runtime
does not currently have a `ChildState::Completed` enum variant.

The collector exits appropriately on `NeedsInput`, failure, cancellation, or
shutdown; budget exhaustion does not extend limits. `ChildFailed` and `ChildStopped`
must produce protected, exactly-once parent wake evidence, including when the only
parked parent is an unattended Layer-1 agent. An old parking message must never be
used as a successful final answer merely because descendants failed.

### Direct Host Interaction and Escalation

Designated Layer-1 clarifier/lead profiles may resolve `direct_interaction = true`.
The host supplies the root `InteractionBroker` and calls
`allow_child_interaction(true)` without the coordinator subsequently clobbering it.
DirectHost remains subject to host policy, placement, available interaction
surface, and ordinary tool authorization. Layer 2 never receives DirectHost.

Standard workers keep `ReturnToParent`: Layer-2 `NeedsInput` wakes Layer-1;
Layer-1 answers from its context or escalates through its own `ask_user` to Root.
Task answers are not approval. Direct questions are attributed by originating
session/turn/call/request and serialized through one root modal. Headless or
unavailable brokers use the existing typed input-required/escalation path.

### Event Forwarding, Inspection, and TUI Projection

Forward canonical Layer-2 `RuntimeEvent::Child*` lifecycle events through the
Layer-1 monitor to Root without inserting grandchild result text into Root's model
context or consuming Layer-1's outcome ledger. Traverse authorized descendants in
`list()`, `status()`, `child_history()`, `subscribe_child_history()`, and companion
read APIs, including retained dormant descendants without provider execution.
Usage attribution must not double-charge forwarded events. Root lists and inspects
its subtree; Layer-1 cannot inspect siblings or unrelated roots.

Indent `child-1.1` below `child-1` in the delegated-work panel, order numeric ID
segments as a tree, and preserve existing conversation inspection/navigation.
Guard root pending-spawn correlation against Layer-2 events (the blueprint calls
this `pending_agent_spawns`; current reducer storage is `pending_spawns`). Headless
completion follows the whole episode through Layer-2 integration and final Root
delivery, not the first completed Layer-1 turn.

### Jev `/v1/systemone` Advisory Triage Router

Add optional `[jev]` settings and a shared pre-turn `JevTriageClient` adapter:

- `enabled` (default false), loopback `endpoint`, bounded `timeout_ms`, finite
  `actionability_threshold` and `profile_confidence_threshold`, an optional bearer
  credential reference, and a named child-enabled clarifier profile.
- For eligible real-user root submissions only, request inverse-actionability
  `noul`, complexity `score`, sub-agent-need `noul`, and profile `choice` with
  explicit one-line descriptions from the effective preflighted profile roster.
  Do not triage internal continuations, child turns, slash/shell commands, or
  questionnaire replies recursively.
- Compute `P(clarify) = 1 - P(is_actionable)`. Complexity uses `score + 1` for the
  expected 1..5 value and `argmax(probabilities) + 1` for the exact class.
  Profile confidence is `(k * p_max - 1) / (k - 1)` clipped to [0,1], not raw pmax.
- A bounded host admission path records the real user input once and dispatches
  at most one authorized Layer-1 episode without an implementation provider turn
  on Root. Clarifier answers can be handed to a new Layer-1 lead without promoting
  implementation into Layer 3. The fallback is the existing root path.
- Timeout, service errors, invalid response/model identity, unavailable profile,
  or low confidence produce a bounded diagnostic and ordinary handling, with no
  duplicate prompt, no dropped attachments, and no authority expansion. Remote
  prompt disclosure requires user-owned endpoint authorization; redirects are off.

## Configuration

The following fields are proposed, not supported configuration in this Stage-1
change. Default timeout/threshold values are engineering starting points, not
calibration guarantees. Existing provider/model declarations are assumed.

```toml
[jev]
enabled = true
endpoint = "http://127.0.0.1:8765/v1/systemone"
timeout_ms = 1500
actionability_threshold = 0.5
profile_confidence_threshold = 0.8
clarifier_profile = "clarifier"

[profiles.sol]
description = "Substantial implementation, debugging, refactoring, and architecture"
use = ["main", "child"]
delegation = true
direct_interaction = true

[profiles.code]
description = "Small well-specified edits, focused tests, and explicit verification commands"
use = ["main", "child"]
delegation = true
direct_interaction = true

[profiles.research]
description = "Read-only investigation of code, documentation, and options"
use = ["main", "child"]
posture = "plan"
delegation = false
direct_interaction = false

[profiles.clarifier]
extends = "research"
description = "Obtain essential missing task information, then return a concise handoff"
use = ["child"]
delegation = false
direct_interaction = true
```

For an endpoint requiring a token, add `bearer_credential_ref = "jev-local"` under
`[jev]`; resolve it through the existing user-owned credential store, not plaintext
TOML. The declaration does not itself authorize an endpoint or a tool.
`direct_interaction` defaults false. Omitted profile `delegation` retains the
existing inherited true setting, but depth and host policy remain upper bounds;
configure leaf profiles explicitly when adopting two-layer behavior.

`timeout_ms` defaults 1500 and is valid in 1..=10,000. Both thresholds must be
finite numbers in [0,1]. `actionability_threshold` compares P(actionable): a value
at or below it suggests clarification. It is not a probability-of-correctness
threshold. The
profile threshold compares normalized choice confidence; the benchmark's pmax
selectivity table is not evidence for the same numeric API-confidence threshold.
Disabling `[jev]` restores ordinary root admission with no sidecar requests.

## Scope

In scope: a bounded two-layer hierarchy, its episode and failure-wake correctness,
profile-aware DirectHost interaction, authorized descendant observability, optional
advisory triage, shared-surface admission, and deterministic validation.

Out of scope: implementing this proposal during Stage 1; unbounded recursion;
training/calibrating Jev; starting/downloading/managing sidecars automatically;
stock Ollama chat-to-probability emulation; automatic worktree merges; independent
remote credentials or network authority; replacing host approval/security policy;
or claiming that the combined router has been validated by independent tasks.

## Impact

- **Affected specs:** `child-agents`, `configuration`, `runtime-integration`,
  `client-surfaces`; their deltas accompany this proposal. The old hierarchy
  requirement is explicitly renamed rather than leaving a contradictory depth-1
  requirement behind.
- **Agent Runtime:** session lineage/turn notification and recovery; factory hooks;
  delegation types, depth/capacity checks, ID minting, episode collection, protected
  terminal ledgers, monitor forwarding, history traversal, and shared testkit
  conformance. Canonical event/checkpoint changes belong upstream, not in a Smith
  shadow runtime.
- **Smith:** `smith-config` model/resolution/provenance; `smith-runtime`
  `delegation.rs`, `delegation/lifecycle.rs`, factory construction/provider routes,
  host admission, session listing, prompts, and proposed `jev.rs` adapter;
  `smith-client` report projection; TUI reducer/composer; CLI interactive/headless
  submission, questionnaire routing, inspection, and completion waiting.
- **Compatibility:** the neutral runtime's default remains depth 1; Smith composes
  depth 2, still narrowed by route/profile/host policy. Version protected lineage
  and failure outcome records, keep existing states and namespace ownership, and
  fail closed on incompatible or authority-widening recovery. Existing root-only
  behavior remains available with delegation disabled on child profiles.
- **Dependencies:** rebase on `update-agent-wait-to-monitor-2026-10-23`, preserving
  its immediate-return wait/timer semantics. Retain profile-model-fallback,
  harness, durable transcript, and permission-narrowing work already in this dirty
  checkout. Implement/release upstream APIs before atomically updating Smith's
  pinned `agent-runtime*` revisions and lockfile. See the inspected baseline and
  source map in `design.md`; this proposal does not modify either codebase.
