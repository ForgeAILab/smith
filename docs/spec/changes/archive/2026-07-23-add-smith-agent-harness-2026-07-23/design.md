## Context

Smith is a terminal-first product and future Forge host over the sibling
`agent-runtime` repository. The shared runtime is now the canonical owner of
the provider/tool loop, normalized provider events, context planning, model
profiles, registries, usage, cancellation, tool execution, and deterministic
test mechanism. Smith owns the decisions a neutral runtime deliberately cannot:
configuration and provenance, prompts, concrete networking, credentials,
project trust, persistence policy, built-in tools, approvals, workspace
authority, orchestration, and presentation.

The current slice already proves this boundary at a low level. `smith-cli`
constructs `RuntimeBuilder`, `smith-tools` implements the shared `Tool` trait,
`smith-host` supplies approval and workspace adapters, and `smith-tui` reduces
shared runtime events. It is not yet a usable production composition:

- the provider and model are hard-coded to the deterministic fake;
- no builder supplies the model profile now required by Agent Runtime 0.1;
- there is no resolved Smith configuration, credential backend, or production
  `HttpTransport`;
- there is no injected `SessionStore` or canonical event journal; and
- the committed manifest depends on sibling paths instead of a versioned
  release source.

This revision turns that partial slice into one reusable Smith composition path
without reintroducing consumer-local copies of shared mechanisms. A Smith
process still owns all active work. Session state and usage may survive restart;
monitors and child tasks do not.

## Goals / Non-Goals

### Goals

- Adopt the shared runtime facade as the only provider/tool/context execution
  path.
- Resolve typed, provenance-carrying Smith configuration before constructing a
  runtime.
- Make the shared OpenAI-compatible adapter usable through a Smith production
  transport and credential resolver.
- Run the same Smith runtime composition from the TUI, non-interactive CLI,
  tests, and future Forge adapters.
- Persist resumable sessions and accurate-or-labelled usage records.
- Support project-specific settings and extensions through an explicit trust
  boundary.
- Turn command output and WebSocket messages into live session notifications
  without interrupting an in-flight model stream.
- Keep useful provider prompt prefixes warm only when the adapter can do so
  economically and report status honestly.
- Let the root agent manage direct children while preventing recursive agent
  trees.
- Keep the core functional with Rust alone while offering a first-party
  TypeScript extension experience.

### Non-Goals

- A daemon, distributed scheduling, or restart-durable background work.
- Full provider parity in the first release.
- Consumer-local forks of shared provider, loop, context, usage, event, or
  registry mechanisms.
- Guessing context limits for an unknown model.
- A committed sibling-path dependency in a released Smith manifest.
- A guarantee that a provider's hidden prompt cache exists.
- A cache rebuild request after a miss.
- Grandchild agents.
- Arbitrary untrusted native extensions.
- Full custom TUI layouts in the first extension API.
- Unsupported reuse of Codex subscription credentials.

## Architecture

```text
┌─────────────────────────────────────────────────────────────┐
│ Hosts                                                       │
│ smith TUI · smith -p · tests · future Forge adapter         │
└────────────────────────────┬────────────────────────────────┘
                             │ resolved Smith run request
┌────────────────────────────▼────────────────────────────────┐
│ Smith policy and composition                               │
│ config/provenance · prompt · transport · credentials        │
│ stores/journal · approval · workspace · tools · orchestration│
└────────────────────────────┬────────────────────────────────┘
                             │ RuntimeBuilder + injected traits
┌────────────────────────────▼────────────────────────────────┐
│ agent-runtime facade                                       │
│ model/context plan · provider/tool loop · registries        │
│ normalized events · usage · cancellation · tool executor    │
└─────────────────────────────────────────────────────────────┘
```

There is no IPC layer in the MVP. `smith`, `smith -p`, and Forge construct the
same `Runtime` in their own process. If a daemon is justified later, its
protocol can be proposed around the already-versioned runtime commands and
events.

## Planned Workspace Boundaries

