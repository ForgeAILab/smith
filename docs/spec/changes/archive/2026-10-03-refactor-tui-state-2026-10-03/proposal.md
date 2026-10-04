---
created_at: 2026-10-03T22:44:48Z
updated_at: 2026-10-03T23:01:32Z
---

## Why

A child agent's state in the TUI is a `String` matched against literals in 22
places, and whether a child can be resumed is decided by reading its detail
prose. When `adopt-claude-code-grammar` changed the interrupted label to
`interrupted (resumable)`, `/agent resume` kept comparing against
`"interrupted"`, so v0.3.1 reports every interrupted child as incompatible.
The test passed because it seeds the old string.

The live turn is also spread over about a dozen `App` fields reset by hand at
several sites. Keeping the app across a reconfigure
(`update-reconfigure-keeps-screen`) needs one call that puts that state back
to idle.

Roadmap step 3 in the [2026-10-02 audit](../../../qa/smith-structure-2026-10-02/report.md)
(S2, S6). This change takes only the two parts above.

## What Changes

- A typed child state in `smith-tui` replaces `ChildSummary.state: String`.
  Labels, tones, liveness, retirement, and resumability are methods on it.
  The host hands restored children over as typed state, not a label.
- `/agent resume` decides resumability from the typed state, which fixes the
  v0.3.1 regression.
- The fields that describe the live turn move into one value with one reset.
  The reducer's turn boundaries and the coming rebind call that reset.
- No visible wording changes. Every label stays as it reads today.

Out of scope, kept for a later change: typed notices with explicit
persistence (S2), one confirm component and an overlay queue (S5), and a
keymap table that drives dispatch (S5).

## Impact

- Affected specs: client-interaction.
- Affected code: `smith-tui` `app/state.rs`, `app/reducer.rs`,
  `app/resources.rs`, `render/helpers.rs`, `render/composer.rs`, child and
  turn tests; `smith-cli` `submission.rs` (`child_summary_projection`) and the
  `restore_child` callers in `tui_driver.rs`.
- No change to provider requests, persistence, machine output, or wording.
