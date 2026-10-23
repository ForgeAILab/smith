---
created_at: 2026-10-23T09:39:03Z
updated_at: 2026-10-23T11:48:59Z
completed_at:
---

## 1. Implementation

- [x] 1.1 Replace the foreground wait in `delegation/tool.rs` with an immediate
  return: terminal status + acknowledgement when terminal, otherwise arm a
  timer and return `armed`, `wake_after_ms`, `deadline`, and `clamped`.
- [x] 1.2 Change `DelegationWaitPolicy::resolve_timeout` to clamp values above
  the maximum instead of rejecting them; keep `0` as immediate status.
- [x] 1.3 Add a per-session wait-timer registry (one timer per child, replace
  on re-arm, cancel on terminal outcome injection, cancel all on shutdown).
- [x] 1.4 On expiry, re-check coordinator status; drop if terminal, otherwise
  queue a bounded host-authored status notice and attempt
  `try_send_internal_if_idle` with source kind `delegation.wait-timeout`,
  batching every expired running child in child-ID order.
- [x] 1.5 Deliver a queued timeout notice at the next safe boundary when the
  parent was busy, without starting a competing turn.
- [x] 1.6 Apply the same behavior to bridged harness roots; keep headless
  process lifetime tied to required child work only.
- [x] 1.7 Rewrite the `agent` tool description and root delegation guidance
  around arming a wait and ending the turn.
- [x] 1.8 Project `delegation.wait-timeout` turns in TUI and headless lifecycle
  output alongside `delegation.child-completion`.
- [x] 1.9 Remove `RUNTIME_WAIT_SLICE_MS` and the foreground slicing loop once no
  caller remains.
- [x] 1.10 Show a `monitor` tag and wake deadline for each armed child in the
  delegated-work panel; clear it on admission, completion, and shutdown.

## 2. Tests

- [x] 2.1 `wait` on a running child returns within one scheduler tick with
  `armed: true` and opens no long-lived tool call.
- [x] 2.2 `wait` on a terminal child returns and acknowledges the outcome; no
  child-completion continuation follows.
- [x] 2.3 Timer expiry on an idle parked parent admits exactly one
  `delegation.wait-timeout` turn with bounded status and no partial output.
- [x] 2.4 Child completes before expiry: timer cancelled, only the
  child-completion turn runs.
- [x] 2.5 Timer expires while parent serves a user turn: no competing turn;
  notice supplied at the next safe boundary.
- [x] 2.6 Expiry and completion race: completion wins, no timeout wake.
- [x] 2.7 Re-arm replaces the earlier deadline; two children expiring together
  produce one continuation in child-ID order.
- [x] 2.8 Over-maximum `timeout_ms` is clamped and reported.
- [x] 2.9 Shutdown with armed timers starts no wake and stays bounded.
- [x] 2.10 Harness-root fixture: immediate return and timeout wake on the
  harness.

## 3. Verification

- [ ] 3.1 Full gate (`--no-fail-fast`, clippy, `--no-default-features` steps).
- [x] 3.2 Live TUI check: spawn a child, call `wait`, confirm the parent parks
  immediately and wakes on completion or at the deadline.
- [x] 3.3 Update `docs/` delegation guidance pages that describe `wait`.

Full-gate evidence is recorded in [verification.md](verification.md). Task 3.1
remains open for the catalog-adoption fixture failure in the isolated tree.

## 4. Guidance follow-up (`session-1878584b`)

A gemini-4-argon root wrote "Parking…", then in the same response announced
child completions it had not received and read about sixty invented artifact
IDs, all `artifact not found`. Every real ID came from a genuine
child-completion turn.

- [x] 4.1 Rewrite root delegation guidance: ending a turn means no further tool
  calls, outcomes arrive only as later host messages, never author or predict
  them, copy artifact IDs exactly.
- [x] 4.2 Rewrite the `agent` description around the same rules; `spawn`,
  queued spawn, and `follow_up` results carry a note saying not to predict
  the outcome.
- [x] 4.3 Report a child's `result` only while it is idle, so `list` and
  `stop` never pair an earlier task's answer with a later state.
- [x] 4.4 Tests: guidance quotes no delivery sentinel; a running follow-up
  reports no earlier result.