| Crate/package | Responsibility | Must not own |
| --- | --- | --- |
| `agent-runtime` | Shared model/context/provider/tool loop, events, usage, registries, cancellation | Smith configuration, UX, credentials, or Forge types |
| `smith-config` (provisional) | Typed layers, provenance, profiles, trust classification, credential references | Provider I/O or runtime execution |
| `smith-runtime` (provisional) | Smith runtime factory, provider/transport/store composition, safe-boundary orchestration | A second agent loop or shared contract copies |
| `smith-host` | Approval policy and project workspace boundary | Provider protocol or TUI rendering |
| `smith-tools` | Smith's concrete read/list/search/edit/shell tools and later monitor tools | Tool scheduler or provider loop |
| `smith-extension` (future) | Extension protocol, TypeScript-host bridge, MCP, trust/capability checks | An embedded JavaScript VM |
| `smith-tui` | Ratatui state and rendering over shared events | Provider HTTP or persistence internals |
| `smith-cli` | Binary, argument/environment input, terminal host loop, and `-p` entry points | Runtime mechanism reimplementation |

`smith-config` and `smith-runtime` may begin as private modules if a separate
crate would be premature. Their dependency direction and public seams are
normative; the initial package count is not.

## Decision 1: Shared Runtime Is the Execution Owner

Smith constructs the shared runtime; it does not own a parallel provider loop.
One resolved Smith run maps to `RuntimeBuilder` with:

1. the selected provider and provider name;
2. a required model profile or model catalog;
3. context reserves, optional compaction, cache capability, loop limits,
   retry, and product instructions;
4. Smith tools plus approval and workspace policy;
5. Smith session/secret stores, event observers, and clock; and
6. a bounded event buffer and shutdown timeout.

Agent Runtime then plans every request, streams normalized events, validates
and executes tool calls, records usage, applies cancellation, and owns session
facade semantics. Smith consumes these events for its transcript, persistence,
machine output, and product orchestration.

Synthetic cache ping/pong text and UI-only notifications remain non-canonical.
Future safe-boundary inbox orchestration may start a new runtime turn or
execution phase; it MUST NOT mutate an in-flight provider request.

The initial built-in coding tool set remains read, list, search, edit, and
shell. These tools implement the shared `Tool` trait and receive the shared
invocation context rather than a Smith-local execution contract.

Agent Runtime revision `fe50d5a` made composed authorization live for tool
invocation while this change was being implemented. Until Smith has an
approved product-specific `SecurityCheck`, its one runtime factory explicitly
calls `legacy_approval_authority()`. That shipped compatibility check never
allows an effectful call itself: it requires Smith's existing approval policy
for filesystem writes, process spawning, and network effects. This preserves
the approved interactive/headless approval behavior without claiming that
provider egress, capability activation, or sub-agent dispatch are already
covered by the unfinished upstream security boundary.

## Decision 2: Provider Composition Uses Shared Adapters

The public provider contract is `agent_runtime::core::provider::Provider`.
Smith selects and constructs implementations of that contract; it does not
define `ProviderAdapter` or a second provider event vocabulary.

Agent Runtime 0.1 currently ships:

| Adapter | Current use |
| --- | --- |
| Deterministic fake | Unit, integration, replay, and development fixtures |
| OpenAI-compatible Chat Completions | First production vertical slice |

The OpenAI-compatible adapter owns request serialization and SSE
normalization, but deliberately accepts an injected `HttpTransport`. Smith owns
the concrete production transport, TLS/client configuration, credential
reference resolution, and endpoint allow/trust policy. It builds
`OpenAiConfig` only after resolving a `Secret` at the host boundary. Debug,
event, and diagnostic surfaces must not expose header values, bodies, or secret
contents.

OpenAI Responses and Anthropic Messages remain desired adapters, but they land
in Agent Runtime first with conformance coverage and then become selectable in
Smith after a compatible release. Smith MUST NOT create consumer-local versions
of shared provider mechanism merely to move that roadmap item earlier.

Provider/model switching occurs only at a safe turn boundary. Until the shared
facade supports in-place reconfiguration, Smith may save the session, construct
a new immutable runtime, and explicitly resume it with the same session ID.
The TUI warns that provider cache identity does not transfer.

Codex subscription remains a separate supported-surface spike. It must not
scrape credentials or depend on private endpoints.

## Decision 3: Session Persistence and Live Events

Agent Runtime supplies versioned events, `SessionSnapshot`, `SessionStore`,
`EventObserver`, and explicit `StartSession::with_id` resume semantics. Smith
implements those host contracts; it does not define another canonical message
or event schema.

Canonical runtime envelopes are appended to versioned JSON Lines files under:

