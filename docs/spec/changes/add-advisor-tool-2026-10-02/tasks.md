---
created_at: 2026-10-03T03:19:53Z
updated_at: 2026-10-03T05:23:35Z
completed_at: 2026-10-03T05:23:35Z
---

# Tasks: Add an advisor tool

## 1. Preconditions

- [x] 1.0 Owner approval (2026-10-02, approved before review; design
  choices delegated).
- [x] 1.1 Prove with a runtime-level test that a tool invoked mid-turn sees,
  through `SessionHandle::history()`, the current step's assistant message
  and every earlier tool result of that turn. Stop if it does not.
  Holds: `crates/smith-runtime/tests/advisor_history.rs`. Tools in the same
  parallel batch do not see each other's results.

## 2. Configuration

- [x] 2.1 Add the `advisor` placement to `use`, the top-level and per-profile
  `advisor` key (`"<name>"` or `false`), inheritance through `extends`, and
  validation: the target exists and includes `advisor` in `use`; a profile
  never resolves itself (a top-level default naming it is skipped; an
  explicit self-reference is rejected).
- [x] 2.2 Report the winner and overridden sources in
  `smith config explain advisor`.
- [x] 2.3 Document the keys, the disclosure note, and an example in
  `docs/configuration.md`.

## 3. Runtime

- [x] 3.1 Build the advisor route through the child-route `prepare` path.
- [x] 3.2 Add the `advisor` tool behind `advisor_eligible`, reaching the
  session through a slot filled after start.
- [x] 3.3 Render the transcript and trim it to the advisor's input budget.
- [x] 3.4 Send the advisor request (built-in prompt plus profile
  instructions, no tools) and return the advice or a tool error.
- [x] 3.5 Contribute the guidance section only when the tool is registered.
- [x] 3.6 Record advisor usage and cost under an advisor attribution.
  Recorded as `UsageRecord` purpose `advisor` at the advisor's catalog rates;
  the pinned runtime has no advisor `UsageSource`, so records carry the
  compatibility source `SemanticSummary`. A native source is an upstream
  follow-up.
- [x] 3.7 Give the tool a display label and result preview.

## 4. Verification

- [x] 4.1 Tests: configuration validation and explain; tool registration
  only on root surfaces with an advisor; transcript rendering, image
  omission, and trimming; error results; usage attribution; prompt section
  present only with the tool.
- [x] 4.2 `cargo fmt --all -- --check`, strict Clippy, workspace tests, and
  fixture compare unchanged.
- [x] 4.3 Live: a project-local configuration that makes `sol` the advisor
  for `code`; ask for a plan, confirm one advisor call, its advice in the
  transcript, and its cost in `/status`.
