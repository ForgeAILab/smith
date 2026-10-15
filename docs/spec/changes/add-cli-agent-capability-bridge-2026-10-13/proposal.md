---
created_at: 2026-10-13T22:40:43Z
updated_at: 2026-10-15T04:48:58Z
---

## Why

A `cli/<kind>/<model>` turn runs entirely inside the installed CLI. Smith
hands over the prompt and watches the stream: `ExternalTurnRequest` carries
the input, a resume id, the turn id, and a cancellation, and nothing else.
Nothing Smith provides reaches the CLI's model. It cannot spawn a child on
another profile, consult the advisor, call a Smith-declared MCP server or a
plugin tool, or load a Smith skill. Selecting Claude Code or Codex as the
main agent therefore gives up most of what Smith is for.

The harness change (`add-cli-agent-harness`) ruled out routing the CLI's
*own* tools through Smith, and that still holds. This change adds the
opposite direction: the CLI calls Smith. Owner direction on 2026-10-13: the
CLI keeps its own file and shell tools, and Smith offers what the CLI lacks.

## What Changes

- **A turn-scoped capability bridge.** For each harness turn Smith serves a
  private MCP endpoint and names it in the CLI invocation. The CLI's model
  sees Smith capabilities as ordinary MCP tools under one server, `smith`.
- **The bridged set is what the CLI lacks.** It is the session's authorized
  tools minus the ones the CLI replaces: the core coding tools, the
  todo-planning tool, and the goal tools. Everything else follows the rules
  that already decide a native session's tools: the profile's `delegation`
  and `advisor` settings, posture, the child-surface rules, and capability
  limits. No new configuration key. An empty set starts no bridge and leaves
  the CLI invocation exactly as it is today; the discovery tools and
  `artifact.read` never keep the set non-empty by themselves.
- **A bridged call is a Smith tool call.** Agent Runtime dispatches it
  through the same validation, authorization, approval, workspace
  enforcement, output bound, events, and history commit as a model-requested
  call. It renders as one of Smith's own tool rows, and the CLI's echo of
  the same call is not shown a second time.
- **`agent` works from a harness session** with its existing actions,
  profiles, and wait policy. A child the model did not wait for is delivered
  by a child-completion turn that runs on the CLI, continuing its session,
  instead of on a provider that cannot serve a `cli/...` model.
- **`advisor` works from a harness session.** Its transcript is built from
  what Smith observed of the CLI's turn, with CLI-run tools labelled as such.
- **Skills** are found with `registry.search`, and activating one returns its
  instructions as the tool result, because Smith cannot stage text into a
  context the CLI owns.
- **Smith-declared MCP servers and plugin and module tools** are reached
  through Smith. They are never written into the CLI's own MCP
  configuration, so trust pins, credentials, and the conservative authority
  floor keep applying.
- **Guidance follows the tools.** The instruction fragment for each bridged
  capability is delivered to the CLI when, and only when, that capability is
  bridged.
- `allow_own_tools` and the CLI's own MCP servers and settings are unchanged.
- **Tests that do not need a CLI.** A scripted CLI stand-in reads the
  arguments Smith built, launches the declared relay, and drives the real
  endpoint, so the whole path runs in the default gate. Its output is
  recorded from the real CLIs. Tests against installed CLIs, and a check
  that a CLI release has not changed the flags or output Smith depends on,
  are opt-in. See the test plan in `design.md`.

## Impact

- Affected specs: `provider-runtime`, `runtime-integration`,
  `configuration`, `client-surfaces`, `child-agents`, `advisor`,
  `extension-system`
- Affected code:
  - Agent Runtime (new branch from the pinned `d2752c1`):
    `agent/external.rs` (a dispatch handle on the turn request),
    `agent/driver/external_turn.rs` (multiplex backend events with bridged
    calls and interactions), `agent/driver/mod.rs` (internal turns on an
    external session); then a pin bump in Smith's `Cargo.toml`.
  - Smith: `smith-runtime/src/cli_agent.rs` (invocation, echo suppression),
    a new crate-private bridge module (MCP server on a Unix socket),
    `advisor.rs` (harness transcript), `smith-cli` (a hidden relay
    subcommand), `docs/configuration.md`.
  - Dependencies: `rmcp` with its `server` feature in `smith-runtime`.
- Compatibility: every existing root `cli/*` session gains the bridge on
  upgrade, offering what its profile already allows natively. A profile that
  denies every tool and skill is invoked exactly as before.
- Security: the endpoint executes with Smith's authority. It is a Unix
  socket in an owner-only directory, exists only while its turn runs, and
  every call still passes Smith's approval policy. See `design.md`.
- Not in scope: routing the CLI's own tools through Smith; offering Smith's
  core coding tools over the bridge; sending Smith's system prompt, project
  instructions, or memory index to the CLI; goal continuation on a harness
  session; context planning or compaction for harness turns.
