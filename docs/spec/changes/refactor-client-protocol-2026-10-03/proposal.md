---
created_at: 2026-10-03T00:00:00Z
updated_at: 2026-10-04T03:47:42Z
---

## Why

Roadmap step 6 in the [2026-10-02 audit](../../../qa/smith-structure-2026-10-02/report.md)
(S10, S11) asked for a decision on the half-built client protocol. Decision
(2026-10-03, under the owner's "continue with the road map"): **remove the
unused command half and keep the event projection.**

- The command half (`SmithInput`, `TurnReceipt`, `SteerReceipt`,
  `SteerRejection`, `SmithSession::submit/steer/cancel`) has no caller. Every
  host drives sessions through `SessionHandle` — submit, steer, interrupt,
  local actions, history, persistence — and finishing the protocol would mean
  growing `SmithSession` to all of that with no second client to shape it.
- `docs/architecture.md` and the `SessionHandle` re-export have called the
  handle deprecated "for one migration release" for 16 releases. The honest
  fix is to retract that, not to make it true by a large migration. The
  typed, redaction-safe `SmithEvent` projection — what the TUI, headless, and
  replay consume, and what an out-of-process client would need — stays.

The event projection itself has drifted: the pinned runtime emits
`BudgetFailure` and `RateLimitObservation`, which `SmithEventKind` does not
mirror, so both arrive as `Unknown` at runtime and a context or output budget
failure never reaches the user. Nothing makes the next runtime bump fail when
it adds a variant. Boundaries are checked by name: `smith-runtime` exports 40
of 42 modules and the architecture test checks that files exist and two
strings are absent.

## What Changes

- Remove the command half; keep `SmithSession` as the event adapter. Remove
  the deprecation on the `SessionHandle` re-export and correct
  `docs/architecture.md`.
- Mirror `BudgetFailure` and `RateLimitObservation` in `SmithEventKind`; show a
  budget failure to the user. A test matches every runtime event variant
  exhaustively, so an unmirrored variant fails to compile on the next pin
  bump instead of becoming `Unknown`.
- Make `smith-runtime` modules with no consumer outside the crate private and
  deny `unreachable_pub` there.
- Replace the name-based architecture tests with structural ones, and correct
  the module docs S11 names.

## Impact

- Affected specs: runtime-integration, code-organization.
- Affected code: `smith-runtime` `client.rs`, `lib.rs` (module visibility),
  `tests/architecture.rs`; consumers of removed types (none outside
  `client.rs`); TUI and headless handling of the two new event kinds.
- Machine output: headless stream events for the two kinds change from
  `unknown` to typed records; nothing else.
