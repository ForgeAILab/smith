---
created_at: 2026-10-23T05:12:51Z
updated_at: 2026-10-23T05:55:57Z
completed_at: 2026-10-23T05:55:57Z
---

## 1. Implementation

- [x] 1.1 Establish the canonical child transcript read/recovery seam, including
  parent ownership, persisted children without a live handle, stable cursors,
  and a history/live watermark. If an upstream API is required, implement it
  in Agent Runtime and update Smith's immutable pin before using it.
- [x] 1.2 Add a shared bounded transcript projection with roles, task text,
  assistant content, tool call/result pairing, and artifact-backed large
  entries. Apply existing credential and reasoning visibility rules; exclude
  internal instructions and protected checkpoint fields.
- [x] 1.3 Add root-only `agent transcript` with `child_id` and pagination.
  Reject unknown or unrelated child IDs. Transcript inspection must preserve
  automatic completion readiness and must never execute the child.
- [x] 1.4 Seed the child inspector from canonical history and merge live
  events exactly once. Recover late subscriptions, reconnects, and retained
  older pages. Preserve the root composer, scroll state, and existing style.
- [x] 1.5 Present observed provider/tool activity and progress age alongside
  the conversation, including an honest unavailable-history state. Do not
  infer thinking or a particular blocked tool from silence.
- [x] 1.6 Update capability descriptions, delegation prompt guidance, and
  session documentation: continue independent work or park, receive completed
  results automatically, inspect transcripts on demand, and use `agent stop`
  for children rather than shell-task controls.

## 2. Verification

- [x] 2.1 Verify a child that emits messages and tool results before the TUI
  subscribes still has a complete ordered transcript; concurrent arrivals
  produce no gaps or duplicate rows.
- [x] 2.2 Verify a running child, a completed child, and a durable child after
  parent restart are readable without a new provider call or child turn.
- [x] 2.3 Verify pagination and artifact retrieval expose complete large tool
  results and final replies while keeping each parent context insertion
  bounded. Verify redaction, authority checks, and unavailable history.
- [x] 2.4 Verify transcript reads do not consume completion delivery. A parked
  parent receives the final result through existing admission without polling;
  existing wait/result acknowledgement tests continue to pass.
- [x] 2.5 Verify the child screen uses the main transcript's message/tool
  rendering and visibility rules, preserves keyboard navigation and composer
  state, and reports observed activity truthfully with fake-clock coverage.
- [x] 2.6 Run formatting, affected crate checks/tests, and focused CLI/TUI
  integration coverage. Record a terminal walkthrough using a deterministic
  child that speaks, runs a tool, pauses, and completes.

## Validation Results

- Final pinned runtime: `bba8996c213f2f65287d84befc1c7dc6ced3241a`.
- Runtime library: 684 passed, 2 existing ignored tests.
- Delegation integration: 36 passed.
- Tool library: 187 passed.
- TUI library: 725 passed.
- CLI binary suite: 414 passed, 1 existing ignored test.
- Upstream runtime: 66 delegation and 64 external-agent integration tests
  passed, including protected observation recovery, native chronology, and
  failed acceptance with concurrent native output.
- Strict Clippy passed for all targets of `smith-runtime`, `smith-tui`,
  `smith-cli`, and `smith-tools`; formatting and whitespace checks passed.
- Terminal walkthrough recorded in
  `docs/qa/child-transcript-2026-10-23/walkthrough.txt`. It covers late
  subscription during a paused request, complete tool output, live final
  answer, and host restart without child execution.
- Twelve JSON/JSONL headless recordings were refreshed. Reviewed differences
  are only estimated prompt/preserved-prefix token counts after the additional
  delegation guidance; event content and answers are unchanged.
- Temporary local Cargo patches were removed. Checks use the immutable
  published Git dependency, not a sibling path.
- Release validation for v0.7.12: `scripts/ci.sh` passed on Rust 1.88.0
  (3,644 tests passed, 15 ignored), including strict workspace Clippy and the
  shared-runtime conformance suite. The minimal build, core-tool test, and
  isolated binary smoke passed. Dependency advisories, bans, licenses, and
  sources passed, together with npm bootstrapper and installer checks.