```text
~/.smith/sessions/<project-id>/<session-id>.jsonl
```

The journal includes shared runtime events and Smith-owned orchestration events.
The Smith `SessionStore` persists compatible snapshots and run manifests. Large
tool output may use bounded sidecars referenced by content hash and metadata.

The runtime's bounded broadcast stream drives the active TUI, while an injected
observer writes the journal. Clean shutdown saves a snapshot. Crash-safe resume
requires either replaying the complete journal into a snapshot or an upstream
checkpoint hook; Smith MUST prove one of those paths before claiming that the
last in-flight turn survives a crash.

On a normal confirmed exit, Smith cancels children, terminates owned process
groups, records terminal events, flushes the session, and exits. On replay
after a crash, any previously running monitor or child is marked
`interrupted_by_process_exit`; it is not restarted.

SQLite remains an option if query or concurrency requirements outgrow the
append-only log. It is not part of the MVP.

## Decision 4: Configuration, Trust, and Secrets

Low-to-high precedence:

```text
built-in defaults
→ ~/.smith/config.toml
→ <project>/.smith/config.toml
→ <project>/.smith/config.local.toml
→ selected profile
→ SMITH_* environment variables
→ CLI flags
→ explicit per-session overrides
```

Every resolved value retains its source so `smith config explain <key>` can
show why it won. Unknown keys and incompatible provider settings fail with
actionable diagnostics.

An illustrative configuration shape is:

```toml
default_profile = "work"

[profiles.work]
provider = "acme"
model = "example-model"
max_output_tokens = 4096

[providers.acme]
kind = "openai-compatible"
base_url = "https://api.example.test/v1"
credential = "keychain:smith/acme"

[models."acme/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[context]
output_reserve = 4096
reasoning_reserve = 0
capability_budget = 12000
```

The example values are placeholders, not built-in claims about a real model.
The selected model's limits must come from an explicit Smith layer or a runtime
catalog source. Smith maps the resolved fields as follows:

| Resolved Smith data | Runtime composition |
| --- | --- |
| provider + model | `RuntimeBuilder::new`, `provider`, `provider_name` |
| model limits/catalog | `model_profile` or `model_catalog` |
| context reserves/budget | versioned `ContextPolicy`; optional compactor |
| prompt and loop tuning | `system_prompt`, retry, reasoning, output/tool/time limits |
| host policy | tools, approval, workspace, stores, observers, clock |
| credential reference | resolve to `Secret`, then construct provider config |

Startup order is strict: discover project, load declarative layers, select a
profile, validate and explain the resolved value, confirm executable project
trust where needed, resolve credentials, resolve a model profile, build the
provider and runtime, start/resume the session, then enter the terminal. A
configuration failure never leaves the user in the alternate screen and never
performs provider network I/O.

Repository-safe project content lives under `.smith/`, including config,
instructions, and extension manifests/modules. Session state, trust records,
credential material, and monitor output live under `~/.smith/`.

Reading declarative settings is allowed before trust, but executable project
extensions, hooks, shell-valued settings, and provider credential helpers are
blocked until the user confirms. Trust is recorded against the canonical
project path and the exact executable-content hash. A content change invalidates
the decision and prompts again.

Provider configuration stores secret references, never plaintext keys.
Credentials use the macOS Keychain or Linux Secret Service when available.
An explicit encrypted-file fallback stores ciphertext under `~/.smith/` and
requires a passphrase or a master key kept outside that file. Smith redacts
secrets from events, logs, command previews, and extension payloads.

## Decision 5: Usage Is Disjoint and Labelled

Agent Runtime owns the disjoint usage counters, attempt visibility, and usage
events. Every provider attempt—including retries and later child turns,
compaction, and cache keepalives—produces or updates the shared usage ledger:

```text
input_uncached
input_cache_read
input_cache_write
output_visible
output_reasoning
output_other
```

Each counter has one provenance:

```text
provider_reported
derived_from_provider_total
tokenizer_estimated
character_estimated
unknown
```

Unknown is never treated as zero. Smith consumes shared records and may add
product-owned endpoint labels, purpose attribution, and a versioned price-table
reference. It MUST NOT reinterpret or double count the shared token categories.
Costs are reported only when compatible counts and a price version support
them; otherwise Smith labels the value estimated or unknown.

The TUI shows current-turn and session totals plus cache read/write counts.
`smith -p --output-format json|stream-json` exposes the same records to callers.

