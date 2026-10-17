---
created_at: 2026-10-17T00:09:41Z
updated_at: 2026-10-17T00:09:41Z
completed_at:
---

## 1. Implementation
- [ ] 1.1 Pipe the stdio MCP process's stderr instead of inheriting it, for
      both the watchdog and the direct-spawn fallback.
- [ ] 1.2 Drain the pipe continuously into a bounded, sanitized, redacted
      tail; log lines at `info` and the tail at `warn` on failure or exit.
- [ ] 1.3 Snapshot queued bytes and stop the drain when Smith drops the
      process, so an escaped descendant cannot keep it alive.
- [ ] 1.4 Report a server that exits on its own after connecting as failed,
      with its exit status, and retire its tools at the next safe boundary.
- [ ] 1.5 Attach the last stderr lines to the failure shown by `/mcp`.
- [ ] 1.6 Add one diagnostic per failed server to headless and child runs.
- [ ] 1.7 Describe the behaviour in `docs/extensions.md`.

## 2. Tests
- [ ] 2.1 Runtime: stderr reaches the tail, is redacted, sanitized, and
      bounded; a server exiting on its own is reported with its status.
- [ ] 2.2 Runtime: a stdio server that writes to stderr and fails to start
      is reported failed with those lines.
- [ ] 2.3 CLI contract: a trusted server's stderr chatter never reaches
      Smith's stderr, and a failing server's stderr reaches the headless
      diagnostics.
- [ ] 2.4 The watchdog contract tests pass unchanged.

## 3. Verification
- [ ] 3.1 Focused MCP suites, clippy, fmt.
- [ ] 3.2 Full gate.
- [ ] 3.3 Live tmux check: a stderr-writing stdio server leaves the composer
      clean, and its lines land in the session log.
