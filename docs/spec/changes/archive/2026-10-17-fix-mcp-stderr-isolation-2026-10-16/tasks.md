---
created_at: 2026-10-17T00:09:41Z
updated_at: 2026-10-17T20:03:09Z
completed_at: 2026-10-17T01:11:34Z
---

## 1. Implementation
- [x] 1.1 Pipe the stdio MCP process's stderr instead of inheriting it, for
      both the watchdog and the direct-spawn fallback. The watchdog is
      unchanged: its stderr is now Smith's pipe.
- [x] 1.2 Drain the pipe continuously into a bounded, sanitized, redacted
      tail; log lines at `info` and the tail at `warn` on failure or exit.
- [x] 1.3 Keep reading for a 500 ms grace when Smith drops the process,
      then stop, so an escaped descendant cannot keep the drain alive.
- [x] 1.4 Report a server that exits on its own after connecting as failed,
      with its exit status, and retire its tools at the next safe boundary.
      An exit is noticed from the closed protocol output or a 250 ms
      process poll (the live check found a descendant holding stdout open).
- [x] 1.5 Attach the last stderr lines to the failure shown by `/mcp`.
- [x] 1.6 Add one diagnostic per failed server to headless and child runs.
- [x] 1.7 Describe the behaviour in `docs/extensions.md`.
- [x] 1.8 Install the log subscriber before the host starts and hold events
      until the session's log file exists, so server startup is logged.
- [x] 1.9 Live-check follow-up: render the MCP footer on the first loop
      pass after a rebind, which previously dropped the failed count.

## 2. Tests
- [x] 2.1 Runtime: stderr reaches the tail, is redacted, sanitized, and
      bounded; a chatty server never blocks; a server exiting on its own is
      reported with its status, also while an orphan holds its output.
- [x] 2.2 Runtime: a stdio server that writes to stderr and fails to start
      is reported failed with those lines; a connected server that exits is
      reported failed and its tools withdrawn; a shutdown is not a failure.
- [x] 2.3 CLI contract: a trusted server's stderr chatter never reaches
      Smith's stderr and reaches the session log; a failing server's stderr
      reaches the headless diagnostics, redacted, and never the terminal.
      Both fail against inherited stderr.
- [x] 2.4 The watchdog contract tests pass unchanged.
- [x] 2.5 CLI: `/mcp` quotes a failed server's stderr on its one line; the
      held log buffer is written first and bounded.

## 3. Verification
- [x] 3.1 Focused MCP suites, clippy, fmt, and the contract suite under
      `/var/folders`.
- [x] 3.2 Full gate.
- [x] 3.3 Live tmux check: stderr-writing stdio servers, including the
      owner's `npx -y rea-agents@6.5.0 mcp`, leave the composer clean; their
      lines land in the session log; failures show in `/mcp` and the footer.
