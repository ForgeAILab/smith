---
created_at: 2026-11-09T21:37:36Z
updated_at: 2026-11-11T03:43:10Z
completed_at: 2026-11-11T03:43:10Z
---

## 1. Store

- [x] 1.1 Add `crates/modules/harness-feedback` with a store under
  `<user_dir>/feedback/`: owner-only directory and files, atomic writes,
  frontmatter + labeled-section Markdown, id generation, `viewed` flag.
- [x] 1.2 Hold an exclusive `fs2` lock on `feedback/.lock` while creating,
  raising a count, marking viewed and pruning.
- [x] 1.3 Enforce bounds: title, per-field and per-file limits with truncation
  markers; 500-file cap with pruning of old filed/dismissed reports; refusal
  when still full.
- [x] 1.4 Same-session duplicate handling (kind + area + normalized title)
  that raises `count` and appends bounded evidence; per-session limit of 5
  new reports.
- [x] 1.5 Read API: list (newest first, with status), show by id, set status,
  mark viewed; malformed files are reported by name and skipped.

## 2. Host facts

- [x] 2.1 Extend `ModuleContext` in `smith-module` with session identity
  (Smith version and commit, profile, provider, model), a redactor handle and
  a recent-failures source; supply them from `smith-runtime`.
- [x] 2.2 Back the redactor with the journal's registered-secret literal
  rules (`journal/redaction.rs`).
- [x] 2.3 Add a tool-failure recorder that wraps tool invocation, keeping the
  last 5 failed calls per session (name, call id, turn id, error ≤ 500
  chars), including bridged calls.

## 3. Tool and pinning

- [x] 3.1 `harness_feedback` tool: schema (kind, area, title, what_happened,
  user_said, repro), empty effects, short acknowledgement result, a visible
  tool row (`Harness Feedback · <title>`).
- [x] 3.2 Attach evidence from `InvocationContext` and the host handles;
  ignore any argument claiming those fields; for `cli/*` turns, state that the
  CLI's own tools are not covered.
- [x] 3.3 Add `harness_feedback` to `pinned_core` beside `view_image`, pinned
  only when registered, so root, child (including read-only) and headless
  turns have it without activation.
- [x] 3.4 Confirm the CLI bridge serves the tool (it is not excluded).
- [x] 3.5 Tool description and one prompt-guidance fragment: when to report,
  when not to, and never announce a report or ask about it.
- [x] 3.6 Register the module, default enabled, behind the existing
  `[modules.harness-feedback] enabled` switch; off removes the tool and the
  guidance.

## 4. Review surfaces

- [x] 4.1 `FeedbackReport` in `smith-client` (list, detail, empty, error
  states) with `render_plain`, following the existing `*_report.rs` pattern.
- [x] 4.2 `/feedback` and `/feedback <id>` as inline local results that never
  become model context; listing marks the shown reports as viewed.
- [x] 4.3 One-line session-start notice when unviewed queued reports exist,
  delivered through the transcript.
- [x] 4.4 `smith feedback list [--json]`, `show <id>`, `dismiss <id>`; JSON
  is bounded and labels report text as untrusted.

## 5. GitHub filing

- [x] 5.1 Inline select on `/feedback <id>`: file on GitHub, dismiss, keep.
- [x] 5.2 Build the public title and body (no workspace path or session ids),
  enforce the 8,000-byte URL bound with section-boundary truncation, and show
  the exact text before confirmation.
- [x] 5.3 Open the prefilled URL through `browser::open_browser`; show the URL
  as copyable text if the hand-off fails; mark `filed` only after a
  successful hand-off.

## 6. Tests and docs

- [x] 6.1 Store tests: permissions, atomic writes, lock contention between two
  writers, bounds, pruning, refusal, redaction, duplicates, per-session limit,
  malformed files.
- [x] 6.2 Pinning tests: the tool is pinned without activation in root, child,
  read-only child and headless turns; it is absent from the activatable
  index; it is in the CLI bridge tool set; it is absent when the module is
  off.
- [x] 6.3 Approval test: a call under `--approval deny` records without a
  prompt.
- [x] 6.4 Fake-provider end-to-end test (activate nothing; the tool is
  pinned): a scripted tool failure, then a `harness_feedback` call, produces a
  report with the failed call as evidence.
- [x] 6.5 TUI fixtures for `/feedback` list, detail, filing preview and
  notice; URL-building and truncation tests.
- [x] 6.6 Document the tool, `/feedback`, `smith feedback`, the store and the
  module switch in `docs/`.
- [x] 6.7 Full gate (`--no-fail-fast`, clippy, `--no-default-features` steps).
