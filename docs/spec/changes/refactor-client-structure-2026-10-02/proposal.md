---
created_at: 2026-10-02T10:35:00Z
updated_at: 2026-10-03T04:58:44Z
---

## Why

The layer between the runtime and the screen has no ownership rule. Session
accounting lives in the terminal crate and is imported by headless; command
wording lives in the CLI crate and is parsed back by the renderer; a slash
command is defined in four lists across two crates; provider data is retyped
in three crates and has drifted. Each user-visible defect in the 2026-10-02
audit traces to one of these.

Evidence: [audit](../../../qa/smith-structure-2026-10-02/report.md)
(findings S1, S2, S3, S4, S8).

## What Changes

- Add one client-neutral crate (`smith-client`) with no
  terminal dependency. Move into it the cache projection, usage, pricing,
  cost, and usage log from `smith-tui`. Headless and exit reporting import
  them from there.
- Send local command results from host to client as typed reports
  (`StatusReport`, `ContextReport`, and so on). The terminal renderer draws
  the report; a plain-text renderer in the new crate serves headless. The
  renderer stops recovering structure from titles, headings, and labels.
- Define each slash command once. One table supplies name, summary, argument
  grammar, group, and route; completion, `Ctrl+P`, `/help`, parsing, and
  dispatch read it. Commands are split by route in the type, so the host
  matches only commands it can execute and the cross-crate `unreachable!`
  arms go away.
- Make `smith-config` provider descriptors the only source of setup entries,
  endpoints, limits, and the connectable-provider list. The CLI passes them
  to the setup surface as data; each entry carries its flow.
- Give each state shown on more than one surface one label function.
- Replace `use super::*` in `smith-cli` production modules with explicit
  imports, and split `handle_local_command` into one function per command.

Behaviour is preserved: transcript text, headless output, exit codes, and
serialized fields are compared against fixtures recorded before the change.

Not in this change: `App` state, overlays, key handling, reconfigure,
rendering performance, the client protocol, or any wording improvement. The
audit roadmap lists the changes that follow.

## Impact

- Affected specs: code-organization.
- Affected code: new crate; `smith-tui` `cache.rs`, `status.rs`,
  `usage_log.rs`, `commands.rs`, `setup.rs`, `app/resources.rs`, and the
  local-result branch of `render/transcript.rs`; `smith-cli`
  `local_command.rs`, `headless/`, `runtime_host.rs`, `resources.rs`,
  `connection.rs`, `setup.rs`, `main.rs`; `smith-config` `setup.rs`.
- Compatibility: workspace crates are not published, so moved paths need no
  deprecation period. No user-visible change is intended.
- Ordering: `render/transcript.rs` and `app/state.rs` carry uncommitted work
  from `show-provider-retry-progress` and `show-idle-compaction-summary`.
  Stages that touch them start after those are committed.
- Depends on nothing; `fix-interaction-defects` may land before or after.
  Its setup-entry test becomes structural once descriptors carry flows.
