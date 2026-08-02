---
created_at: 2026-08-02T07:15:49Z
updated_at: 2026-08-02T16:30:37Z
completed_at: 2026-08-02T16:30:37Z
---

## 1. Contract and Design

- [x] 1.1 Land or pin the approved Agent Runtime
  `add-active-turn-steering-2026-08-02` contract, including typed admission and
  committed/discarded dispositions.
- [x] 1.2 Update `DESIGN.md` keyboard, anchored-area, pending-input,
  interruption, and non-goal contracts before enabling behavior.
- [x] 1.3 Introduce a Smith-owned prepared ordinary submission type that
  preserves display text, paste expansion material, and canonical file
  identities without performing provider I/O.

## 2. Pending Input State

- [x] 2.1 Add bounded process-local state for accepted pending steers,
  rejected-steer follow-ups, and explicitly queued future turns.
- [x] 2.2 Track the serving runtime turn identity and stable steer identities
  from typed envelopes/dispositions.
- [x] 2.3 Add reducer operations for FIFO insertion, one-at-a-time drain,
  committed removal, discarded restoration, and newest queued-turn editing.
- [x] 2.4 Make pending input participate in exit/reconfigure live-work policy
  without claiming process-restart durability.

## 3. Composer and Host Routing

- [x] 3.1 Preserve overlay precedence and map busy ordinary `Enter` to steer,
  busy non-empty ordinary `Tab` to queue, and `Alt+Up` to edit the newest
  explicitly queued turn.
- [x] 3.2 Keep slash commands, shell shortcuts, child-agent actions,
  approvals, questionnaires, and idle profile cycling on their existing paths.
- [x] 3.3 Refactor the interactive host loop through one prepared-submission
  dispatcher for ordinary send, file materialization, steer, fallback, and
  local error restoration.
- [x] 3.4 Handle no-active-turn and expected-turn races, runtime-declared
  non-steerable work, bounds, shutdown, and materialization failures without
  dropping or duplicating input.
- [x] 3.5 Submit exactly one rejected/queued follow-up after an eligible
  successful terminal boundary before automatic goal continuation admission.
- [x] 3.6 Implement interrupt-for-steer resubmission and conservative restore
  behavior for other non-success terminal outcomes.

## 4. Rendering and Guidance

- [x] 4.1 Render bounded pending-steer, rejected-follow-up, and queued-turn
  sections in the existing anchored composer budget with overflow counts.
- [x] 4.2 Add conditional key hints and update `/help` so `Enter`, `Tab`,
  `Alt+Up`, and `Esc` behavior is explicit while busy.
- [x] 4.3 Verify narrow terminals, wrapping, todo coexistence, modal priority,
  cursor placement, and non-color accessibility.

## 5. Verification

- [x] 5.1 Add reducer tests for busy key routing, FIFO category ordering,
  editing, queue bounds, prompt ownership, and exact draft restoration.
- [x] 5.2 Add deterministic runtime integration tests for streaming steer,
  tool-boundary steer, final-response continuation, stale-turn fallback,
  rejected steering, and exactly-once transcript commitment.
- [x] 5.3 Add interruption tests proving committed steers are not resent and
  uncommitted steers are neither lost nor duplicated.
- [x] 5.4 Add paste/file tests proving queued material is lossless and files
  are read at dispatch time with safe restoration on failure.
- [x] 5.5 Add goal-controller race tests proving real user steer/queue work
  wins before idle-only automatic continuation.
- [x] 5.6 Run formatting, Clippy, Smith workspace tests, Agent Runtime consumer
  conformance, and update evidence with the exact runtime revision.
