---
created_at: 2026-08-07T20:26:54Z
updated_at: 2026-08-07T20:33:17Z
---

## Why

Smith cannot connect to Model Context Protocol servers. The `extension-system`
spec already requires it — "Smith SHALL support trusted compile-time Rust
registration and MCP alongside the subprocess protocol", with a scenario named
"Register an MCP tool server" — and `project.md` lists extensions/MCP as
deferred work. Nothing implements it. The only trace in the codebase is a TUI
render fixture asserting how a tool named `mcp.some_third_party_tool` should
display.

The result is that Smith's tool surface is closed. Everything the agent can do
is something a Smith contributor compiled in. MCP is the ecosystem's answer to
that, and users expect a coding agent to reach their existing servers.

This change is the consumer half of a two-repository effort. The shared
mechanism — transport, schema translation, and the authority model for tools
whose effects Smith cannot inspect — is specified by
`../agent-runtime/docs/spec/changes/add-mcp-capability-source-2026-08-07/`.
This proposal owns what that package deliberately excludes: where servers are
declared, how a user comes to trust one, and how their tools appear.

## What Changes

- Declare MCP servers in layered configuration under an `[mcp.servers.<name>]`
  table, resolved through the existing precedence chain and explainable by the
  same source-attribution machinery as every other setting.
- **Bind server execution to hash-bound project trust.** A project-declared
  stdio server is a repository asking Smith to run an arbitrary command, which
  is exactly what `ExecutableKind` exists to gate. Add an `McpServer` variant
  and require confirmation of the exact command content before any spawn.
- **Forbid self-authorization.** A project-declared server MUST NOT arrive
  pre-approved, and repository configuration MUST NOT auto-approve MCP tools,
  matching the existing rule that repository config cannot self-authorize tools.
- Register connected servers' tools through the shared ability/tool registry so
  they obtain approval and attribution identically to built-ins — the behavior
  the `extension-system` spec already requires.
- Add a `/mcp` command listing configured servers, their connection state, tool
  counts, and failure reasons, plus status-line visibility while servers connect.
- Render remote tools as `mcp__<server>__<tool>` with arguments hidden by default,
  extending the existing redaction-safe display rule to server-defined
  arguments Smith cannot classify.
- Degrade rather than fail: a server that will not start leaves the session
  fully usable and reports why.

## Impact

- Affected specs: `configuration`, `extension-system`, `client-surfaces`,
  `tool-call-display`
- Affected code: `smith-config` (declarations, trust, resolution),
  `smith-runtime` (composition wiring), `smith-cli` (the `/mcp` command),
  `smith-tui` (status, display, the existing `mcp.*` fixture)
- Dependency: `agent-runtime-mcp` with its `stdio` and `http` transports
- Public compatibility: additive configuration. Existing projects without
  `[mcp.servers]` behave identically and gain no new dependency at runtime.
- Security: this is the first path by which repository-controlled configuration
  can cause Smith to execute a program the user did not compile in. The trust
  and non-self-authorization requirements are the load-bearing part.
- Prerequisite: the coordinated agent-runtime change must land slices 1 through
  4 before this change's Section 3 can begin.

## Non-Goals

- No server installation, discovery, or registry browsing. Users bring their own
  commands.
- No MCP resources, prompts, or sampling — tools only, matching the shared
  package's scope.
- No MCP server discovery, installation, or registry browsing (see above). Remote
  HTTP servers were originally deferred out of this change; they are now included
  — the credential path they needed was exercised by the stdio slice, and a
  remote server reuses it unchanged.
- No per-tool approval configuration beyond what the existing approval policy
  already expresses.

## Delivery Slices

1. Configuration surface: `[mcp.servers.<name>]` declarations, typed resolution,
   source attribution, and validation. No execution.
2. Trust: the `McpServer` executable kind, digest binding over the resolved
   command, the confirmation prompt, and the non-self-authorization preflight.
3. Composition: connect trusted servers, register their tools, and isolate
   failures. Requires agent-runtime slices 1 through 4.
4. Surfaces: `/mcp`, status-line connection state, and `mcp__<server>__<tool>`
   display.
5. Remote servers: the `http` transport, a bearer `credential`, and declared
   headers — all resolved through the same secret path and bound by the same
   trust decision as a local command.

Slice 2 must pass before slice 3 spawns anything.

## Approval Boundary

Approval authorizes Stage 2 implementation in this repository only. It does not
authorize the coordinated agent-runtime change, remote HTTP servers, MCP
resources/prompts/sampling, or any server-installation behavior.
