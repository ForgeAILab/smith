---
created_at: 2026-09-23T00:00:00-04:00
updated_at: 2026-09-23T00:00:00-04:00
---

# Proposal: Show the idle-compaction summary

## Why

Idle compaction is durable but invisible. When the one-hour inactivity limit
fires, Smith replaces eligible old history with a semantic summary and stores
its body in a protected session-owned artifact with full provenance
(purpose, provider/model, revision, source coverage). The only user-visible
trace is one scrolling counts-only transcript notice (`compacted context ·
reclaimed N tokens`) emitted while the user is away, and nothing at all is
shown when the session is resumed later. Users who return to a compacted
session therefore cannot see what the summary says or know that their next
prompt starts from it. The summary also cannot be inspected on demand: no
interactive command renders it, and machine output intentionally excludes
summary text.

## What Changes

- Render a distinct idle-compaction transcript notice when automatic idle
  compaction completes, naming what happened and where the summary lives,
  instead of relying on the generic per-event counts notice alone.
- On resume of a session whose last idle interval completed compaction, show
  one bounded on-return presentation so a returning user sees the compacted
  state without digging through history.
- Add an interactive `/summary` command that renders the current durable
  semantic summary body from its protected artifact with its provenance
  (purpose, provider/model, revision, source coverage, created_at), bounded in
  size, and clearly labeled as non-authoritative presentation: semantic text
  never overrides exact state.
- Make explicit that the next prompt continues from the summary: the
  on-return presentation states that continuing starts from the summarized
  context, and `/summary` shows exactly what that is.
- Headless text mode writes one bounded stderr pointer (session and artifact
  identity, no body) after a completed turn that included idle compaction,
  consistent with existing cache-miss notices; JSON and stream-JSON gain only
  bounded redaction-safe metadata (`idle_compaction_completed`,
  `summary_available`), never summary text.
- A missing, corrupt, or incomplete summary renders a bounded failure notice
  and never falls back to untrusted prose.

Out of scope: changing when idle compaction runs or its budgets, exposing
summary bodies in machine output or `/status`, forking a new conversation
seeded from the summary (see the draft `fork-conversation-branches` change),
handoff-checkpoint summaries, and any provider request changes.

## Impact

- Affected specs: `client-surfaces`, `session-recovery`
- Affected Smith code: `crates/smith-tui/src/app/{reducer,conversation}.rs`, `crates/smith-client/src/commands.rs` (`/summary`),
  and transcript rendering; `crates/smith-runtime/src/cache_controller/`
  (completed-compaction projection), `crates/smith-runtime/src/resume_capsule.rs`
  (bounded protected-artifact view already exists); `crates/smith-cli/src/headless/`
  stderr pointer and bounded JSON metadata; focused tests in each crate
- No configuration, credential, authority, provider, or persistence-layout
  changes; no new events beyond Smith's existing consumer projections

## Approval Boundary

Approval authorizes the visible idle-compaction notice, the bounded on-return
presentation, the interactive `/summary` view of the protected summary body
with provenance, the headless stderr pointer plus bounded machine metadata,
and focused tests and documentation. It does not authorize changing idle
compaction timing or budgets, emitting summary text into machine output or
status projections, conversation forking, or any provider request change.
