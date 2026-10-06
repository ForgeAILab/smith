---
created_at: 2026-10-06T21:33:26Z
updated_at: 2026-10-06T22:06:36Z
---

## Why

The advisor can only be chosen in configuration (`advisor = "sol"` or
`"provider/model"`), so `/` lists nothing for it and a running session cannot
turn it on, turn it off, or point it at another reviewer. The owner wants to
switch it at any time from the TUI.

## What Changes

- Add `/advisor` to the command registry, so it appears in `/` discovery and
  `/help`.
- `/advisor` with no argument opens the shared inline picker: `off`, `default`
  (what configuration resolves), and each configured profile and model that
  can serve as an advisor. The current choice is preselected.
- `/advisor off`, `/advisor on`, `/advisor default`, `/advisor <profile>`,
  and `/advisor <provider>/<model>` select directly. `on` restores the
  configured advisor and is refused with guidance when none is configured.
- The choice is a session override, like `/think` and `/effort`: it requires
  an idle turn, applies through the existing safe-boundary reconfigure, keeps
  the session and its history, and does not write `config.toml`.
- A target that does not resolve (unknown profile, unconfigured provider,
  model without limits) is refused before the session is rebuilt, and the
  previous advisor stays in effect.
- A notice confirms the result, and `/status` shows the effective advisor and
  whether it comes from configuration or the session override.

## Impact

- Affected specs: `advisor`, `client-surfaces`
- Affected code: `crates/smith-client/src/commands.rs` (registry,
  `SelectionCommand::Advisor`), `crates/smith-tui/src/app/resources.rs`
  (picker and dispatch), `crates/smith-cli/src/cli.rs` (`Selection` override),
  `crates/smith-cli/src/runtime_host.rs` (apply the override where the advisor
  profile is resolved, reconfigure routing), `crates/smith-client/src/status.rs`
  (status line).
- Not in scope: persisting the choice to configuration, a headless flag, and
  changing what the advisor sees or how its usage is accounted.

## Decisions

- The override does not survive a restart, `/new`, or `/resume`: it resets to
  configuration, so a resumed session never silently bills a reviewer the
  configuration does not name. Approved as proposed on 2026-10-06.
- If a rebuilt session cannot start with the new advisor, the previous one is
  restored and the reason is shown, instead of ending the TUI.
