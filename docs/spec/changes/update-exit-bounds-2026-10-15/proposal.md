---
created_at: 2026-10-15T09:41:45Z
updated_at: 2026-10-15T09:41:45Z
---

## Why

Quitting Smith is not bounded as a whole. Each shutdown step has its own wait
(turn drain, cache drain, children, background tasks, journal, MCP servers),
so one exit can take well over ten seconds, and `/quit` typed during a turn
stops at a confirmation that disappears if the turn ends first. A
Smith-started stdio MCP server that is busy when Smith is SIGKILLed keeps
running until its call finishes and it reads the closed input.

The owner asked for all three on 2026-10-15: "/quit should quit right away
without confirmation as user typed", "we should have some hard limit on the
timeout as well" (answered: total exit time, and MCP servers after SIGKILL).
That request is the approval for this change.

## What Changes

- **BREAKING (TUI behavior):** `/quit` exits at once, even during a turn,
  with a pending prompt, or with children or background tasks running. The
  exit confirmation is removed. The exit output names the work that was
  stopped.
- From the moment exit begins, Smith exits within 10 seconds. Whatever has
  not stopped by then is killed and the exit reports the deadline.
- Smith-started stdio MCP servers run under a small Smith watchdog process.
  When Smith dies, by any signal including SIGKILL, the watchdog stops the
  server within a second.
- Agent Runtime: the conformance test
  `session::one_shutdown_deadline_bounds_all_active_turns` (issue #4) runs on
  a paused clock and asserts exactly one deadline elapsed. Smith's pin moves
  to the commit that carries it.

## Impact

- Affected specs: `agent-session`, `client-surfaces`, `extension-system`
- Affected code: `crates/smith-tui/src/app/input.rs` (exit request),
  `crates/smith-cli/src/tui_driver.rs`, `crates/smith-cli/src/headless/`,
  `crates/smith-cli/src/main.rs`, `crates/smith-cli/src/cli.rs`, a new
  watchdog subcommand beside `harness_supervise.rs`,
  `crates/smith-runtime/src/mcp/stdio.rs`, `Cargo.toml` (runtime pin)
- Docs: `DESIGN.md` (exit confirmation, `/quit`), `docs/headless-protocol.md`,
  `docs/extensions.md`
