---
created_at: 2026-09-05T11:15:00Z
updated_at: 2026-09-05T11:15:00Z
---

## Why

Smith can reach a model through a subprocess, but not an agent. Claude Code and
Codex are installed on the same machine as Smith, already authenticated, and
already able to read a repository and edit it. Today the only way to use one
from Smith is a `command-jsonl` bridge that has to pretend the CLI is a
stateless model: it re-sends the entire canonical history on every attempt,
disables the CLI's own tools, and reads prose for tool intent. That is slow,
expensive, and loses exactly the thing the CLI is good at.

Agent Runtime now names the boundary this needs. An external agent backend runs
one whole turn and streams normalized events, and Smith already renders those
events.

## What Changes

- Add the profile-level `harness` selection: `claude-code` or `codex`. A profile
  with a harness runs its turns on that CLI instead of a model provider, and is
  usable as the main agent or as a delegated child.
- Add per-harness settings under `[harness.<name>]`: the executable, model,
  extra arguments, working directory, and whether the CLI may run its own
  tools. Every value stays layered and source-explainable like the rest of
  Smith's configuration.
- Default the CLI's own tools to **off**. A harness turn runs read-only unless
  the owner opts in, because a CLI running its own tools executes them outside
  Smith's approval, workspace-scoping, and usage accounting.
- Continue the CLI's own session across turns instead of replaying history.
  Smith stores the identity the CLI reports and offers it back; a CLI that
  cannot resume reports a new one and the turn proceeds.
- Inherit the ambient environment, with explicit per-harness overrides. A
  coding CLI needs its own login, PATH, and home to work at all, and clearing
  the environment is what made the earlier bridge report "not logged in".
- Label harness activity in the TUI and headless output, so it is always
  visible that a turn ran on an installed CLI, which tools it ran itself, and
  that Smith did not approve them.

## Impact

- Affected specs: `configuration`, `provider-runtime`, `client-surfaces`.
- Affected code: `smith-config` profile and harness settings; `smith-runtime`
  factory composition and two adapters; `smith-tui` and headless rendering for
  the external events; documentation.
- Compatibility: additive. A profile with no `harness` behaves exactly as
  before, and no existing configuration changes meaning.
- Security: this is the significant one. A harness with its own tools enabled
  executes reads, writes, and commands that Smith never approved, never scoped
  to the workspace, and cannot show in its tool history. That is why it is off
  by default, why enabling it is an owner-only setting, and why harness
  activity is labelled rather than shown as ordinary tool use.

## Non-Goals

- No attempt to route a CLI's own tool calls through Smith's approval or
  workspace boundary. Smith never dispatched them and cannot vouch for them;
  presenting them as approved tool calls would be a false claim.
- No context planning, compaction, or prompt-cache maintenance for harness
  turns. The CLI owns its own context, which is the point of continuing its
  session.
- No installation, version management, or authentication flow for either CLI.
  Smith uses what is installed and logged in.
- No replacement for `command-jsonl`, which remains the way to reach a model
  through a subprocess.
