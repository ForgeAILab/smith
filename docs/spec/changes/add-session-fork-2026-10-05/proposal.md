---
created_at: 2026-10-05T20:06:57Z
updated_at: 2026-10-05T20:06:57Z
---

## Why

The user wants to duplicate the current chat and continue the copy in a
different direction, keeping the original intact. Smith can only resume a
session or start a new one; there is no way to branch from where a
conversation is now.

This replaces `fork-conversation-branches-2026-08-07` (deleted with this
proposal; it remains in git history), which proposed a
different feature: the model spawning a branch of itself that runs in
parallel, which needs an Agent Runtime change. The user-facing duplicate is a
Smith-only feature.

## What Changes

- Add `/fork`. When the session is idle, Smith writes a new session whose
  history is a copy of the current one, switches the TUI to it, and reports
  the new and original session ids. The original stays in `/resume`.
- Add `--fork-session` for `--resume [<id>]`: start from a forked copy of that
  session instead of continuing it, as in Claude Code. Works for the TUI and
  `smith -p`.
- What the fork copies: canonical history, the resume capsule and other
  history-derived extension state, the reasoning override, and the
  display-only shell sidecar so the transcript reads the same.
- What the fork starts fresh: session id, event journal, usage ledger (cost
  belongs to the session that spent it), protected checkpoint, background task
  spool, approval grants scoped to the parent session, and the change
  attribution journal (`<id>.changes.jsonl`), so `/undo` in one session cannot
  revert edits made in the other.
- Record lineage beside the snapshot: the parent session id and the parent's
  turn count at the fork point. `/resume` and the TUI header show "forked from
  <id>".
- Artifacts: the copied history refers to artifacts the parent owns (offloaded
  tool output, the idle-compaction summary), and every artifact read checks
  the owner. A forked session MAY also read artifacts owned by the sessions it
  was forked from, checked in `SmithArtifactStore::read` against the recorded
  lineage. History stays byte-identical, so the fork's first request reuses
  the provider's prompt cache. Smith never deletes artifacts with a session
  today, so the parent being deleted does not orphan them.
- Refuse `/fork`, with the reason, while a turn is running, a child agent is
  running, or a child result is waiting to be delivered; and on a session
  that runs on an installed coding agent (`cli/...`), because the CLI's own
  conversation id would then be continued by two sessions.

## Out of Scope

- Forking from an earlier point in the conversation (rewind and branch).
- Model-initiated parallel branches (the superseded proposal).
- Forking an installed-agent session; it needs the CLI's own fork support.
- Merging a fork back into its parent.

## Impact

- Affected specs: `session-recovery`, `client-surfaces`.
- Affected code: `crates/smith-runtime/src/session.rs` (fork write, lineage
  file), `crates/smith-runtime/src/artifact.rs` (lineage-aware read),
  `crates/smith-runtime/src/host*` (fork entry point and refusals),
  `crates/smith-client/src/commands.rs` (`/fork`), `crates/smith-cli`
  (`--fork-session`, session switch), `crates/smith-tui` (header, `/resume`
  row).
- Compatibility: additive. Sessions without a lineage file behave as today.
  No runtime change and no pin bump.
- Security: a fork can read only artifacts of its own ancestors, never a
  sibling's or an unrelated session's. Approval grants never transfer.

## Approval Boundary

Approval covers `/fork`, `--fork-session`, the copy/fresh split above, the
lineage file, ancestor-only artifact reads, and the refusals. It does not
cover rewinding to an earlier point, model-initiated branches, installed-agent
forks, or merging.
