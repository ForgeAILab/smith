## Context

Smith's composer currently returns `Action::Send` for an ordinary prompt
regardless of `App::is_busy`. The interactive host calls
`SessionHandle::send`, whose FIFO scheduler accepts another whole turn. This is
mechanically safe but invisible: the TUI immediately appends a user transcript
row, discards the returned `TurnHandle`, and has no queue state to preview or
edit.

Codex separates two intents during a running turn. A normal submission targets
the serving turn and is consumed at the next safe model/tool boundary; an
explicit queue action remains UI-owned until a later turn may start. Smith
needs the same distinction while retaining its simpler composer, anchored todo
area, typed runtime facade, and goal-controller ordering.

## Goals / Non-Goals

### Goals

- Let a user amend eligible serving work without mutating an in-flight provider
  request or fabricating a separate turn.
- Make later-turn intent explicit, bounded, visible, editable, and FIFO.
- Keep optimistic UI state separate from canonical transcript commitment.
- Preserve exact ordinary prompt materialization, including paste placeholders
  and file references, across steering and queuing.
- Preserve real-user priority over goal-controller internal admission.
- Reuse Agent Runtime's typed turn identity, admission, disposition, and
  provider/tool-loop behavior.

### Non-Goals

- Queueing slash commands, shell shortcuts, child-agent operations,
  questionnaires, approvals, reconfiguration, or recovery actions.
- Persisting pending steers or queued future turns across process exit.
- Steering returned-interaction, local-tool-only, or otherwise runtime-declared
  non-steerable work.
- Adding a daemon, background worker, multiple visible sessions, or headless
  asynchronous input protocol.
- Reusing generic monitor/child injection as an indistinguishable user steer.

## Decisions

### Enter steers and Tab queues only ordinary prompts while busy

When no overlay owns input, `Enter` on a locally valid ordinary prompt targets
the eligible serving turn. `Tab` queues that prompt only when the runtime is
busy and the composer is non-empty. Idle empty `Tab` keeps profile cycling;
palette and picker `Tab` behavior keeps precedence. Slash, shell, and child
syntaxes continue through their existing explicit paths rather than silently
changing execution time.

Alternative: make every busy `Enter` a future turn. Rejected because it
preserves today's ambiguity and cannot correct ongoing work. Alternative: let
`Tab` queue parsed host commands. Deferred because commands have different
idle, confirmation, authority, and local-side-effect contracts.

### Smith owns future turns until dispatch

`PendingInputState` contains three ordered categories: steers accepted by the
runtime but not yet committed, runtime-rejected steers that must become the
first follow-up, and explicitly queued ordinary turns. A queued entry retains
the exact display text, paste material needed for expansion, and canonical
workspace-relative file identities. File content is read when the queued turn
actually starts so it observes the workspace produced by the preceding turn.

Smith never calls `SessionHandle::send` merely to stage a future TUI turn.
Once accepted by the runtime, a whole turn is no longer editable through the
product queue. The lower-level runtime FIFO remains available to other hosts
and concurrent callers but is not Smith's queue UI.

Alternative: store returned `TurnHandle`s and cancel queued runtime turns to
edit them. Rejected because cancellation is not queue mutation, transcript
ordering would remain optimistic, and local file/paste materialization errors
would occur after the user lost editable state.

### Runtime disposition controls transcript commitment

Smith tracks the serving `TurnId` from event envelopes. A successful steering
call returns a stable steer identity and leaves the text in the pending preview.
When Agent Runtime emits the matching committed disposition, Smith closes any
open assistant block, appends the user row at that exact boundary, and removes
the pending preview. A discarded disposition restores the original material to
the front of follow-up handling without duplicating composer history.

An expected-turn mismatch is retried at most once only when the runtime reports
the actual still-eligible turn. No-active-turn races become an ordinary new
turn if the session is idle; runtime-declared non-steerable work becomes the
first queued follow-up. Limit or materialization failures restore the exact
draft and remain visible as local errors.

Alternative: append the user transcript row when `Enter` is pressed. Rejected
because acceptance is not commitment and interruption may discard the steer.

### Terminal boundaries drain conservatively

After a successful ordinary terminal event, Smith dispatches exactly one
future turn: rejected steers first (merged in FIFO order when the runtime
rejected a batch), then explicit queued turns. Starting one turn re-establishes
busy state before another may drain.

When `Esc` is pressed with uncommitted steers, Smith marks an
interrupt-for-steer intent. After the cancelled terminal disposition, it
merges those steers in FIFO order and sends one immediate ordinary turn. Other
cancelled, failed, limited, or needs-input boundaries restore uncommitted
steers and queued material to the composer rather than auto-spending.

### Pending input shares the anchored composer region

The renderer adds no second permanent pane. Within the existing anchored
budget it shows bounded sections for pending steers, rejected follow-ups, and
queued turns, with at most three preview lines per section plus overflow
counts. Modal/picker ownership remains highest priority; pending input then
shares remaining rows with public todo state. Text labels and key hints carry
meaning without color.

The newest explicitly queued turn can be popped back into the composer with
`Alt+Up`; this never edits a steer already accepted by the runtime. Pending
process-local input contributes to exit confirmation and is cleared only by
explicit discard, successful dispatch/commit, or terminal session shutdown.

### Goal scheduling keeps real-user priority

The goal controller's idle-only internal admission remains serialized with
ordinary user work. A TUI-owned queued future turn is submitted at the terminal
boundary before Smith permits an automatic goal continuation attempt. A user
steer accepted into a serving goal-owned turn remains real user input inside
that turn and does not fabricate a new goal objective or relax goal accounting.

## Risks / Trade-offs

- The key contract changes during busy work; the footer and `/help` must state
  the conditional behavior clearly.
- Pending input is process-local until runtime commitment. A crash may lose it,
  so the UI must not label a pending steer durable.
- File references in a queued entry observe dequeue-time content. This is
  intentional but differs from freezing a snapshot at keypress time.
- Multiple pending categories create interruption and race edge cases. Stable
  identities, one runtime disposition per accepted steer, and deterministic
  tests are required.
- The active goal changes and steering both touch session admission. The exact
  Agent Runtime revision must include both compatible contracts before Smith's
  consumer gate is recorded.

## Migration Plan

1. Land and validate Agent Runtime's typed active-turn steering contract.
2. Update `DESIGN.md` and Smith reducer state without enabling busy key paths.
3. Route ordinary prompt materialization through one prepared-submission type.
4. Add local queue/rejected/pending state, rendering, edit, and terminal drain.
5. Enable `Enter` steering and `Tab` queuing with runtime race handling.
6. Run deterministic unit, integration, replay, goal-ordering, and workspace
   gates, then record the exact runtime revision.

## Open Questions

None for proposal approval. Durable pending drafts, queued host commands,
multiple queue-edit operations, and headless asynchronous steering require
separate follow-up decisions.
