## Context

Smith already composes standard harness components through Agent Runtime. The
todo component demonstrates the intended ownership split: versioned component
state, context contribution, tool-output processing, turn-commit mutation, and
typed events live in the shared runtime; Smith selects public/sensitive policy
and projects events into terminal and headless surfaces.

A persistent goal differs from a todo plan. A todo is scoped to one turn and
reconciles unfinished items at that turn's terminal boundary. A goal survives
turn boundaries and may intentionally start a later turn without a new user
message. The current `SessionHandle::send` always queues ordinary `UserInput`,
while injected content is introduced as user-role content inside an existing
turn. Neither contract can safely represent an automatic goal continuation.

## Goals / Non-Goals

### Goals

- Provide one explicit, persistent root-session objective that continues until
  it completes or reaches a stopped state.
- Keep goal state, context, events, accounting, and continuation reusable in
  Agent Runtime rather than duplicating them in Smith's TUI or host loop.
- Preserve canonical conversation history: automatic turns are typed internal
  work, never fabricated user messages.
- Make real user input, interruption, global limits, approval, and shutdown
  authoritative over automatic continuation.
- Report token and elapsed usage with honest provenance and fail closed when an
  explicit token budget cannot be observed reliably.
- Keep interactive and headless goal semantics equivalent.

### Non-Goals

- Goal attachment files, image handling, oversized-paste materialization,
  analytics, metrics, thread forks, fork deferrals, app-server JSON-RPC, or SDKs.
- Multiple goals in one session, goal trees, child goals, review goals, or
  sharing one goal across sessions.
- A daemon, remote scheduler, monitor executor, work after process shutdown, or
  automatic restart of an interrupted process.
- Replacing per-turn todos. A goal may use ordinary todos inside each turn, but
  the two state machines and UI projections remain distinct.

## Decisions

### One versioned goal envelope per persistent root session

The standard goal state contains a stable `goal_id`, bounded objective, status,
optional positive token budget, token usage with provenance, active elapsed
seconds, created/updated timestamps, and a bounded stopped reason when
applicable. The lifecycle statuses are:

- `active`: eligible for automatic continuation;
- `paused`: stopped by explicit user control or interactive interruption;
- `blocked`: stopped because the model declared a genuine blocker, a required
  interaction is unavailable, accounting required for a budget is unavailable,
  or a non-limit terminal error made continuation unsafe;
- `usage_limited`: stopped by an external provider/account usage limit;
- `budget_limited`: stopped after observed goal usage reaches or exceeds the
  requested token budget;
- `complete`: the objective is achieved.

Only `active` schedules work. Paused, blocked, and usage-limited goals may be
resumed by explicit user control after the condition changes. A budget-limited
goal may resume only after the user raises or removes the budget. Complete goals
remain visible and may be replaced by a new explicitly requested goal. User
clear removes the current projection without fabricating completion.

The goal envelope uses one stable component namespace and schema revision in
the existing `SessionSnapshot::extension_state`. The objective is bounded
public session working state, like Smith's public todo projection; it may be
displayed and journaled through typed goal events. It never grants authority
and is not written into the project checkout.

Alternative considered: a Smith-owned SQLite table or sidecar file. Rejected
because it would create a second session identity, atomicity, migration,
resume, and deletion contract beside the canonical runtime snapshot.

### Model tools are deliberately narrower than user controls

The reusable goal ability exposes three model tools:

- `get_goal`: read the current goal and remaining budget evidence;
- `create_goal`: create when no goal exists or replace a complete goal;
- `update_goal`: mark the current goal only `complete` or `blocked`.

Tool descriptions require explicit user or higher-priority instruction before
creation, prohibit inferred token budgets, and define the repeated-blocker
threshold. Runtime validation rejects an empty/oversized objective, a
non-positive budget, creation over an unfinished goal, invalid transitions, and
stale goal identity.

The fixed goal tool schemas are installed as a dormant ability for every
eligible persistent root session. This lets a direct natural-language request
reach `create_goal` without a heuristic intent classifier changing the tool
surface from prompt text. The explicit-intent restriction remains normative in
the model tool contract; deterministic runtime validation still covers the
objective, budget, identity, and lifecycle preconditions. No goal context
fragment, state, event, or automatic turn exists until a goal is present.
Child, review, and ephemeral sessions do not install the ability.

Pause, resume, objective edits, budget changes, and clear are typed user/host
operations. They are not available through `update_goal`, preventing the model
from overriding user- or system-controlled stopped states.

Alternative considered: one unrestricted `set_goal` tool. Rejected because it
would let the model resume after user pause, remove a budget, or replace
unfinished work without explicit authority.

### Automatic work uses a conditional internal-turn contract

Agent Runtime provides a bounded internal-turn input with explicit source,
revision, sensitivity, and hard size cap. `try_send_internal_if_idle` accepts a
goal continuation only if the session has no active or queued turn at the same
serialized decision boundary. It returns a structured busy result instead of
queueing behind user work. A concurrent real user submission therefore wins or
causes the goal attempt to be skipped and reconsidered at the next terminal
boundary.

An internal goal turn emits normal attributed lifecycle events and receives
the same provider, context, tool, approval, workspace, cancellation, retry,
checkpoint, and global-limit policy as an ordinary turn. Its internal input is
checkpointable but does not append a user-role message to canonical history.
The goal component contributes the bounded objective, status, and usage as an
authoritative no-cache context fragment.

