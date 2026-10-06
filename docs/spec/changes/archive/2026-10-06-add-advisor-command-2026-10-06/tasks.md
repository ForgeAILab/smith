---
created_at: 2026-10-06T21:33:26Z
updated_at: 2026-10-06T22:06:36Z
completed_at: 2026-10-06T22:06:36Z
---

## 1. Command and selection

- [x] 1.1 Add `advisor` to the command registry with `[on|off|default|TARGET]`
  grammar, idle requirement, and parse tests.
- [x] 1.2 Add `SelectionCommand::Advisor` (off, default, target) and a matching
  session override on `Selection`.
- [x] 1.3 Route it through the existing safe-boundary reconfigure, keeping the
  session identity and frozen catalog.

## 2. Resolution

- [x] 2.1 Apply the override where the advisor profile is resolved: off
  registers no tool, default uses configuration, a target replaces it.
- [x] 2.2 Validate a target before the host is rebuilt; on failure keep the
  previous advisor and report the reason.
- [x] 2.3 Refuse `on` when configuration names no advisor.

## 3. Surfaces

- [x] 3.1 `/advisor` picker with off, default, profiles, and models; current
  choice preselected.
- [x] 3.2 Confirmation notice after each change; refusal notice when busy.
- [x] 3.3 `/status` shows the effective advisor and its source.

## 4. Verification

- [x] 4.1 Reducer and render tests for picker, direct forms, and refusals.
- [x] 4.2 Host-routing tests: off removes the tool and guidance, a target
  registers it, a bad target leaves the session unchanged.
- [x] 4.3 Live tmux walkthrough against the real config: off, on, switch,
  then an advisor call.
