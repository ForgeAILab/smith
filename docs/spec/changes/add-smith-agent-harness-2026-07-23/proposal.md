---
created_at: 2026-07-23T20:43:51Z
updated_at: 2026-07-27T20:48:01Z
---

## Why

Smith now has a working TUI, host approval/workspace policy, and coding tools
over the shared `agent-runtime`, but the integration predates the runtime's
registry-driven context release. Smith still constructs a fake provider with a
hard-coded model and prompt, supplies no required model profile, has no
production transport or resolved configuration, and has no durable session
store.

The shared runtime is now the canonical owner of provider normalization, model
and context planning, the direct provider/tool loop, tool execution,
cancellation, events, usage accounting, registry primitives, and deterministic
test support. This proposal is revised so Smith owns only product policy and
host infrastructure and composes those shared mechanisms through one reusable
boundary.

## What Changes

- Adopt the committed `agent-runtime` 0.1 facade as Smith's canonical runtime
  mechanism. A release build SHALL use an exact semantic version or Git
  revision; a sibling path is only an uncommitted local-development override.
- Add a Smith-owned configuration layer that resolves defaults, user/project
  files, profiles, environment variables, CLI arguments, and per-session
  overrides into one validated value with per-field provenance.
- Require every selected provider/model to resolve an immutable runtime model
  profile or layered catalog entry. Unknown limits fail before terminal entry
  or provider network I/O.
- Add one Smith composition API, provisionally `smith-runtime`, that maps
  resolved Smith policy into `RuntimeBuilder`: provider, provider name, model
  profile/catalog, context policy, prompt, retry/limits, tools, approval,
  workspace, stores, observers, and clock.
- Make the first production vertical slice the shared OpenAI-compatible
  Chat-Completions adapter over a Smith-supplied streaming HTTP transport.
  Smith resolves credential references at the host boundary and never places
  plaintext secrets in project configuration.
- Keep the deterministic fake provider as a test/development seam, not the
  default interactive provider.
- Implement Smith-owned session storage and event journaling behind the
  runtime's `SessionStore` and `EventObserver` contracts, then use
  `StartSession` for explicit create/resume.
- Reuse shared runtime messages, events, usage counters, model profiles,
  context plans, tool traits, approval/workspace contracts, and registry
  identities. Smith MUST NOT maintain parallel copies of those mechanisms.
- Keep provider adapters not yet present upstream—OpenAI Responses and
  Anthropic Messages—as shared-runtime follow-ups. Smith consumes them after a
  compatible runtime release instead of creating consumer-local duplicates.
- Preserve the remaining Smith product roadmap: layered trust and credentials,
  cache-lifetime policy, monitors, direct children, extensions, `smith -p`,
  TUI polish, and one-way Forge integration.

## Non-Goals

- Reimplementing the provider/tool loop, context planner, provider contract,
  tool executor, usage ledger, event schema, or registry in Smith.
- Treating raw TOML or environment variables as runtime configuration without
  typed validation and provenance.
- Guessing model context limits or silently falling back for an unknown model.
- Shipping OpenAI Responses or Anthropic support locally before the shared
  runtime exposes compatible adapters.
- Committing a sibling `../agent-runtime` dependency as the release source.
- Adopting the unapproved `agent-runtime` security-boundary proposal in this
  revision; that breaking API requires a coordinated follow-up after it lands.
- Modifying Agent Runtime, Nyx, or Open Forge under this approval.
- Publishing packages, deploying a daemon, or using unsupported Codex
  authentication/model endpoints.

## Impact

- Affected specs: `runtime-integration`, `configuration`, `provider-runtime`,
  `agent-session`, `usage-accounting`, `tool-execution`, `prompt-cache`,
  `extension-system`, `client-surfaces`, and `forge-integration`
- Affected code: workspace manifests; new or equivalent Smith configuration
  and runtime-composition modules; `smith-cli`; session/secret/HTTP host
  adapters; runtime-driven tests
- Removed ownership: Smith-local provider/loop/context/usage/registry
  mechanisms described by the original proposal
- External interfaces: resolved Smith configuration, the Smith runtime factory,
  CLI configuration/diagnostics, versioned machine output, and future Forge
  host adapters
- Security impact: provider credentials are resolved only at the host boundary;
  unknown models and missing policy fail closed; executable project content
  remains hash-trusted
- Operational impact: startup validates provider, credential, model limits,
  context policy, workspace, and persistence before entering the TUI

## Resolved Decisions

| Topic | Decision |
| --- | --- |
| Shared mechanism owner | `agent-runtime` 0.1 facade |
| Smith ownership | Configuration, prompts, transport, credentials, stores, tools, approval/workspace policy, orchestration, and presentation |
| Composition boundary | One reusable Smith runtime factory used by TUI, `-p`, tests, and future Forge adapters |
| Dependency source | Exact release/Git revision; sibling path only as an uncommitted Cargo patch |
| Initial usable provider | Shared OpenAI-compatible Chat-Completions adapter over a Smith production transport |
| Initial test provider | Shared deterministic fake with an explicit fake model profile |
| Model limits | Required explicit profile or layered catalog; never guessed |
| Model metadata | Explicit/session > provider-local > embedded > validated cached remote, retaining runtime provenance |
| Context planning | Shared runtime `ContextPolicy`, planner, optional compactor, and plan events |
| Sessions | Smith `SessionStore` plus event journal over shared snapshots/events |
| Usage | Shared disjoint counters and attempt records; Smith owns cost tables and presentation |
| Missing upstream adapter | Add upstream, release, then consume; do not fork in Smith |
| Upstream security proposal | Coordinated follow-up after approval and implementation |
| Runtime lifetime | In-process; the constructing TUI/CLI/host owns shutdown |
| Initial platforms | macOS and Linux |

## Deferred Choices

- Select the final tagged or exact Git source after Agent Runtime has a real
  repository remote and release reference.
- Decide whether Smith's configuration and composition boundaries begin as
  modules in `smith-host` or separate `smith-config` / `smith-runtime` crates;
  their contracts and dependency direction are fixed either way.
- Select the concrete Rust HTTP and OS credential backend during implementation
  after dependency/license review.
- Recheck all public package names immediately before publication.

## Approval Boundary

Re-approval authorizes Stage 2 implementation in this repository according to
the revised `tasks.md`. It preserves already-completed TUI, host, and coding-tool
work. It does not authorize changes to Agent Runtime, Nyx, Open Forge, package
publication, daemon deployment, unsupported Codex access, or implementation of
the pending upstream security proposal.
