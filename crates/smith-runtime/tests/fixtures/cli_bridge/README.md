# Recorded CLI output for the capability bridge

Captured on 2026-10-13 from the installed CLIs talking to a throwaway MCP
server, for the spike in
`docs/spec/changes/add-cli-agent-capability-bridge-2026-10-13/tasks.md`
section 1. Tests and the scripted CLI stand-in build on these files; a shape
Smith guessed is not a test.

| Directory | CLI | Version |
| --- | --- | --- |
| `claude-code-2.1.296/` | `claude -p --output-format stream-json --verbose` | 2.1.296 |
| `codex-0.162.1/` | `codex exec --json` | 0.162.1 |

The MCP server in every recording is named `spike`, so tool names read
`mcp__spike__<tool>` (Claude Code) and `"server":"spike"` (Codex). Its tools
are `fast_echo`, `slow_wait`, `registry.search` (a dotted name), and one
whose name is 75 characters long.

## What is in each directory

- `stream/` is what the CLI printed on stdout, one JSON event per line.
- `wire/` is what the MCP server saw: `at_ms` since the connection opened,
  `dir` (`in` from the CLI, `out` to it, `call-finished`, `open`, `closed`),
  and the JSON-RPC message.
- `stderr/` is a CLI's stderr where the failure never reached stdout.

## Claude Code

| File | How it was produced |
| --- | --- |
| `stream/mcp-call.jsonl` | `--allowedTools mcp__spike`, one `fast_echo` call |
| `stream/mcp-call-beside-own-tools.jsonl` | same allow rule, default permission mode: `ToolSearch`, the MCP call, `Bash` (ran), `Write` (refused), `Read` (ran) |
| `stream/mcp-call-accept-edits.jsonl` | `--permission-mode acceptEdits --allowedTools mcp__spike`, same four steps, all ran |
| `stream/mcp-call-denied-without-allow-rule.jsonl` | no allow rule: the MCP call is refused with a `permission_denied` event |
| `stream/mcp-call-timed-out.jsonl` | `MCP_TOOL_TIMEOUT=4000`, `slow_wait` 25 s |
| `stream/three-tool-uses-in-one-message.jsonl` | one assistant message with three MCP `tool_use` parts |
| `stream/failed-turn.jsonl` | an unknown model: `result.is_error` is true while `subtype` is `success`; exit status 1 |
| `stream/resumed-turn.jsonl` | `--resume` with the server declared again |
| `stream/server-missing-on-resume.jsonl` | `--resume` without the server |
| `wire/handshake-and-call.jsonl` | the `server/discover` probe, `initialize`, `tools/list`, one call |
| `wire/call-cancelled-on-timeout.jsonl` | the CLI sends `notifications/cancelled` when its timeout fires |
| `wire/concurrent-calls-to-read-only-tools.jsonl` | two calls overlap only because the tools carried `readOnlyHint` |

## Codex

| File | How it was produced |
| --- | --- |
| `stream/mcp-call.jsonl` | `-c mcp_servers.spike.default_tools_approval_mode="approve"`, `--sandbox read-only` |
| `stream/mcp-call-refused-without-approval-mode.jsonl` | no approval mode: "MCP tool call requires approval, but approval policy is never" |
| `stream/mcp-call-beside-own-tool.jsonl` | `--sandbox workspace-write`: a `command_execution` item, then an `mcp_tool_call` item |
| `stream/mcp-call-timed-out.jsonl` | `-c mcp_servers.spike.tool_timeout_sec=4`, `slow_wait` 25 s |
| `stream/three-calls-in-one-step.jsonl` | three calls the CLI ran one after another |
| `stream/failed-turn.jsonl` | an unknown model: an `error` event, then `turn.failed` |
| `stream/resumed-turn.jsonl` | `codex exec <flags> resume <thread> <prompt>` |
| `stream/server-missing-on-resume.jsonl` | resumed without the server |
| `stderr/exec-flags-after-resume-rejected.txt` | `codex exec resume <thread> --json ... -C <dir> --sandbox ...` exits 2 |
| `wire/handshake-and-call.jsonl` | `initialize`, `tools/list`, one call with its `_meta` |
| `wire/call-timed-out-without-cancel.jsonl` | the CLI's timeout fires and nothing is sent to the server |

## Redactions

Applied mechanically; nothing else was rewritten.

- Machine paths became `/workspace`, `/spike`, `/scratch`, and `/home/user`.
- Claude Code's `init` event was reduced to `type`, `subtype`, `session_id`,
  `uuid`, `model`, `permissionMode`, `claude_code_version`, `cwd`, the `spike`
  entry of `mcp_servers`, and the `spike` tools plus a few built-in names.
  The full event lists the recording machine's plugins, skills, and agents.
- Claude Code's `hook_started`, `hook_response`, and `rate_limit_event`
  events were dropped.

Session, thread, request, and tool-use ids are as recorded.
