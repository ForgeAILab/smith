## Context

Exit runs `HostSession::shutdown` (delegation lifecycle, goal controller,
child coordinator, cache controller, Runtime session drain of up to 5 s plus
a cache drain of up to 5 s, background tasks up to 2 s, journal), then
`shutdown_stdio_servers` in `main` (SIGTERM, 500 ms, SIGKILL, 1 s reap).
Each step is bounded or cooperative on its own; nothing bounds the sum, and
a turn whose provider ignores cancellation holds the drain for its full
timeout. `/quit` with live work opens a confirmation (`App::request_exit`).

A stdio MCP server is a direct child of Smith in Smith's process group.
When Smith is SIGKILLed, the server notices only when it next reads its
closed stdin, which a server in the middle of a long call does not do.

## Goals / Non-Goals

- Goals: `/quit` never asks; one hard exit deadline for every exit path;
  stdio MCP servers die with Smith even on SIGKILL; a deterministic runtime
  conformance test for the shutdown deadline.
- Non-Goals: changing the Runtime's own drain timeout; background shell
  tasks or CLI harnesses after a Smith SIGKILL (the harness already has
  `harness-supervise`); killing an MCP server's own descendants.

## Decisions

- **`/quit`.** `request_exit` always cancels pending prompts and returns
  `Action::Quit`; the exit confirmation dialog is deleted. The app captures
  what was live at that moment (running turn, pending approvals and
  questionnaires, running background tasks by ID, running children) and the
  exit summary prints one `stopped:` line when that list is not empty.
- **Exit deadline.** `EXIT_DEADLINE = 10 s`, not configurable. A process-wide
  exit clock in `smith-cli` starts once, at the first of: the TUI's quit, a
  headless run starting host shutdown, or the first stop signal. Starting it
  also starts a backstop thread.
  - Async path: every exit-path `host.shutdown()` runs under a timeout of
    the remaining time minus a 2 s reserve for MCP cleanup. On expiry Smith
    stops the session's background tasks, kills stdio servers, and continues
    its normal exit with the error `exit deadline reached`, so headless
    still writes its result and the TUI still restores the terminal and
    prints its summary.
  - Backstop: a plain thread (not a Tokio task, so a blocked runtime cannot
    stop it). If the process is still alive at the deadline it restores the
    terminal best effort, SIGKILLs the stdio servers, writes `smith: exit
    deadline reached (10s); stopped remaining work` to stderr, and exits 1.
  - The Runtime's 5 s drain stays: an ordinary hung turn is aborted at 5 s
    and the session is still saved inside the 8 s host budget.
- **MCP watchdog.** A hidden subcommand, `smith mcp-watchdog --parent <pid>
  -- <program> [args...]`, next to `harness-supervise`. It never loads
  configuration. It spawns the server with inherited stdin, stdout, stderr,
  environment, working directory, and process group, so JSON-RPC flows
  directly between Smith and the server. It polls `getppid()` every 100 ms
  and compares it with `--parent`; when they differ, Smith is gone, and the
  watchdog sends the server SIGTERM, waits 300 ms, sends SIGKILL, reaps it,
  and exits 1. On SIGTERM it runs the same stop sequence and exits with the
  server's status. SIGINT and SIGHUP do not stop it: a terminal signal
  already reaches the server directly, and Smith decides the rest. When the
  server exits on its own, the watchdog exits with the same status.
  - `--parent` instead of the parent at startup: Smith may already be gone
    when the watchdog starts.
  - Polling instead of an extra pipe: passing a second descriptor needs
    `pre_exec`, and the workspace forbids `unsafe`.
  - Kill by pid, same group: an own process group broke the
    SIGKILL-mid-turn harness test in 9.6, and a group kill from the
    watchdog would reach Smith.
  - Smith's grace (500 ms) is longer than the watchdog's (300 ms), so a
    SIGTERM-ignoring server is killed by the watchdog before Smith
    escalates to SIGKILL on the watchdog.
  - The executable path is injected from `smith-cli`
    (`std::env::current_exe()`); without one, Smith spawns the server
    directly and warns once, as the CLI harness does.
- **Runtime flake.** The test runs on Tokio's paused clock
  (`start_paused`, current-thread) and asserts `30 ms <= elapsed < 60 ms`,
  which fails at 90 ms if the deadline is applied per turn. The old
  wall-clock bound (150 ms) neither caught that regression nor tolerated a
  loaded runner. The fix sits on `fix/shutdown-deadline-test`, based on
  `feat/smith-web-tools` `49e8197`.

## Risks / Trade-offs

- A watchdog SIGKILLed by Smith's escalation leaves its server orphaned; that
  needs both the server to ignore SIGTERM for 300 ms and Smith to kill the
  watchdog first, which the grace ordering prevents.
- One extra small process per stdio server.
- Removing the confirmation means a mistyped `/quit` stops a long turn; the
  owner chose this. Double Ctrl-C already behaved this way.
