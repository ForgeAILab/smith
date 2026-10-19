---
created_at: 2026-10-19T00:57:31Z
updated_at: 2026-10-19T02:10:00Z
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
- [ ] 2.7 No checkpoint or session-state revision constant changed. Merge
  PR #55 to `main` after CI.

## 3. Smith

- [ ] 3.1 Pin the merged runtime revision; remove the local `[patch]`.
- [x] 3.2 Pass the parent route's resolved inline bytes
  (`tool_output_context.inline_bytes`, from `context.tool_output_inline_bytes`)
  as the root coordinator's delivery budget. Children have no coordinator
  (one-level hierarchy), so only the root delivers.
- [x] 3.3 All 30 `tests/delegation` tests pass against the runtime branch,
  including completion, parking, and no-double-delivery.
- [ ] 3.4 Run the gate (`--no-fail-fast`, 1.88 clippy, CI's
  `--no-default-features` steps) on the pinned revision, push the branch for
  CI, then merge to `main`.
