# Proposal: Recover Smith's own edits in mixed turns, and show cache misses by default

## Why

Two local-surface behaviors read to the user as Smith being noisier and less
capable than it is.

**Undo goes away for a whole turn because of one shell command.** A turn that
edits files with Smith's `edit` tool and also runs `cargo fmt` records one
`ToolMutation::Exact` per edited file plus one `ToolMutation::Ambiguous` for
the shell call (`crates/smith-tools/src/change.rs`). `is_fully_attributable()`
requires *every* mutation to be exact, so the mixed turn fails the gate and
`/undo` refuses entirely — even though the exact pre/post images for Smith's
own edits are sitting in memory, individually hash-checked and reversible. The
transcript then reports `contains ambiguous changes; use /diff`, which is
accurate about the cause and silent about the fact that the recoverable half
exists. Shell use is routine, so in practice undo is unavailable most of the
time.

The original gate was conservative for a real reason: reversing part of a turn
can leave the tree in a state neither the user nor Smith authored. That risk is
already covered by a stronger mechanism — the per-path post-image check. A path
a shell command rewrote after Smith's edit no longer hashes to the recorded
post-image, so it is refused. What the all-or-nothing gate adds on top of that
is not safety, only lost recoverability.

**Cache-miss notices are off by default.** `cache.miss_notices` defaults to
`false`, so the one notice that reports money already spent — a completed turn
that missed ≥20,000 expected cache-read tokens or ≥$0.10 — stays hidden unless
the user knew to enable it. It is presentation-only and already threshold-gated
to significant misses.

## What Changes

- `TurnChangeSet` gains `exact_mutations()`, `has_exact_mutations()`, and
  `ambiguous_tools()`. `is_fully_attributable()` keeps its current meaning and
  stays the label for "this turn is exact end to end".
- `/undo` eligibility moves from "fully attributable" to "has at least one
  exact mutation, and is not already undone". The reverse patch covers exactly
  those mutations; the per-path post-image check is unchanged and still
  refuses the whole operation without touching any path. `/redo` follows the
  same rule so a partial undo is reversible.
- The undo and redo previews name the tools whose delta was not attributable
  and state that those changes are left untouched, so a partial undo is never
  presented as a whole-turn reversal. Preview text is rendered once and shared
  by the preview and cancellation paths, keeping their journaled fingerprints
  identical.
- A turn with no exact mutation at all still has nothing to undo and refuses,
  pointing at `/diff` and `/revert`.
- The transcript notice and `/status` keep the "contains ambiguous changes"
  phrasing and add what is actually available: `/undo` covers Smith's own
  edits, `/diff` shows the rest. The change timeline labels such a turn
  `mixed` rather than `ambiguous`.
- `cache.miss_notices` defaults to `true`. It remains a layered, explainable,
  presentation-only Boolean that any layer can set back to `false`.

Out of scope: skipping individual conflicted paths instead of refusing the
operation; attributing shell deltas file by file; reverting ambiguous changes
automatically; every other transcript notice source (capabilities activation,
speculative-attempt discards, downgrade and limit wording), which stays as it
is in this change.

## Impact

- Affected specs: `change-review` (undo requirement and scenarios),
  `configuration` and `prompt-cache` (one default flip)
- Affected code: `crates/smith-tools/src/change.rs`,
  `crates/smith-cli/src/tui_driver.rs`, `crates/smith-cli/src/local_command.rs`,
  `crates/smith-config/src/resolve/load.rs`
- No wire-protocol, persistence-format, credential, approval, or authority
  change. The change journal keeps its existing `fully_attributable` field and
  schema version; no new dependencies.
- Safety: recovery remains fail-closed per path. A mixed turn whose exact path
  was overwritten after the fact is refused exactly as before, and no path
  outside the recorded exact patches is ever written.

## Approval Boundary

Approval authorizes the eligibility change from "fully attributable turn" to
"exact mutations within a turn", the preview disclosure of unattributable
tools, the matching notice/status/timeline wording, and the
`cache.miss_notices` default flip. It does not authorize partial application
across conflicted paths, any reconstruction of shell deltas, or changes to any
other notice source.
