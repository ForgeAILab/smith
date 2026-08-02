---
created_at: 2026-08-02T07:15:49Z
updated_at: 2026-08-02T16:30:37Z
---

## Why

Smith currently accepts another `SessionHandle::send(UserInput)` while a turn
is serving, so Agent Runtime silently runs that input as a later whole turn.
The TUI renders the text immediately but cannot show, edit, remove, or steer it
into the active turn, making an important busy-turn action look more immediate
than it is.

## What Changes

- **BREAKING** Reserve `Enter` on an ordinary user prompt during eligible
  provider-backed work for active-turn steering, while `Tab` on a non-empty
  ordinary prompt explicitly queues a later whole turn.
- Add bounded process-local pending-input state that distinguishes accepted
  steers, rejected steers awaiting follow-up, and queued future turns without
  prematurely adding any of them to the transcript.
- Render a compact pending-input preview in the existing anchored composer
  area and allow the newest queued future turn to be restored for editing.
- Submit exactly one queued follow-up after a successful terminal boundary;
  preserve or restore pending input across rejection, interruption,
  reconfiguration, and local materialization failure without duplication.
- Integrate only through Agent Runtime's coordinated typed steering contract;
  Smith does not reinterpret the generic injection inbox or build a second
  provider/tool loop.
- Keep local slash commands, shell shortcuts, child-agent actions, approvals,
  and questionnaires outside automatic future-turn queuing in this slice.

## Impact

- Affected specs: `client-interaction`, `client-surfaces`,
  `runtime-integration`
- Affected code: `DESIGN.md`, `crates/smith-tui` composer/app/rendering,
  `crates/smith-cli` interactive host routing, deterministic TUI/runtime tests,
  and the Agent Runtime dependency revision
- Coordinated runtime change:
  `../agent-runtime/docs/spec/changes/add-active-turn-steering-2026-08-02/`
- Active-change coordination:
  `add-persistent-session-goals-2026-08-02` remains authoritative for real
  user priority over automatic continuation, while
  `add-smith-agent-harness-2026-07-23` remains authoritative for the generic
  safe-boundary injection inbox.

## Approval Boundary

Approval authorizes Smith's interactive steering and ordinary user-turn queue
policy after the coordinated Agent Runtime contract is available. It does not
authorize queued host commands, durable pending drafts across process exit,
background execution, new headless input protocols, or changes to approval and
questionnaire ownership.
