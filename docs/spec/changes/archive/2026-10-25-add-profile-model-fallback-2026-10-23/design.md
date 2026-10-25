## Context

`ProfileSection` in `crates/smith-config/src/model.rs` has one `provider` and
one `model`; `ResolvedConfig` likewise resolves one model binding. The Smith
factory installs that binding's profile, context planner, reasoning
interceptor, capabilities, and cache identity into `RuntimeBuilder`.

`PooledProvider` in `crates/smith-runtime/src/rotation.rs` reuses one adapter
and request while asking to change credentials. Its accepted-stream fence is
deliberate; changing providers or context limits inside that decorator would
leave the shared planner and usage attribution bound to the wrong model.

The pinned Agent Runtime revision
`bba8996c213f2f65287d84befc1c7dc6ced3241a` owns retries in
`agent/driver/provider.rs`. It tracks semantic response progress, but its
retry loop reuses a planned request. Cross-model fallback therefore requires
a shared request-boundary route/replan contract, not a provider decorator
that silently substitutes the model id.

## Goals

Provide ordered cross-provider fallback on unavailability and a user-set
first-response deadline, without changing agent identity or replaying tools.
Use the same behavior for ordinary main/child turns and TUI/headless hosts.

## Decisions

### Configuration and resolution

Use `[profiles.<name>.fallback]` with `models` and an optional positive
`first_response_timeout_ms`. Keep existing primary fields unchanged. Omitting
the table preserves current behavior. Missing fields inherit normally;
`models` replaces the entire inherited or lower-layer list, and `models = []`
disables the chain, including its startup timer.

Each model entry requires `provider` and `model`, and may override
`reasoning`, `max_output_tokens`, `context_window`, and
`first_response_timeout_ms`. The primary profile's reasoning and output ask
remain defaults unless an entry overrides them. An explicit candidate
`reasoning` table replaces lower-layer reasoning controls; an empty table
selects model defaults. Invocation controls retain precedence. Named context
windows are model-specific: fallbacks use their own model's default unless
their entry or a higher invocation layer explicitly selects a window. Explicit control
values are validated, never silently downgraded or clamped.

Validate every candidate's declarations, limits, supported controls, and
credential-reference shape before starting. Acquire credential sources only
for the selected candidate; an unavailable source marks that candidate
unavailable without launching an enrollment flow. Reject duplicate bindings,
including the primary, and `cli/...` harness selections. Explicit CLI or
session provider/model selection disables this profile chain until the
profile is selected again. A reasoning/window/output override keeps its
normal precedence and must be compatible with every enabled candidate.

### Latency policy

The user selected fallback when no response starts within a configurable
time. The timer is a candidate budget for the current logical provider
request, starting at dispatch. It includes connection/open time, empty-stream
waiting, retries, and backoff. It does not include local tool execution or
time spent waiting for user input. Retries and credential renewal cannot
restart it. Each next candidate gets its own budget, bounded by the same
original turn deadline.

Nonempty text, meaningful reasoning, tool-call activity, or hosted-tool
activity ends the startup window; successful completion also ends it.
Headers, keepalives, empty deltas, and usage-only observations do not.
Expiry advances immediately rather than spending the same model's remaining
retry count. The timeout is optional, with no new latency cutoff by default;
30 seconds in the proposal is an example, not a hard-coded default.

Cancel and retire the old local attempt before dispatching another. Runtime
must reject late events by attempt identity and release the old stream and
transport. Server-side inference might already have consumed tokens: retain
reported usage and label unavailable usage unknown.

After meaningful progress, this startup timer is disarmed and automatic
cross-model replay is forbidden for that request. Ordinary existing turn and
attempt limits still apply. An accepted stream without meaningful output may
be retired only through an adapter contract proving pre-output replay is
safe. A request allowing provider-hosted side effects cannot use latency
replay after possible dispatch unless the shared adapter contract proves
those effects cannot already have started or can be safely deduplicated;
otherwise fail at the timeout without advancing. This applies even before
stream creation. Do not infer safety merely from silence on the stream.

### Failure policy and route lifetime

Advance immediately on typed `LimitExhausted`, or after existing bounded
retries on `Network`, `Timeout`, `Server`, and `RateLimited`, only while the
current request is safe to replay. Authentication, generic `BadRequest`,
unsupported capabilities, malformed streams, cancellation, and model/control
validation are not automatic fallback triggers. Generic HTTP 404 is not
treated as proof that a model is unavailable.

An enabled chain takes precedence over reactive account-rotation prompts on
quota exhaustion. Record the current member's cooldown and return the typed
failure to the shared fallback controller. Profiles without a chain retain
existing rotation behavior; manual and proactive rotation remain unchanged.

Visit each binding at most once per user turn, preserving the turn's tool,
time, and cancellation budgets. The successful candidate continues subsequent
tool steps; a later pre-response failure may advance to the next candidate.
New user turns and resumed sessions start at the configured primary. No
cross-turn circuit breaker or persistent preferred fallback is introduced.

### Replanning and history

Before dispatch on a new binding, re-resolve limits, modalities, reasoning
dialect, context and output budgets, model-facing tools and hosted services,
and cache endpoint/account identity. Run normal compaction when needed. Skip
a candidate that cannot represent the current history or required modalities
without dropping protected content. Clear previous provider response handles,
cache baselines, and provider-private continuation data; preserve canonical
messages, tool results, profile instructions, and all authority boundaries.

Rebuild only the pending provider request. Never resubmit the original user
turn or rerun previously completed tool calls. Usage records and prices stay
bound to the actual attempted provider/model, including failed and timed-out
attempts. Live model/status reporting changes with the route while the
selected profile name remains constant.

### Ownership and rollout

Agent Runtime owns candidate activation, request replanning, deadlines,
cancellation and replay fencing, canonical events, and attempt accounting.
Smith owns declarative selection, candidate preparation through its existing
factory, host policy, and client projection. Agree the additive shared API and
prove its consumer conformance before updating Smith's immutable runtime pin.
Implementing a second retry loop in `smith-cli` or switching an adapter behind
an unchanged `RuntimeBuilder` profile is not an alternative.

## Risks / Trade-offs

- A shorter first-response cutoff may abandon valid slow reasoning before it
  emits progress. Users set the threshold and may override it per candidate.
- Sequential cancellation reduces overlapping work but cannot guarantee a
  remote provider refunds or stops inference. Unknown usage stays unknown.
- Resetting to the primary each new turn can repeat a known outage check;
  persistent health routing is deliberately deferred.
- A smaller or less capable fallback may be skipped after replanning. The
  terminal result must explain all failed/skipped candidates.
- A configured cross-provider chain authorizes resending the conversation to
  those declared providers. Explain output and guides must make that clear.

## Migration Plan

No existing config or session migration is required. New fields and event
metadata are additive, absent fallback policy keeps the old behavior, and
older snapshots remain readable. Historical route identity is preserved for
usage and replay, independently of the next turn's preferred route.

Fallback-enabled turns opt into checkpoint transition table 5. The frozen
revision-4 table and operation fingerprints remain unchanged. An in-flight
revision-4 turn finishes under its original table; fallback applies to new
turns admitted after it. Both materialized and native journal readers accept
known revisions, and reject unknown revisions or mid-turn table changes.

## Implementation dependency

The exact shared route/replan API and adapter pre-output replay-safety
contract must be finalized in Agent Runtime. This is the first implementation
task and a release gate; the currently pinned retry loop is not sufficient.
