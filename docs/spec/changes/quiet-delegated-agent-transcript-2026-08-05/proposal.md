---
created_at: 2026-08-05T00:00:00Z
updated_at: 2026-08-05T00:00:00Z
---

## Why

Every child lifecycle event currently prints an attributed notice into the root
transcript: `started`, `is working`, `ran Grep`, `ran Read`, `recovered`, and
then the terminal one. A single delegated agent therefore writes more rows than
the turn that spawned it, and a handful of them bury the conversation the user
is actually reading. The delegated-work panel above the composer already shows
each child's identity, latest activity, and live clock, so the transcript rows
duplicate ambient state that has a better home.

The panel is also the only place that state exists — and it is not reachable
from the keyboard. `App::inspected_child` has existed since child navigation
landed, but nothing sets it except `/agent <id>`, so the panel is a display
with no selection, and a user watching a child work has no way to see what it
has been doing beyond one clipped row.

## What Changes

- Keep only delegation's boundaries in the root transcript: spawned, resume
  started, needs input, interrupted, completed, stopped, failed. Drop
  `is working`, `ran <tool>`, and durable `recovered` from it.
- Retain every lifecycle event — printed or not — in a bounded per-child log so
  nothing is lost, only relocated.
- Make `Down` past the newest composer draft walk the delegated-work panel and
  `Up` walk back to the root timeline, in the panel's own live-first order.
- Replace the transcript region with the selected child's read-only view —
  identity heading, coordinator card, and log — until `Esc`. The composer keeps
  focus throughout.
- While the inspector is open, an ordinary submission is a follow-up to that
  child through the existing confirmation; `/commands` and `!` shell shortcuts
  still address the root.
- Refresh the inspector's turn/token/session/workspace card from the delegation
  coordinator on the host's existing poll-on-redraw, so an arrow-selected child
  reports exactly what `/agent <id>` reports.

## Impact

- Affected specs: `child-agents`, `client-surfaces`
- Affected code: `crates/smith-tui` reducer, state, input, and render;
  `crates/smith-cli` `/agent` handling and the interactive redraw poll;
  `DESIGN.md` sections 4 and 8
- Compatibility: no runtime, journal, or event-schema change. The same events
  are folded; only which surface renders them moves. `/agent`, `@child-id`
  follow-up, and `/agent resume` keep their meaning.
- Focus: unchanged. The inspector is a read-only region, never a focus target,
  per the existing single-focus requirement.

## Approval Boundary

Approval authorizes moving mid-flight child progress out of the root transcript
into the panel and a bounded per-child log, and adding arrow-key selection with
a transcript-region inspector. It does not authorize suppressing any terminal
child event, dropping progress from the canonical journal or the parent's
safe-boundary inbox, giving the inspector its own focus, or letting the client
compute child turn/token figures the coordinator owns.
