---
created_at: 2026-07-27T04:52:54Z
updated_at: 2026-07-27T05:50:19Z
completed_at: 2026-07-27T05:50:19Z
---

## 1. Implementation

- [x] 1.1 Update `DESIGN.md` before code: remove region focus cycling and the
  focusable inbox; define composer focus, command completion, inline
  notifications, temporary diff/review/revert views, shortcuts, empty/error
  states, and confirmation behavior.
- [x] 1.2 Replace composer/transcript/inbox focus state with
  composer-plus-modal state. Keep transcript scroll/follow shortcuts global
  and ensure hidden or absent regions are never focus targets.
- [x] 1.3 Refactor slash input, `/help`, and `Ctrl+P` onto one typed command
  registry with filtering, descriptions, argument hints, selection,
  `Tab`/`Shift+Tab` completion, `Enter` execution, `Esc` dismissal, `//`
  passthrough, and local unknown/idle-state errors.
- [x] 1.4 Render child/monitor lifecycle as concise inline transcript notices
  and add `/agent` list/detail actions without a persistent inbox pane.
- [x] 1.5 Add `/status` using resolved runtime/session/change provenance
  without provider spend.

## 2. Git inspection and change attribution

- [x] 2.1 Add a Smith-owned Git inspection service that discovers repository
  state, staged/unstaged/untracked changes, commits, merge bases, bounded
  patches, path hashes, and structured non-Git/unavailable outcomes without
  shelling through user aliases.
- [x] 2.2 Define versioned `TurnChangeSet` journal records for exact edit
  patches, observed shell/extension deltas, attribution confidence,
  pre/post-image hashes, untracked-file recovery metadata, and undo/revert
  outcomes.
- [x] 2.3 Capture turn/tool mutation boundaries without exposing protected
  arguments or credentials, and mark ambiguous or externally overlapping
  deltas non-undoable rather than guessing ownership.
- [x] 2.4 Replay old and new journals compatibly; historical turns without
  change-set records remain resumable and visibly non-undoable.

## 3. Diff, review, undo, and revert

- [x] 3.1 Implement `/diff` with all-uncommitted, last-turn,
  staged/unstaged/untracked, file, and hunk views; include empty, non-Git,
  binary, oversized, and conflict states.
- [x] 3.2 Implement `/review` scope selection and a read-only reviewer
  child/session that reports prioritized file/line findings without mutation
  authority.
- [x] 3.3 Implement `/undo` for the newest fully attributable Smith turn with
  reverse-patch preview, explicit confirmation, exact post-image checks,
  atomic application, and structured refusal on ambiguity or overlap.
- [x] 3.4 Implement `/revert` file/hunk selection over the current diff with
  origin labels, preview, explicit confirmation, atomic bounded patches, and
  recoverable storage for removed untracked files. Do not implement
  `revert all`.
- [x] 3.5 Journal successful and failed recovery operations so a successful
  revert is itself recoverable during the session.

## 4. Verification

- [x] 4.1 Add command-registry tests proving `Tab` never changes regions,
  completion does not execute, `Ctrl+P` and `/` share one registry, local
  commands do not spend provider tokens, and busy/unknown commands fail
  locally.
- [x] 4.2 Add render tests for command filtering, narrow/short terminals,
  inline child/monitor notices, status, empty diff, binary/oversized diff,
  review findings, undo preview, revert selection, conflicts, and no-color
  operation.
- [x] 4.3 Add change-ledger and recovery tests covering pre-existing user
  changes, mixed staged/unstaged state, exact edit attribution, ambiguous
  shell writes, concurrent edits, renamed/deleted paths, unchanged and
  modified untracked files, partial hunks, repeated undo, journal replay, and
  crash-safe atomicity.
- [x] 4.4 Add end-to-end fake-provider tests for edit → `/diff` → `/undo`,
  selective `/revert`, read-only `/review`, child detail, session resume, and
  refusal outside Git.
- [x] 4.5 Run workspace fmt, Clippy with warnings denied, tests, MSRV/CI gates,
  and real cmux visual QA at narrow, normal, and wide terminal sizes.
