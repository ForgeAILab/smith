---
created_at: 2026-10-03T22:44:48Z
updated_at: 2026-10-03T22:44:48Z
completed_at:
---

Approved 2026-10-03 ("yes, lets do those please", after the roadmap
summary named this step and `update-reconfigure-keeps-screen`).

## 1. Typed child state

- [ ] 1.1 Add a typed child state in `smith-tui` covering every state a
  child row can hold today: running, resuming, needs input, completed, idle,
  interrupted (with whether an exact checkpoint exists), stopped (with its
  reason in words), failed, expired, blocked, terminal. Labels for the states
  `smith_client::agent_report::ChildState` also names come from that type, so
  the two cannot drift.
- [ ] 1.2 `ChildSummary.state` holds the typed value. Replace every string
  comparison: liveness, retirement, tone, the composer activity text, the
  parked-activity check, and `/agent resume`'s resumability (which must stop
  reading `detail`).
- [ ] 1.3 `App::restore_child` takes the typed state. `smith-cli`'s
  `child_summary_projection` passes it typed instead of a label; the detail
  keeps its other facts.
- [ ] 1.4 Regression test for v0.3.1: a child restored the way the host
  restores it, and a child interrupted through a live event, both open the
  resume confirmation; a non-resumable one reports incompatible. Existing
  tests that seed literal states use the typed value.
- [ ] 1.5 Every child label renders exactly as before (fixtures unchanged).

## 2. One live-turn value

- [ ] 2.1 Move the fields that describe the live turn (`active_turn`, `work`,
  `provider_phase`, `provider_retry`, `speculative`, `turn_started_at`,
  `turn_started_timestamp`, `turn_usage`, `local_shell_turn`) into one value
  with a single reset. Fields that outlive the turn (the attached turn
  summary and its revisions, stream gap tracking, cache-notice dedupe) stay
  outside it.
- [ ] 2.2 Every turn boundary in the reducer that clears live-turn state
  calls the single reset; no field is cleared by hand elsewhere.
- [ ] 2.3 Expose the reset to the host (`App::reset_live_turn` or similar)
  for `update-reconfigure-keeps-screen`.
- [ ] 2.4 Behaviour is unchanged: existing turn, retry, interrupt, and
  shortcut tests pass without edits to their assertions.

## 3. Verification

- [ ] 3.1 `cargo fmt --all -- --check`, strict Clippy, workspace tests.
- [ ] 3.2 Local-command fixtures unchanged.
