---
created_at: 2026-10-19T00:57:31Z
updated_at: 2026-10-19T02:10:00Z
---

## Why

When a parent does not read its child's result with `agent wait` or
`agent result`, Smith delivers it automatically in a
`delegation.child-completion` turn. Two defects make that delivery unreliable:

- Agent Runtime builds the delivery as one internal turn capped at 4,096
  characters (`MAX_INTERNAL_TURN_CHARS`). A longer batch is refused as a
  conflict, and Smith's admission worker then stops trying until another
  outcome arrives. Every later batch still contains the long reply, so one
  report over about 4 KB stops automatic delivery for every later child in the
  session. A batch of several short replies hits the same cap.
- The delivery carries only each child's id and reply text. The artifacts
  copied to the parent from the child's turn are never named, so the parent
  model cannot know they exist unless it calls `agent result`.

## What Changes

- An automatic delivery is never refused because of its size. Each completed
  child's reply is delivered in full while it fits the delivery's inline
  budget. A reply that does not fit is stored as a parent-owned artifact, and
  the delivery carries a bounded excerpt plus that artifact's reference, so
  the parent can read the rest with `artifact.read`.
- The inline budget is Smith's existing `context.tool_output_inline_bytes`
  (8,192 bytes by default), the same threshold that already applies to an
  `agent result` tool result. No new setting.
- Every completed outcome in a delivery lists the artifacts transferred to
  the parent: the parent-owned id, the child-owned id it came from, media
  type, and size.
- When a session has no artifact store, an oversized reply is delivered as a
  bounded excerpt that points the parent to the child's result, which
  `agent result` returns under the ordinary output limits. Delivery still
  never stalls.
- Two repositories: the delivery builder and the internal-turn bound live in
  Agent Runtime (PR to `main`, then a Smith pin bump); Smith passes the
  resolved inline budget and owns the end-to-end tests.

## Alternatives considered

- **Deliver the whole reply with no bound**, as Claude Code's subagent
  notification appears to. Simpler for the model, but a runaway or verbose
  child could put an unbounded synthetic turn into the parent's context. A
  user who wants that can raise `context.tool_output_inline_bytes`.
- **Keep the 4,096-character cap and tell the parent to call `agent result`**.
  Most review and research reports exceed 4 KB, so most deliveries would
  become a pointer the model may not follow.

## Impact

- Affected specs: `child-agents` (two ADDED requirements). The offload reuses
  `context-working-set` behavior unchanged.
- Affected code: Agent Runtime
  `crates/agent-runtime/src/delegation/coordinator.rs`
  (`child_completion_content`, admission), `delegation/persistence.rs`
  (artifact transfer), `agent-runtime-core/src/content.rs`
  (`InternalTurnInput` bound for this source), and the exact-format test
  `tests/integration/external_agent/internal_turns.rs`. Smith
  `crates/smith-runtime/src/delegation.rs` (delivery budget wiring),
  `delegation/lifecycle.rs`, and `tests/delegation/`.
- Failing tests already written on branch `fix/child-result-delivery`:
  `tests/delegation/result_delivery.rs` (long reply delivered; a long reply
  does not block a later child) and an added assertion in
  `tests/delegation/artifacts.rs` (the delivery names the transferred
  artifact).
