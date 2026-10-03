---
created_at: 2026-10-03T22:44:48Z
updated_at: 2026-10-03T22:44:48Z
---

## Why

Changing model, profile, effort, thinking, or context window, pressing Tab
to cycle profiles, connecting a provider, or an MCP server finishing its
connection ends the TUI. Smith leaves the alternate screen, rebuilds the
host, and starts a fresh application. Local results, notices, folded and
expanded state, scroll position, and composer history are lost, and the
transcript is rebuilt from history alone. This is finding U3 and roadmap
step 4 in the [2026-10-02 audit](../../../qa/smith-structure-2026-10-02/report.md),
and the most visible UX defect left after v0.3.1.

## What Changes

- The terminal is entered once per Smith run, not once per host.
- A rebuild that keeps the same session keeps the application. That covers
  selection changes, Tab profile cycling, MCP recomposition, and connect or
  disconnect of a provider other than the active one. The transcript, folded
  state, scroll position, composer, and composer history stay. Status,
  resources, children, and usage are re-derived from the new host, and the
  live turn resets to idle (`refactor-tui-state`).
- The transcript is not rebuilt from history on such a rebuild. One notice
  says what changed.
- Switching to another session (`/new`, `/resume <other>`) replaces the
  transcript with that session's history and keeps composer history and the
  screen.
- Connect and guided flows that draw their own screen suspend the
  application's screen and return to it unchanged.
- A rebuild is refused while a turn is running, as a defence behind the
  existing requires-idle rule.

## Impact

- Affected specs: client-surfaces.
- Affected code: `smith-cli` `runtime_host.rs` (`run_interactive_command`),
  `tui_driver.rs` (`run_interactive`, `run_tui`), `terminal.rs` (suspend and
  resume); `smith-tui` an app rebind entry point.
- Depends on `refactor-tui-state` (single live-turn reset, typed child state).
- No change to provider requests, persistence, configuration, or machine
  output.
