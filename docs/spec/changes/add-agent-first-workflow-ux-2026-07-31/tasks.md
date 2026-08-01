---
created_at: 2026-07-31T22:06:35Z
updated_at: 2026-08-01T00:44:17Z
completed_at: 2026-08-01T00:44:17Z
---

## 0. Approval and baselines

- [x] 0.1 Approve this proposal, design, and all six capability deltas.
- [x] 0.2 Capture existing reducer/replay, headless schema-v2, activation,
  checkpoint-key-unavailable, and PTY golden fixtures before behavior changes.
- [x] 0.3 Add a disposable stable-ready-queue benchmark harness that never
  commits generated project state and records product/version/model/reviewer
  provenance.

## 1. Configuration and policy foundations

- [x] 1.1 Add typed `build`, `plan`, and `review` root agent modes plus bounded
  read-only `explore` and `review` child presets; prove they only narrow the
  authoritative run policy.
- [x] 1.2 Add layered, source-explainable per-request output budgets and set the
  cataloged Z.AI Coding Plan `glm-5.2` default to 32,768 within its declared
  model limit.
- [x] 1.3 Add environment and explicit owner-only inline checkpoint-key sources,
  mutual-exclusion/project-scope validation, redaction, zeroization, and a
  `smith setup checkpoint-key` atomic transaction.
- [x] 1.4 Prove an inline/environment checkpoint key causes zero Keychain or
  Secret Service calls and preserves authenticated-encrypted checkpoints.

## 2. Harness terminal correctness and activation

- [x] 2.1 Tune coding-intent retrieval so inspection activates exact read
  abilities, modification prefers `edit`, broad `shell` requires command/build
  intent, and multi-step/review intent activates todos/delegation.
- [x] 2.2 Reconcile all todo items to terminal statuses at every turn terminal
  boundary, with unfinished items cancelled rather than guessed complete.
- [x] 2.3 Make retry/limit terminals discard uncommitted visible reasoning/text,
  emit concise structured evidence, and keep live/replay/headless results
  equivalent.
- [x] 2.4 Add scenario tests reproducing the original 8K limit failure,
  broad-shell activation miss, and stale `in_progress` result.

## 3. Prepared composer actions

- [x] 3.1 Add one bounded `@` resource index/completion parser for canonical
  workspace files and registered child presets, including literal escaping.
- [x] 3.2 Resolve file attachments through exact prepared read authorization
  into provenance-bearing bounded context fragments or artifact references.
- [x] 3.3 Resolve explicit `@agent` requests through the existing depth-one
  coordinator with visible inherited model, limits, read-only policy, and spend
  confirmation.
- [x] 3.4 Route leading-`!` commands through the canonical prepared ShellTool
  executor/approval path and add literal `!!` passthrough.
- [x] 3.5 Verify cancellation, deadlines, approvals, output offloading,
  checkpoint recovery, redaction, and no-provider-spend behavior for every
  prepared composer action.

## 4. TUI presentation and navigation

- [x] 4.1 Render idle agent/mode, provider/model, project/branch, context, and
  prioritized shortcut hints without a permanent header; implement empty-idle
  `Tab` mode cycling.
- [x] 4.2 Render `@` completion with labelled file/agent entries and preserve
  current command/questionnaire/approval keyboard contracts.
- [x] 4.3 Add one replaceable replay-equivalent work summary for plan, active
  tools, gates, retries, changed paths, and children plus a bounded `/details`
  toggle.
- [x] 4.4 Add `/timeline` and temporary previous/next/parent child inspection
  while keeping the root composer as the only persistent focus.
- [x] 4.5 Add `/redo` preview/confirmation over exact recovery records and
  fail closed for stale or ambiguous changes.
- [x] 4.6 Add snapshot and PTY coverage at 44x14, 74x24, and 120x32, with
  colorless/reduced-motion/accessibility assertions.

## 5. Persistence, cleanliness, and compatibility

- [x] 5.1 Resume prepared file/shell actions, plan terminal state, timeline, and
  child interruption exactly once under the selected checkpoint key.
- [x] 5.2 Add compatible migration behavior for existing keyring checkpoints
  and explicit atomic rotation/refusal behavior when changing key sources.
- [x] 5.3 Prove interactive/headless runs create no `.smith`, `.omo`, session,
  timeline, or child-control metadata in the project checkout.
- [x] 5.4 Preserve schema-v2 machine output or version any unavoidable additive
  fields with compatibility fixtures and redaction tests.

## 6. Product validation and documentation

- [x] 6.1 Run fmt, warning-denied Clippy, workspace tests, Rust 1.88, diff
  hygiene, strict spec validation, cargo-deny, and cargo-audit.
- [x] 6.2 Run the disposable coding benchmark on Z.AI Coding Plan GLM-5.2 with
  todos, exact read/edit/shell, validation gates, same-model child review, and
  all terminal plan items; independently rerun public and adversarial tests.
- [x] 6.3 Run a no-Keychain durable live scenario that interrupts after a
  checkpoint boundary, resumes without repeating provider/tool work, and
  verifies no credential-service access occurred.
- [x] 6.4 Update `DESIGN.md`, configuration/setup reference, security threat
  model, persistence/recovery docs, command help, and benchmark notes.
- [x] 6.5 Reinstall the verified `smith` binary and record exact local evidence;
  leave hosted macOS/Linux release gates explicit if unavailable.
