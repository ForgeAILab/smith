---
created_at: 2026-08-03T20:21:57Z
updated_at: 2026-08-03T20:21:57Z
completed_at: null
---

## 1. Conditional instruction sections

- [x] 1.1 Split `DELEGATION` and `QUESTIONNAIRE` out of `stable_fragments()` in
      `crates/smith-runtime/src/prompt.rs` into capability-gated fragments,
      keeping their ids, revisions, and kinds. Repack the remaining
      unconditional sections into a contiguous `Stable` run at positions 0..n.
- [x] 1.2 Place every conditional section after that run, in the ephemeral
      block alongside `agent-profile`, so the leading stable run never varies.
- [x] 1.3 Split the `write_todos` sentence out of `WORKFLOW` into its own
      gated fragment in the same block; bump only the `workflow` revision.
- [x] 1.4 Extend `DynamicPromptContext` with the registered-capability flags
      and populate them in `factory.rs` from the same predicates that decide
      registration, so prose and tool surface cannot drift.
- [x] 1.5 Tests: child surface omits questionnaire; non-delegating profile
      omits delegation; fully capable run keeps authored order after the
      unconditional block; unconditional section revisions are unchanged.
- [x] 1.6 Test: no cache-stable fragment follows a non-stable one in canonical
      order, and the unconditional head is byte-identical across postures.

## 2. Todo posture gating

- [x] 2.1 Register `WriteTodosTool` only for non-read-only postures in
      `factory.rs::tools`.
- [x] 2.2 Skip the todo context contributor and its state projection when the
      tool is not registered.
- [x] 2.3 Tests: plan and review postures omit both tool and fragment; build
      posture keeps both.

## 3. Edit operations

- [x] 3.1 Add `operation` to the `edit` schema with `replace` default; map each
      operation to its permission set and effects, adding `Permission::FsDelete`.
- [x] 3.2 Implement `overwrite` via the existing temp-file-and-rename path and
      `delete` via a removal that records the pre-image.
- [x] 3.3 Keep empty `old_string` meaning `create`; keep `create`'s atomic
      `create_new` open.
- [x] 3.4 Extend `ChangeRecorder` attribution to the removal case and confirm
      undo restores a deleted file.
- [x] 3.5 Update the tool description to steer toward `replace` for partial
      changes, in the terse house style.
- [x] 3.6 Tests: each operation's permissions; create refuses an existing
      target; legacy empty `old_string` replays; delete is undoable; journal
      records no file body.

## 4. Read-before-overwrite

- [x] 4.1 Add `crates/smith-tools/src/read_state.rs` with a `ReadRecorder`
      keyed by canonical path, storing read time and whether the view was full.
- [x] 4.2 Record reads in the `ObservedTool` wrapper; thread the recorder
      through `observed_tools` alongside `ChangeRecorder`.
- [x] 4.3 Enforce the precondition in `edit::prepare` for `overwrite` and
      `delete`, with distinct messages for unread, partial, and stale.
- [x] 4.4 Tests: unread refused; partial refused with its own message; stale
      refused and the external change preserved; `replace` unaffected.

## 5. Cache-aware context triggers

> The trigger itself landed in `agent-runtime` under
> `trigger-summaries-on-context-pressure-2026-08-03`, because
> `SemanticSummaryPolicy` and the decision both live there. `TurnCommitView`
> already carried the usage ledger, so no pipeline API change was needed — only
> the policy fields and a budget the host supplies from its resolved model
> limits. The notice stayed in Smith: it is a context contribution, not a
> summarization concern.

- [x] 5.1 Replace `trigger_turns` with `min_turns` in `SemanticSummaryPolicy`
      and add `trigger_fraction`; validate bounds.
- [x] 5.2 Measure post-prefix usage against the resolved input budget rather
      than total usage, and trigger on the fraction.
- [x] 5.3 Add the one-shot appended budget notice with a per-window claim flag
      and `notice_threshold_tokens`.
- [x] 5.4 Tests: long-but-small session not summarized; short session with a
      large tool result summarized; turn floor respected; prefix size does not
      move the trigger; notice delivered once per window and re-armed after
      compaction; history preceding the notice is byte-identical.

## 6. Usage reporting and analytics

- [x] 6.1 Print per-counter session totals with provenance on TUI exit, reusing
      `status.rs` formatting.
- [x] 6.2 Append one bounded session usage record under user state, including
      compaction-window, notice, summary, and posture-switch tallies.
- [x] 6.3 Tests: estimated counts are not shown as reported; the record carries
      no conversation content; tallies reflect observed triggers.

## 7. Base harness budget test

- [x] 7.1 Add the ceiling constant and a test over `stable_fragments()` plus
      the default tool specs, printing a per-section breakdown on failure.

## 8. Validation

- [x] 8.1 `cargo fmt --check`, `cargo clippy` with warnings as errors, and the
      workspace test suite.
- [x] 8.2 Update `docs/` where the tool surface or context behavior is
      documented.
