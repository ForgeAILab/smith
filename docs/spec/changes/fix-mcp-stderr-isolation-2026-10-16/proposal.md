---
created_at: 2026-10-17T00:09:41Z
updated_at: 2026-10-17T00:09:41Z
---

## Why

The owner reported on 2026-10-16 that adding an MCP server breaks the
interactive session: lines such as `npm notice ... run npx ...` appear inside
the TUI, usually over the composer, and cannot be cleared.

Smith starts every stdio MCP server with its stderr inherited from Smith's own
process. The watchdog passes the same descriptor on to the real server. While
the TUI owns the alternate screen, whatever `npx`, `node`, or `python` writes
to stderr is painted straight over the frame, and the diff renderer never
repaints those cells. Text output is also a diagnostic the user then loses: a
server that fails to start explains why only on stderr.

## What Changes

- Smith pipes the stderr of each stdio MCP server process (the watchdog, or
  the server when it is started directly). The watchdog keeps passing its own
  stderr to the server, so the server, the watchdog's own spawn errors, and
  any descendant all reach the same pipe. Nothing reaches the terminal.
- A task drains the pipe continuously, so a chatty server can never block on
  a full pipe. It keeps a bounded tail per server (64 lines, 16 KiB), strips
  terminal escapes, keeps only the text after a carriage return, and redacts
  every environment value Smith passed to the server.
- Each line goes to the session log at `info`. When a server fails to start
  or exits on its own, the tail is logged at `warn`.
- The last few lines are attached to the failure that `/mcp` and the status
  bar already show. A server that exits after connecting is now reported as
  failed with its exit status and stderr, instead of staying `connected`.
  Healthy chatter adds nothing to the screen.
- A headless or child run adds one diagnostic per server that failed to
  start, so `smith -p` users still learn why a server is missing.

## Impact

- Affected specs: `extension-system`
- Affected code: `crates/smith-runtime/src/mcp/stdio.rs`,
  `crates/smith-runtime/src/mcp.rs`, `crates/smith-cli/src/mcp.rs`,
  `crates/smith-cli/src/local_command/mcp.rs`,
  `crates/smith-cli/src/runtime_host.rs`, `docs/extensions.md`
- The watchdog itself is unchanged; its contract tests keep their
  expectations.
- A tool call that fails because the server died still returns the upstream
  `agent-runtime-mcp` error to the model. Server stderr is untrusted
  third-party text and is not forwarded into the model's context.