## Decision 6: Monitor and Safe-Boundary Steering

`monitor` accepts exactly one source:

- `command: string`, run in the same environment and current-directory model as
  the shell tool; or
- `ws: { url: string, ... }`, where each WebSocket text frame is an event.

For commands, every stdout line becomes one raw event. The full stdout/stderr
stream is spooled to a bounded output file, but stderr does not emit
notifications unless the command explicitly merges it with `2>&1`.

Default `timeout_ms` is five minutes and the maximum is one hour.
`persistent: true` is mutually exclusive with `timeout_ms` and instead runs
until `TaskStop`, session shutdown, or source termination.

Lines received within a 200 ms window are bundled into one notification. A
configurable flood guard defaults to stopping a monitor after either 1,000 raw
events or 1 MiB of raw event payload within a rolling ten-second window. The
terminal notification explains that the monitor was stopped for noise and
points to its output file.

The tool documentation warns that every pipeline stage must flush per line and
that filters must include failure signatures as well as success markers.

Notifications appear in the TUI immediately. They never splice text into an
active provider stream or tool result. The session inbox queues and coalesces
them, then the agent consumes them before its next provider request or after the
current tool boundary. Terminal monitor events are never silently dropped.

## Decision 7: Cache Observation, Keepalive, and Compaction

Agent Runtime's context planner owns stable-prefix fingerprints, provider cache
capabilities, cache-plan revisions, preflight token accounting, and semantic
compaction. Smith supplies versioned policy and treats remote prompt caching as
a provider capability, not local state it controls. Each shared adapter
describes:

- unsupported, automatic-prefix, explicit-breakpoint, or explicit-resource
  caching;
- observable cache-read/cache-write fields;
- supported retention choices and refresh behavior;
- whether a synthetic keepalive can refresh the same stable prefix.

The shared plan fingerprints the exact stable prefix, provider, model, tool
schemas, system content, adapter/sizer revisions, and cache controls. Smith's
session-level policy may report `unsupported`, `unknown`, `eligible`,
`warm_observed`, `miss_observed`, and `suspended`. Only provider usage or a
provider cache API can produce an observed hit or miss.

The default policy targets one hour of inactivity. Meaningful activity is a
real user message or actual agent/provider/tool work. Passive monitor lines and
synthetic keepalives do not reset the clock; if a monitor notification causes a
real agent or tool turn, that work does reset it. Each child session has its own
clock.

When the adapter supports keepalive safely, Smith sends the exact cache anchor
plus a minimal ephemeral ping and accepts only a bounded pong response. The
adapter selects a provider-appropriate interval with jitter and suppresses the
ping when recent real activity has already refreshed the prefix. Ping and pong
are excluded from the canonical transcript, but their token cost and cache
observation are recorded.

Automatic-prefix adapters enable this behavior only after a conformance test
shows that the shorter future transcript prefix can reuse the refreshed anchor.
Otherwise Smith remains observation-only.

On an observed keepalive miss, Smith records the miss and suspends further
keepalives. It does not send a second prewarm or rebuild request. A later real
turn may naturally establish a new cache.

At the configured inactivity limit, Smith waits for a safe boundary, performs
one automatic compaction using the current session/provider policy, persists
the summary and usage, stops keepalives for the old prefix, and does not
prewarm the compacted prefix.

## Decision 8: Extensions Use a Process Boundary

Smith exposes one versioned, language-neutral protocol over stdio using framed
JSON messages. Extensions initialize, negotiate protocol/capability versions,
declare permissions, and register contributions. Extension crashes are isolated
from the Rust runtime and produce a visible diagnostic.

The first-party TypeScript host provides a Pi-like API:

```ts
export default async function (smith: ExtensionAPI) {
  smith.registerTool({ name: "deploy", /* ... */ });
  smith.registerCommand("stats", { /* ... */ });
  smith.on("tool_call", async (event, ctx) => { /* ... */ });
}
```

Initial contribution points are:

- tools, including explicit trusted replacement of a built-in;
- commands and keyboard shortcuts;
- lifecycle/provider/tool/session events;
- permission gates and path protection;
- compaction and summarization policy;
- provider registration through shared provider/registry contracts;
- basic declarative status-line items and widgets;
- MCP server/tool registration.

