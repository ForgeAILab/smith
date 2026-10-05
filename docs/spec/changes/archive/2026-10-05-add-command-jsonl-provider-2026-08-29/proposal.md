---
created_at: 2026-08-30T03:17:16Z
updated_at: 2026-09-05T04:28:00Z
---

## Why

Agent Runtime now has a bounded command-provider mechanism, but Smith cannot
select or configure it. Users can choose native HTTP adapters through the one
Smith runtime factory, while a trusted local model bridge cannot participate in
the same provider, model, MCP/tool, approval, persistence, and TUI/headless
paths.

Existing coding CLIs are not automatically direct model providers. In
particular, Codex app-server owns threads, turns, approvals, sandboxed command
and file execution, MCP, and streamed agent events. Wrapping that agent loop as
a Smith `Provider` would create two owners for history, tools, retries, and
authority. Smith needs a narrow model-level command protocol first; autonomous
CLI backends remain a separate future capability.

## What Changes

- Add the Smith provider kind `command-jsonl`, backed by Agent Runtime's
  feature-gated command-provider framework and composed through the existing
  `Arc<dyn Provider>` factory seam.
- Add a namespaced `[providers.<name>.command]` table for an absolute
  executable, fixed non-secret arguments, a workspace-or-absolute working
  directory, and an explicit environment map. Every value remains layered and
  source-explainable.
- Allow process-bearing command-provider settings only from owner-controlled
  user configuration or a future explicit invocation/session authority.
  Project configuration may select a user-declared command provider but cannot
  define or override its executable, arguments, working directory, or
  environment.
- Define `smith-command-provider` JSONL protocol revision 1. A fixed preflight
  operation validates protocol revision and the selected model. Each provider
  attempt then receives one versioned canonical request on stdin and emits
  versioned text, tool-call, usage, finish, or error frames on stdout.
- Make revision 1 a text-input, streaming, tool-capable protocol with required
  usage reporting and no reasoning, structured-output, prompt-cache, or
  server-side continuation capability. Unsupported requests fail before a
  process starts.
- Preserve Smith and Agent Runtime as the only owners of canonical history,
  context planning, MCP and built-in tool execution, approvals, retries,
  cancellation, usage accounting, persistence, and events. The child process
  is created once per visible provider attempt and receives no ambient
  environment.
- Update Smith's exact Agent Runtime revision to one containing the approved
  command-provider framework and enable its `command-provider` feature only in
  `smith-runtime`.
- Add deterministic command fixtures and shared-factory tests covering
  preflight, a text turn, a Smith/MCP tool round trip, retry attribution,
  cancellation, malformed output, redaction, and TUI/headless equivalence.

## Impact

- Affected specs: `configuration`, `provider-runtime`, `runtime-integration`.
- Affected Smith code: workspace dependency pin; `smith-config` file and
  resolved models; `smith-runtime` command protocol, factory, preflight,
  redaction, and composition tests; configuration/reference documentation.
- Upstream dependency: the approved Agent Runtime change
  `add-command-provider-framework-2026-08-29` may be consumed through Smith's
  documented ignored sibling patch during coordinated development, but must be
  available at the exact revision Smith pins before the change is complete.
- Active-change coordination: `add-google-gemini-provider-2026-08-02` also
  touches provider configuration and factory composition. This change adds an
  orthogonal provider-owned `command` table and does not alter its dedicated
  model-file, Google descriptor, endpoint, or credential work.
- Public compatibility: additive. Existing native providers and projects with
  no `command-jsonl` provider behave identically.
- Security: a command provider receives the complete planned model request,
  including prompt content and tool schemas, and executes as the Smith user.
  Its executable surface is therefore restricted to user-controlled settings,
  direct argv, an allowlisted environment, bounded I/O, explicit compatibility
  preflight, and process-tree cleanup.

## Non-Goals

- No generic shell command template, shell interpolation, PATH lookup, or
  prompt text in argv.
- No direct `codex-cli`, Claude Code, Cursor, Gemini CLI, OpenCode, or other
  autonomous-agent adapter in this change.
- No reuse of another CLI's conversation, MCP, approval, tool, retry, or
  persistence loop.
- No command-provider setup wizard, executable discovery, installation, or
  package-manager invocation.
- No project-local executable grant or interactive trust ceremony. Project
  layers may only select a provider whose entire process declaration came from
  owner-controlled configuration.
- No command-provider reasoning, structured output, images, provider cache,
  rate-limit metadata, credential pools, or server-side continuation in
  protocol revision 1.
- No changes to Agent Runtime under this approval.

## Approval Boundary

Approval authorizes Stage 2 implementation in the Smith repository, using the
documented ignored sibling patch until a compatible immutable Agent Runtime
revision is available. It does not authorize an autonomous agent backend,
unsupported Codex subscription access, a project-controlled process
declaration, changes to the sibling Agent Runtime, or package publication.
