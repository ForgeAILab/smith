---
created_at: 2026-10-19T00:57:31Z
updated_at: 2026-10-19T04:00:00Z
completed_at:
---

## 1. Failing tests (Smith)

- [x] 1.1 `tests/delegation/result_delivery.rs`: a child reply over 4,096
  characters reaches the parent without a user turn; a long reply does not
  block a later child's delivery.
- [x] 1.2 `tests/delegation/artifacts.rs`: the automatic delivery names the
  parent-owned artifact id.
- [x] 1.3 Record each test's failure output against the pinned runtime
  (runtime `e38809e`, 2026-10-18): both `artifacts` cases fail because the
  delivery is only `- child child-1 completed:\nchild artifact ready`; the
  long-reply test sees 1 provider request (the child) and no delivery in
  10 s; the blocking test sees 2 (both children) and no delivery for either.
- [x] 1.4 A reply larger than the budget is delivered as an excerpt plus a
  parent-owned reference, and reading that reference returns the exact
  reply (`a_reply_over_the_inline_budget_is_delivered_as_an_excerpt_and_a_readable_copy`,
  budget 2,048 bytes). The 1.1 reply is about 6,200 characters: over the
  old cap, under the 8 KiB default budget.

## 2. Agent Runtime (PR to `main`)

Recorded in Agent Runtime as
`docs/spec/changes/update-child-completion-delivery-2026-10-18`, PR #55.

- [x] 2.1 Runtime unit tests: the existing delivery text is unchanged for a
  short reply; a reply within the budget is whole; an over-budget reply is an
  excerpt with a stored reference, or a pointer to the child's result
  without one; replies inline in order while they fit; artifacts are listed;
  a batch over the ceiling is delivered compactly; only child-completion
  turns exceed the general internal-turn bound.
- [x] 2.2 `DelegationConfig::delivery_inline_bytes` (default 4,096, max
  1 MiB), validated when the coordinator is built.
- [x] 2.3 A reply longer than 1,024 bytes is stored on completion and on
  recovery as a parent-owned artifact with purpose `delegation.child-reply`,
  keyed by parent, child session, child, turn, and text, so a repeat returns
  the same copy (`ChildTaskResult::reply`). Replies are inlined in batch
  order while they fit; replies of at most 1,024 bytes are always inline; the
  rest are delivered as their first 1,024 bytes plus the reference. Storing
  happens at completion because admission builds its content synchronously
  under the outcome gate.
- [x] 2.4 `delegation.child-completion` turns are validated against
  `MAX_CHILD_COMPLETION_TURN_CHARS` (2 Mi characters); other internal turns
  keep 4,096. A batch over that ceiling is rebuilt as one line per outcome.
- [x] 2.5 Each completed outcome lists its transferred artifacts (parent id,
  child source id, media type, size), 32 at most, then a count.
- [x] 2.6 The exact-format assertion in
  `tests/integration/external_agent/internal_turns.rs` needed no change: a
  short reply without artifacts keeps the old text.
- [x] 2.7 No checkpoint or session-state revision constant changed. PR #55
  merged to runtime `main` as `70c4f96` after CI passed (the macOS job's one
  failure, `task_2_6_crash_matrix_accepted`, also fails on `main` and passed
  on rerun).

## 3. Smith

- [x] 3.1 Pin runtime `70c4f96` (still version 0.7.0); no `[patch]` table.
  The branch is stacked on `release/v0.7.5`, which moves the pin from
  `e38809e` to `2b6c1e3`.
- [x] 3.2 Pass the parent route's resolved inline bytes
  (`tool_output_context.inline_bytes`, from `context.tool_output_inline_bytes`)
  as the root coordinator's delivery budget. Children have no coordinator
  (one-level hierarchy), so only the root delivers.
- [x] 3.3 All 30 `tests/delegation` tests pass against the runtime branch,
  including completion, parking, and no-double-delivery.
- [x] 3.4 Gate on the pinned revision with `--locked`: fmt, clippy, 1.88
  clippy, fixtures, `--no-default-features` build and tests, smoke, and deny
  pass; 3,137 tests passed and 1 failed. The failure,
  `interactive_rebind::metadata_adoption_waits_for_open_picker_and_active_turn_then_keeps_session_and_draft`,
  passed 5 of 5 alone and in every later full run. The `smith-cli` test
  binary also aborts with SIGABRT in some parallel runs (3 of 5 here, 2 of 6
  on `release/v0.7.5` without this change), never single-threaded.
- [ ] 3.5 Push the branch for CI; land on `main` after `release/v0.7.5`.