The host is an optional Node process. Smith's base binary starts without Node
and reports a clear dependency error only when a TypeScript extension is
enabled. Event ordering, timeouts, cancellation, payload limits, and failure
policy are deterministic.

Trusted compile-time Rust traits serve first-party and Forge integrations. WASM
Component/WASI is deferred until the protocol and permission model are stable;
it can later implement the same contribution protocol for portable sandboxed
tools. Arbitrary in-process dynamic libraries are excluded.

## Decision 9: Direct Children Only

The root agent receives child-management tools: spawn, list, send/follow-up,
wait, fetch result, and stop. A child receives the ordinary coding tools but
not child-management tools. Runtime authorization also rejects a spawn request
from depth one, so prompt injection cannot bypass the UI-level omission.

For each child, the parent must choose:

- task and expected result;
- provider/model and token/turn/deadline limits;
- workspace policy: shared project, explicit directory, isolated worktree, or
  read-only view;
- permission policy.

This is an agent decision surfaced in events rather than a hidden global
default. Write-capable shared workspaces display a conflict warning and tools
serialize overlapping writes where possible.

Child progress and final results enter the parent inbox using the same
safe-boundary rule as monitor events. Children are bounded by configurable
global/session concurrency and stop when their parent or Smith process stops.
Their canonical summaries and usage remain in the parent session log, but the
child process/task itself is not resumed.

## Decision 10: TUI and Non-Interactive Surfaces

Running `smith` opens a simple Ratatui application with:

- scrollable transcript and streaming assistant text;
- composer and command palette;
- tool calls, approval prompts, and result rows;
- provider/model selector;
- token, estimated-token, cache, and session status;
- monitor notifications and live-work list;
- one-level child-agent list and results;
- session create/resume and graceful-exit confirmation.

Root `DESIGN.md` defines the visual hierarchy, colors, focus states, motion,
and accessibility behavior for the implemented TUI slice. The MVP favors an
operational coding surface over a dashboard of panels.

Non-interactive examples:

```text
smith -p "explain this repository"
printf '%s' "fix the failing test" | smith -p -
smith -p "review" --output-format json
smith -p "run checks" --output-format stream-json
```

Human text goes to stdout and progress/diagnostics to stderr. JSON is one final
versioned result envelope; stream JSON is one versioned event per line.
Approval-required actions fail closed instead of hanging when no TTY is
available unless the caller supplied an explicit policy.

If background work remains at exit, the TUI asks for confirmation. Headless
callers select `error`, `wait`, or `stop` as the background-exit policy, with
`error` as the default. Smith targets macOS and Linux first.

## Decision 11: Forge Integration Is One-Way

Forge may depend on the shared `agent-runtime` facade plus Smith's
configuration/composition policy boundary, supply its own
tool/approval/workspace/store adapters, and consume shared typed events
in-process. Smith must not depend on Forge crates or task-state types.

This proposal may add a host-adapter example and contract tests inside Smith.
It does not modify Open Forge. A later Forge proposal will introduce Smith
behind an opt-in executor path, prove behavioral parity, and decide whether a
daemon is ever necessary.

## Failure and Shutdown Semantics

- Provider errors retain retryability, status, request ID, and redacted adapter
  detail.
- Tool and extension failures are returned as structured results and cannot
  corrupt the session log.
- Cancellation propagates from host to provider stream, tool process group,
  monitor, and child tasks.
- A confirmed shutdown stops accepting new work, cancels children, terminates
  process groups, records terminal events, flushes canonical state, and exits
  within a bounded grace period.
- A crash may lose non-canonical live UI events but must leave prior complete
  JSONL records parseable. Replay truncates only an incomplete final record and
  marks formerly active work interrupted.

## Testing Strategy

- Every runtime fixture supplies an explicit fake model profile; a missing
  profile test proves startup fails before provider I/O.
- Agent Runtime's Smith consumer conformance test remains a release gate.
- Smith integration tests cover the resolved-config-to-`RuntimeBuilder` mapping,
  text, tools, retries, usage, model-profile events, switching, and compaction.
- Provider contract fixtures cover the currently shared OpenAI-compatible
  adapter without live spend. Responses/Anthropic fixtures become Smith gates
  only after their shared adapters exist.
- Transport tests cover streaming bytes, HTTP/status classification,
  cancellation, deadlines, header/body redaction, and secret non-disclosure.
