---
created_at: 2026-10-16T00:10:07Z
updated_at: 2026-10-16T04:53:48Z
---

## Why

Choosing `cli/claude-code/...` or `cli/codex/...` should feel like running
that agent, with Smith's extras on top. Today it does not. In session
`3ef40514` (orchestrator on `cli/claude-code/opus`) every WebFetch and
WebSearch call, `python3 -c ...`, and `smith --help` were denied. Only
Claude's always-allowed reads (Read, `ls`, `cat`) ran.

The cause is the default. `allow_own_tools` is off unless the owner sets it,
so Claude starts in its default permission mode. In `-p` mode nobody can
answer a permission prompt, so every prompt is an automatic denial. Turning
the setting on does not fix it either. It selects Claude's `acceptEdits`,
which still prompts, and so denies, WebFetch, WebSearch, and most shell
commands (checked against Claude Code 2.1.301). For Codex it selects
`workspace-write`, which has no network.

Owner direction on 2026-10-15: keep it simple. Someone who picks an
installed agent wants it to work with no extra configuration, and limits are
something they add, not something Smith starts with.

## What Changes

- **BREAKING (security default):** `[harness.<kind>] allow_own_tools`
  defaults to on. Owners who never set it get a CLI that runs its own reads,
  writes, commands, and network tools without asking.
- **On means the CLI's no-prompt mode**, because a non-interactive CLI
  cannot ask:
  - Claude Code: `--permission-mode bypassPermissions`, replacing
    `acceptEdits`.
  - Codex: `--dangerously-bypass-approvals-and-sandbox`, replacing
    `--sandbox workspace-write`.
- **Off is unchanged.** `allow_own_tools = false` keeps today's invocation:
  Claude in its default mode with only the `smith` server pre-allowed, Codex
  with `--sandbox read-only`.
- **Read-only postures still narrow, now at the root as well.** A child
  without write-capable tools already runs the CLI with its own tools off. A
  root profile with a `plan` or `review` posture now does the same. Today
  the root ignores posture.
- **Owners can add limits.** Turn it off with `allow_own_tools = false`. For
  Claude Code, owner `args` come after Smith's flags, so
  `args = ["--permission-mode", "acceptEdits"]` or
  `args = ["--disallowedTools", "WebFetch"]` narrows the default. Codex
  rejects a repeated `--sandbox`, so its only limit is turning the setting
  off. Project layers still cannot set `allow_own_tools` or `args`.
- The capability bridge is unchanged.

## Impact

- Affected specs: `configuration`.
- Affected code: `crates/smith-config/src/resolve/harness.rs` (default),
  `crates/smith-config/src/model.rs` and `resolve/types.rs` (doc comments),
  `crates/smith-runtime/src/cli_agent/invocation.rs` (flags),
  `crates/smith-runtime/src/factory/construction.rs` (root posture), the CLI
  stand-in and contract tests in `crates/smith-cli`, and the live canaries.
- Docs: `docs/configuration.md` (installed agents, harness section),
  `docs/security.md` (installed coding agents).
- Users: existing `cli/...` profiles gain full CLI autonomy on upgrade. The
  release notes must say so and name `allow_own_tools = false`.
