## Context

`agent.wait` today slices a foreground wait into 30 s runtime calls
(`RUNTIME_WAIT_SLICE_MS`) up to a five-minute soft boundary
(`DEFAULT_WAIT_TIMEOUT_MS`) and then returns `running` with a timeout marker.
Parking (`parked-awaiting-child`) and automatic `delegation.child-completion`
delivery already exist, so the blocking wait adds nothing a parked parent
cannot get — except a way for the model to say "check back on me later".
Models use the blocking wait for exactly that, which is the behavior this
change keeps without blocking.

## Goals / Non-Goals

- Goals:
  - No `agent` tool call stays open waiting for a child.
  - A model that wants a deadline still gets woken at that deadline.
  - Completion delivery, acknowledgement, and admission priority stay as they
    are.
- Non-Goals:
  - Automatic re-arming or periodic status wakes (that recreates polling).
  - Persisting armed timers across restart.
  - Partial child output in the timeout wake (transcript inspection already
    exists for that).
  - Changing the image/snapshot size issue seen in the same sessions (separate
    change).

## Decisions

- Decision: `wait` returns immediately.
  - Result: `{child_id, state, armed, wake_after_ms, deadline}` while running;
    the terminal status and acknowledgement (unchanged) when already terminal.
  - `timeout_ms = 0`: immediate status, `armed: false`.
  - Omitted `timeout_ms`: resolved default (built-in five minutes).
  - Above the maximum: clamped, with `clamped: true` in the result. Rejecting
    produced error/retry loops for no benefit once the wait no longer blocks.
- Decision: one armed timer per child; re-arming replaces it.
- Decision: the wake is Smith-side. Smith already admits its own idle-only
  internal turns through `SessionHandle::try_send_internal_if_idle` with an
  `InternalTurnSource` (see module notifications in `host/watchers.rs`). The
  wait timer uses the same path with source kind `delegation.wait-timeout`.
  This corrects the earlier assumption that Agent Runtime needed a new timer
  primitive; Runtime is only touched if dedupe against child completion
  turns out to be impossible from coordinator status (not expected).
- Decision: dedupe and ordering.
  - Terminal outcome injected for a child → its timer is cancelled.
  - Timer fires but the child is terminal (outcome pending or delivered) →
    the timeout wake is dropped; completion delivery wins.
  - Timer fires while the parent is serving → the timer stays expired and
    its notice is admitted as a `delegation.wait-timeout` continuation when
    that turn ends. It is not injected into the running turn, and no
    competing turn starts. (Owner decision, 2026-10-24: turn-end delivery.)
  - Several timers expire before admission → one continuation carries every
    expired, still-running child in deterministic child-ID order.
  - Real user input keeps priority over a timeout wake, as over completion.
- Decision: the timeout wake text is host-authored and bounded: child ID,
  profile, state `running`, elapsed time, armed duration. It invites the model
  to decide whether to keep waiting (re-arm), inspect, stop, or continue.
- Decision: no minimum timer is imposed. Guidance explicitly says the parent
  wakes automatically and should end its turn rather than poll.
- Decision: agent/tool waits retain the five-minute cap. General monitors may
  run indefinitely; they do not extend the agent wait timer range.
- Decision: the delegated-work panel shows a `monitor` tag and wake deadline
  for each armed child, clearing it after admission or cancellation.
- Decision: headless hosts honor the timer only while they are already alive
  for required child work; a timer alone never extends process life.
- Decision: harness (CLI-bridged) roots get the same immediate return; the
  timeout wake runs on the harness exactly as child-completion turns do.

## Risks / Trade-offs

- A model may loop arm → wake → arm every five minutes. Each wake is one
  short turn instead of a blocked tool call, which is still strictly cheaper
  in wall-clock and keeps the parent interruptible. No minimum timer is
  imposed; automatic-wake guidance discourages polling.
- A model that arms a timer and keeps working gets the notice only after its
  current turn ends, so a long turn can deliver it well past the deadline.
  Turn-end delivery keeps the running turn's context unchanged.
- Removing the foreground path removes the 30 s slicing code; existing tests
  that assert a blocking wait must be rewritten, not deleted.

## Migration Plan

No data migration. Old checkpoints contain only completed `wait` tool results,
which remain valid history. The config keys keep their names, so existing
user files resolve unchanged.

## Approval

Approved for implementation on 2026-10-23, with no minimum timer, the existing
five-minute wait cap, and a visible monitor tag with each child's wake time.