- Configuration tests cover precedence, provenance, unknown keys, invalid
  provider/model combinations, secret references, and model-limit failures.
- Persistence tests cover clean resume, event ordering, manifest retention,
  incomplete-tail recovery, and explicit crash-recovery limits.
- A controllable clock covers the inactivity window, provider-specific
  keepalive scheduling, jitter bounds, miss suspension, and safe-boundary
  compaction.
- Monitor tests cover line buffering, stderr isolation, `2>&1`, WebSocket text,
  batching, timeout, persistence, flood stop, and process-group cleanup.
- Extension tests cover negotiation, async initialization, permissions,
  deterministic ordering, timeout, crash isolation, and absent Node.
- Child tests cover depth enforcement, workspace selection, concurrency,
  steering, cancellation, and shutdown.
- Golden CLI/TUI tests cover output contracts, approvals, status provenance,
  resize/focus, and macOS/Linux behavior.
- Live-provider cache and subscription tests are opt-in, redacted, and
  explicitly spend-capped.

## Risks and Mitigations

| Risk | Mitigation |
| --- | --- |
| Runtime API moves while Smith uses a sibling path | Pin an exact released revision; use an uncommitted Cargo patch only for coordinated local work |
| Runtime has no published source yet | Keep the path during local development, but block release until a real remote/tag or exact revision exists |
| Desired provider adapter is not upstream yet | Land it with shared conformance first; do not fork provider mechanism in Smith |
| Model metadata is absent or stale | Require explicit limits or a validated layered catalog and fail before network I/O |
| Snapshot-only persistence loses a crash tail | Pair `SessionStore` with an append-only observer journal and prove reconstruction or add an upstream checkpoint seam |
| Provider APIs diverge | Shared capability descriptors, conformance fixtures, and explicit downgrades |
| Cache keepalive costs more than it saves | Provider-specific enablement, observed usage, spend limits, miss suspension, no second rebuild |
| TypeScript extensions gain broad host access | Separate process, explicit trust/hash, declared permissions, payload/time limits |
| Monitor output floods the session | 200 ms batching, bounded files/inbox, configurable flood stop |
| Shared child workspace conflicts | Parent-selected policy, visible warnings, write serialization, isolated-worktree option |
| JSONL becomes hard to query | Versioned events and repository abstraction allow later SQLite migration |
| Codex subscription has no supported direct provider path | Keep the spike isolated; use official app-server only as an external agent backend or report unsupported |
| TUI scope consumes integration work | Keep runtime/config composition headless and let the TUI reduce shared events only |

## Reference Material

- [Pi extension API](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/extensions.md)
  for the desired TypeScript authoring ergonomics and contribution breadth.
- [Codex plugins](https://developers.openai.com/codex/plugins/) and
  [Codex hooks](https://developers.openai.com/codex/hooks/) for process-based
  capabilities, approvals, and hash-bound trust.
- [Codex authentication](https://developers.openai.com/codex/auth/) and
  [Codex app-server](https://developers.openai.com/codex/app-server/) for the
  subscription technical spike's supported boundary.
- [Anthropic prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
  for explicit prefix breakpoints, observed read/write usage, and provider TTL
  behavior.
- [OpenAI prompt caching](https://platform.openai.com/docs/guides/prompt-caching)
  for exact-prefix matching, cache usage fields, model-specific retention, and
  explicit cache controls where supported.

## Rollout Order

1. Restore compatibility by supplying explicit fake model profiles everywhere
   and pass all current Smith tests plus shared Smith conformance.
2. Resolve Smith configuration with provenance and map it through one headless
   runtime factory.
3. Add a production streaming transport, credential resolution, and the shared
   OpenAI-compatible adapter; replace the interactive fake default.
4. Add Smith session snapshots/event journaling and explicit create/resume.
5. Pin a real Agent Runtime release/revision and move sibling paths into an
   uncommitted local Cargo override.
6. Add `smith -p`, provider/model selection, and configuration diagnostics over
   the same composition.
7. Add cache-lifetime orchestration, monitors, direct children, extensions,
   and MCP without duplicating shared mechanisms.
8. Consume future shared Responses/Anthropic/security releases through
   coordinated migration changes.
9. Add the Codex supported-surface spike and Smith-owned Forge embedding
   example.

Each phase must preserve a runnable vertical slice; no daemon work is included.
