---
created_at: 2026-10-15T09:41:45Z
updated_at: 2026-10-15T09:41:45Z
completed_at:
---

## 1. `/quit` without confirmation

- [ ] 1.1 `/quit` exits at once with any live work; pending approvals and
  questionnaires are cancelled; the exit confirmation is removed. TUI unit
  tests that asserted the confirmation now assert the immediate exit.
- [ ] 1.2 The exit summary prints one `stopped:` line naming the work that
  was live at quit (turn, prompts, background task IDs, children).
- [ ] 1.3 PTY: `/quit` typed during a held turn exits the process, and the
  session resumes with the turn recorded as cancelled. Replaces the
  confirmation PTY tests from `7c0f092` and `setup_pty.rs` "quit with work
  in progress?".

## 2. Exit deadline

- [ ] 2.1 One process-wide exit clock (10 s) starts at TUI quit, headless
  shutdown, or the first stop signal; every exit-path `host.shutdown()` is
  bounded by it, and expiry stops background tasks, kills stdio servers, and
  reports `exit deadline reached` through the normal exit.
- [ ] 2.2 Backstop thread exits the process at the deadline if the async
  path has not, after restoring the terminal and killing stdio servers.
- [ ] 2.3 Tests: the async bound with an injected hung shutdown; the
  backstop in a child process; headless result reports the deadline.

## 3. MCP watchdog

- [ ] 3.1 `smith mcp-watchdog --parent <pid> -- <program> [args...]`
  (hidden): inherited stdio and group, `getppid` poll, TERM → 300 ms → KILL,
  exits with the server's status.
- [ ] 3.2 Smith starts declared stdio servers through it; direct spawn with
  a one-time warning when no executable path was injected.
- [ ] 3.3 `killing_smith_mid_turn_stops_harness_and_child_without_graceful_shutdown`
  also asserts the busy MCP server is gone within 5 s; the
  `headless_*_stdio_mcp_server` tests still pass.

## 4. Runtime flake (ForgeAILab/smith#4)

- [x] 4.1 `session::one_shutdown_deadline_bounds_all_active_turns` runs on
  a paused clock and asserts one deadline (30 ms <= elapsed < 60 ms). Fails
  at 90 ms when shutdown applies the timeout three times.
- [x] 4.2 300 consecutive passes with all cores loaded; full testkit suite
  passes. (300/300 with 14 cores saturated; 235 testkit tests pass; runtime
  `b037801` on `fix/shutdown-deadline-test`.)
- [ ] 4.3 Smith pins the runtime commit carrying 4.1; full gate passes.

## 5. Docs and release notes

- [ ] 5.1 `DESIGN.md` (modal list, confirmation list, `/quit` row,
  background task paragraph), `docs/headless-protocol.md` (exit deadline),
  `docs/extensions.md` (watchdog).
- [ ] 5.2 Close issue #4 with the commit references once CI is green.