A reusable goal controller attaches to the session after construction. It
observes durability-aligned goal and terminal events, deduplicates by goal id
and state generation, and tries one continuation whenever an active goal is
idle. It never recursively calls the provider from the TUI reducer and never
uses an unbounded timer loop.

Alternative considered: call `SessionHandle::send(UserInput::text(...))` from
Smith. Rejected because it would create fake user history and FIFO-queue
automatic work ahead of later user input.

### Accounting is safe-boundary enforcement with explicit provenance

Goal token usage is the sum of provider-reported uncached input and output
tokens attributable after that goal became active. Cached input is excluded.
The component snapshots cumulative usage at activation and updates deltas at
tool/output and turn-commit boundaries so concurrent notifications cannot
double count. Completing, pausing, clearing, or erroring a goal finalizes the
in-flight delta exactly once before the status mutation commits.

Active elapsed time advances only while an active goal owns a serving turn in
the current process. Idle time, paused time, process downtime, and time spent
before creation are excluded. Elapsed time is labelled derived; token counters
retain provider-reported or unknown provenance.

A token budget is an observed safe-boundary budget, not a pre-spend hard cap:
one provider request may overshoot it before usage is reported. Once observed
usage reaches or exceeds the budget, the state becomes `budget_limited`, no
further automatic turn starts, and any same-turn provider continuation receives
one bounded wrap-up instruction. The UI reports the actual overshoot.

If a budgeted goal reaches a boundary without the required trustworthy input
or output counters, Smith stops it as `blocked` with an
`accounting_unavailable` reason. It never continues while displaying a guessed
remaining budget. An unbudgeted goal may retain unknown token usage because no
token enforcement claim is being made.

Alternative considered: estimate missing usage. Rejected because an explicit
budget is a control boundary, and Smith's product policy forbids presenting an
estimate as provider-reported enforcement.

### User controls are local, serialized, and intentionally small

The first TUI surface supports:

```text
/goal
/goal <objective>
/goal edit <objective>
/goal budget <positive-tokens|none>
/goal pause
/goal resume
/goal clear
```

Bare `/goal` is a read-only summary. Create, edit, budget mutation, resume, and
clear require an idle session and use one typed goal-control path without
provider I/O. Budget mutation accepts a positive token count or `none`; a
budget-limited goal remains stopped after mutation until the user separately
resumes it. Pause is the exception: during an active goal turn it marks the
requested stop, interrupts that turn, finalizes accounting, and commits
`paused` exactly once. A non-goal turn interruption retains existing turn-local
behavior.

The TUI renders a compact goal status with status, derived elapsed time, and
reported/unknown token evidence. It does not add a second anchored pane; the
existing todo pane remains per-turn work detail. Live events and journal replay
must derive the same goal projection. Plain bounded objective text is accepted;
image, paste-file, and oversized-objective materialization are rejected or
reported locally rather than implemented implicitly.

Alternative considered: allow every mutation during a busy turn. Rejected for
the first slice because objective/budget replacement would require a broader
mid-stream steering contract. Pause covers the safety-critical busy action.

### Headless hosts follow the same goal to a stopped boundary

Ordinary `smith -p` remains one explicit turn. If that turn explicitly creates
or activates a goal, the headless host stays subscribed while the goal
controller starts internal continuations and exits when the goal becomes
complete, paused, blocked, usage-limited, or budget-limited, or when existing
process/global limits terminate execution.

Text output reports the final answer plus a concise goal terminal summary.
JSON includes an optional final goal record and continuation-turn count;
JSON Lines emits typed goal updates and each attributed turn lifecycle. Existing
non-goal fields retain their meaning. A goal that needs unavailable interactive
input stops as blocked and returns the existing structured
`interaction_required` outcome with the goal snapshot.

No headless goal runs after process exit. An active goal remains in the durable
snapshot and may continue only when the user explicitly resumes that session in
a later Smith process.

## Risks / Trade-offs

- Explicit goals can spend substantially more than one ordinary turn. Creation
  is opt-in, current usage remains visible, optional budgets stop at observed
  boundaries, and existing global limits remain authoritative.
- Provider usage commonly arrives only after a response, so token budgets can
  overshoot by one request. The product reports that behavior rather than
  describing the budget as a preflight reservation.
- The Agent Runtime contract expands to support internal turns and goal-aware
  usage views. A separate upstream proposal, release, pin, and consumer
  conformance pass are required before Smith implementation.
- Public goal objectives may contain user-authored sensitive text. This matches
  public todo/session-history posture; users needing protected content should
  reference an authorized artifact rather than place the secret in the goal.
- An unbudgeted explicitly requested goal may continue until a stopped state or
  existing system limit. Smith must keep interruption and shutdown responsive.

## Migration Plan

1. Capture ordinary-turn history, event, persistence, TUI, and headless fixtures
   as a no-goal compatibility baseline.
2. Approve and implement the coordinated Agent Runtime goal/internal-turn
   change with generic conformance coverage, then pin its immutable release.
3. Compose the goal ability and controller for persistent root sessions, add
   resume/accounting/error transitions, and keep child/review surfaces excluded.
4. Add local controls, compact rendering, journal replay, and headless
   multi-turn output behind the same Smith host construction path.
5. Run both repositories' format, Clippy, tests, consumer conformance, strict
   spec validation, and narrow/normal/wide terminal snapshots.

## Open Questions

None for proposal approval. Goal attachments, mid-turn edit/budget steering,
child goals, fork inheritance, analytics, app-server APIs, and remote execution
remain explicit follow-up decisions.
