## Context

`smith-module` defines the contract: a module has an id, a revision, and
requirements, and `mount(&ModuleContext)` returns contributions as values
(`Mounted::Contributions` or `Mounted::Inactive`). The factory adapts those
into Agent Runtime's `Tool`, the harness pipeline traits, and status
sources. Compiled-in modules implement the trait directly. This change adds
a second implementer: an adapter that forwards the trait to a WASM
component.

`../nyx/crates/nyx-plugin` (wasmtime 46) is the prior attempt. Its lessons
are recorded in `add-module-kernel/design.md`; in short, the interface shape
was the problem, not WASM. Zed's extension host (`../zed`, wasmtime, WIT
under `crates/extension_api/wit/since_v*`) is the working precedent for a
versioned interface.

## Goals / Non-Goals

- Goals:
  - A user installs a file and gets tools and context behaviour, with no
    toolchain and no ambient authority.
  - One interface that grows by versions, never by worlds.
  - The same listing, switching, and evidence as compiled-in modules.
- Non-Goals:
  - Matching what a compiled-in module can do.
  - Supporting every guest language.

## Decisions

### One world, mount returns contributions, calls are routed by id

```wit
package smith:module@0.1.0;

/// Bounds are protocol requirements, enforced by the future host adapter.
/// Unless overridden: strings <= 4096 UTF-8 bytes, lists <= 256 entries,
/// ids/revisions/names <= 128 bytes and nonempty. Each complete argument or
/// result <= 1 MiB including nested data. Oversize data fails, never silently
/// truncates. Unique ids/map keys/set entries are required. No ambient WASI.
interface types {
  /// Exact Module metadata, including selection default and mount dependencies.
  record descriptor {
    id: string,
    revision: string,
    description: string,
    default-enabled: bool,
    requirements: list<string>,
  }

  /// Exhaustive smith_runtime::harness::Capability vocabulary. Declaration is
  /// not authority; process/credential/approval/provider are never granted in v0.1.
  enum capability {
    workspace-read, workspace-write, host-read, host-write, process, network,
    external-read, external-write, data-egress, credential, approval, provider,
  }

  /// Opaque root aliases and exact DNS hosts, not host filesystem paths.
  /// Roots/hosts each <= 32 entries; aliases <= 128 bytes; hosts <= 253 bytes.
  record capability-grant {
    capability: capability,
    roots: list<string>,
    hosts: list<string>,
  }

  variant setting-value { text(string), boolean(bool), integer(s64) }
  record setting { name: string, value: setting-value }
  enum posture { read-only, read-write }

  /// Safe projection of ModuleContext. settings are this module's own map.
  /// Native user_dir/transport/credentials/image binding are never serialized:
  /// plugin-data-root is an opaque alias; HTTP/history use host imports.
  /// grants are host-owned ModuleSpec evidence, not guest-requested authority.
  record mount-context {
    settings: list<setting>,
    plugin-data-root: string,
    posture: posture,
    has-image-binding: bool,
    has-session-history: bool,
    semantic-summary-enabled: bool,
    max-input-tokens: u32,
    built-in-tools: bool,
    grants: list<capability-grant>,
  }

  /// Exact Effect fields; scope/service/endpoint strings obey the common bound.
  record host-write-effect { resource-kind: string, scope: string }
  record external-write-effect { service: string, scope: string }
  variant effect {
    read,
    write(string),
    host-read(string),
    host-write(host-write-effect),
    external-read(string),
    external-write(external-write-effect),
    spawn-process,
    network,
    network-to(string),
    data-egress(string),
  }

  /// Exact Agent Runtime Permission variants, including Other's separate identity.
  variant permission {
    fs-read, fs-write, fs-create, fs-delete, host-fs-read, host-fs-write,
    external-read, external-write, net-http, data-egress, credential-use,
    process-spawn, stdio-read, stdio-write, clock-read, random-read, other(string),
  }

  /// ToolSpec. parameters-json is JSON Schema, <= 64 KiB. effects and permission
  /// bound each <= 32 entries. The host adds the conservative unreviewed-tool
  /// authority floor; neither declaration can widen a manifest grant.
  record tool-spec {
    name: string,
    description: string,
    parameters-json: string,
    effects: list<effect>,
    permission-upper-bound: list<permission>,
  }

  /// ComponentDescriptor; ordering stays phase-local and runtime-owned.
  /// before/after each <= 32 ids; protected runtime.core.* ids cannot be owned.
  record component-spec {
    id: string,
    revision: string,
    before: list<string>,
    after: list<string>,
  }

  /// Names route calls within their contribution kind; no function-name strings.
  variant contribution {
    tool(tool-spec),
    context-contributor(component-spec),
    tool-output-processor(component-spec),
    status-item(string),
  }
  variant mounted { contributions(list<contribution>), inactive(string) }

  /// JSON arguments <= 1 MiB; attribution comes from ToolContext/PreparedToolCall.
  record tool-input {
    session: string,
    turn: string,
    request: string,
    call-id: string,
    arguments-json: string,
  }
  /// Exact serde representation of ToolOutcome (value, content, is_error),
  /// <= 1 MiB. Domain failures use is_error; invocation failures use module-error.
  type tool-output = string;

  enum error-kind { unknown-id, invalid-input, unavailable, failed }
  record module-error { kind: error-kind, message: string }
  record payload-limit { field: string, maximum: u32 }
  record http-failure { status: option<u16>, message: string }
  /// All host failures are recoverable typed results, including denied authority.
  variant host-error {
    capability-denied(capability),
    invalid-input(string),
    payload-too-large(payload-limit),
    not-found,
    unavailable(string),
    io(string),
    http(http-failure),
  }

  enum role { system, user, assistant, tool }
  record reasoning-producer { provider: string, model: string }
  record reasoning {
    text: string,
    redacted: bool,
    signature: option<string>,
    producer: option<reasoning-producer>,
  }
  record image { url: string, detail: option<string> }
  record tool-call { id: string, name: string, arguments-json: string }
  /// ContentPart's non-result cases. Payload text/URLs/signatures may use the
  /// aggregate 1 MiB budget; tool arguments remain JSON. Redacted reasoning
  /// must never carry original text/signatures across the boundary.
  variant leaf-content {
    text(string), reasoning(reasoning), image(image), tool-call(tool-call),
  }
  /// Canonical result content has one nesting level, <= 256 parts. Deeper
  /// nested ToolResult ContentParts are rejected rather than flattened/lost.
  record tool-result {
    call-id: string,
    name: string,
    content: list<leaf-content>,
    is-error: bool,
  }
  variant content-part { leaf(leaf-content), tool-result(tool-result) }
  record message { role: role, content: list<content-part> }

  /// Typed encoding of serde_json::Value for read-only component state, not a
  /// JSON string. Root is node 0; children point forward within nodes; each
  /// non-root has exactly one parent; all nodes reachable; depth <= 32.
  /// nodes <= 4096, child/member lists <= 256; numbers must be finite.
  record value-member { key: string, node: u32 }
  variant value-node {
    null, boolean(bool), signed(s64), unsigned(u64), float(f64),
    text(string), array(list<u32>), object(list<value-member>),
  }
  record value-tree { nodes: list<value-node> }
  enum state-sensitivity { redaction-safe, sensitive }
  record component-state {
    revision: string,
    sensitivity: state-sensitivity,
    value: value-tree,
  }

  /// ContextView. activation is the exact opaque Fingerprint string.
  /// history is exact bounded canonical history, <= 256 messages; if it
  /// cannot fit, this call fails. Access requires host-read/session-history.
  /// state is only the caller's VersionedSessionState, never another namespace.
  record context-view {
    session: string,
    turn: string,
    history: list<message>,
    activation: string,
    state: option<component-state>,
  }

  enum fragment-kind {
    system-instruction, developer-instruction, ability-instruction, tool-schema,
    history, tool-result, user-input, memory, retrieval, continuation, summary,
  }
  enum context-lane { instructions, capabilities, memory, conversation, tail-context }
  record context-position { lane: context-lane, sequence: u64 }
  variant fragment-source { host, ability(string), history, tool, provider, compactor }
  enum requirement { required, optional }
  enum sensitivity { public, internal, sensitive, secret }
  enum cache-class { stable, ephemeral, no-cache }
  record tool-schema { name: string, description: string, parameters-json: string }
  variant fragment-content { text(string), message(message), tool(tool-schema) }
  /// ContextFragment's exact fields; content text uses the aggregate bound,
  /// schema <= 64 KiB. pairings are a unique set. Source/placement/kind are
  /// declarations subject to host validation, not authority to forge history.
  record context-fragment {
    id: string,
    kind: fragment-kind,
    position: context-position,
    source: fragment-source,
    revision: string,
    requirement: requirement,
    priority: s32,
    content: fragment-content,
    pairing: option<string>,
    pairings: list<string>,
    conversation-group: option<string>,
    sensitivity: sensitivity,
    cache-class: cache-class,
    token-hint: option<u32>,
  }
  /// ContextPatch; <= 64 fragments, aggregate <= 1 MiB.
  record context-patch { fragments: list<context-fragment> }

  enum usage-source { provider-attempt, tool-loop, external-agent, rollup }
  enum attempt-purpose {
    ordinary, cache-keepalive, cache-handoff-checkpoint, idle-compaction,
    cache-resource-create, cache-resource-extend, cache-resource-inspect,
    cache-resource-delete,
  }
  enum counter-kind { input-uncached, input-cached, cache-write, output, reasoning }
  record usage-counter { kind: counter-kind, value: u64 }
  /// Read-only Provenance projection: private CacheIdentity becomes its exact
  /// digest (not reconstructible from guest input). Other fields copy directly.
  record usage-provenance {
    request: option<string>,
    attempt: option<string>,
    tool-call: option<string>,
    purpose: option<string>,
    attempt-purpose: option<attempt-purpose>,
    cache-identity-digest: option<string>,
    failed: bool,
  }
  /// UsageRecord; counters are a unique set of <= 5 nonzero entries.
  record usage-record {
    source: usage-source,
    provenance: usage-provenance,
    counters: list<usage-counter>,
  }
  /// ToolOutputView plus the separate ToolOutcome argument of process().
  /// usage <= 256 exact records. now-monotonic-ms is host-relative elapsed
  /// time, deliberately withholding native Timestamp's wall-clock origin.
  record tool-output-view {
    session: string,
    turn: string,
    request: string,
    call: tool-call,
    state: option<component-state>,
    usage: list<usage-record>,
    now-monotonic-ms: u64,
    outcome-json: tool-output,
  }
  /// Narrow ToolOutputPatch: adapter uses state=None and events=[] in v0.1.
  /// Persistent state mutations and event projections await a later version.
  record tool-output-patch { outcome-json: tool-output }

  enum status-severity { info, warning, error }
  /// Exact StatusItem, <= 80 Unicode scalar values in label (not bytes).
  /// name must equal the registered status id; no renderer handles.
  record status-item { name: string, label: string, severity: option<status-severity> }

  enum log-level { trace, debug, info, warning, error }
  /// Relative confined path <= 4096 bytes; root alias <= 128 bytes.
  record file-path { root: string, path: string }
  record file-write { file: file-path, bytes: list<u8> }
  /// HTTP uses Smith's transport. name <= 128 bytes, value <= 4096 bytes;
  /// <= 64 headers. Host-owned credentials are never exposed.
  record http-header { name: string, value: string }
  enum http-method { get, head, post, put, patch, delete, options }
  record http-request {
    method: http-method,
    url: string,
    headers: list<http-header>,
    body: list<u8>,
  }
  record http-response { status: u16, headers: list<http-header>, body: list<u8> }
  /// Key <= 128 bytes, value <= 64 KiB; isolated in this plugin's data store.
  record kv-entry { key: string, value: list<u8> }
  /// A bounded recent suffix in canonical order, never a second history store.
  record history-snapshot { messages: list<message> }
}

/// Every function returns result<_, host-error>; grants are checked per call.
/// fs/HTTP byte lists <= 1 MiB including surrounding record; redirects require
/// fresh endpoint checks. No function grants arbitrary host path access.
interface host {
  use types.{log-level, host-error, file-path, file-write, http-request,
             http-response, kv-entry, history-snapshot};
  /// Always available; <= 4096-byte message, host bounded/redacted.
  log: func(level: log-level, message: string) -> result<_, host-error>;
  /// workspace-read for workspace roots; host-read for plugin-data root.
  fs-read: func(file: file-path) -> result<list<u8>, host-error>;
  /// workspace-write for workspace roots; host-write for plugin-data root.
  fs-write: func(write: file-write) -> result<_, host-error>;
  /// network with exact granted host; sending task data also requires data-egress.
  http: func(request: http-request) -> result<http-response, host-error>;
  /// host-read scoped to plugin-data; missing keys return ok(none).
  kv-get: func(key: string) -> result<option<list<u8>>, host-error>;
  /// host-write scoped to plugin-data.
  kv-set: func(entry: kv-entry) -> result<_, host-error>;
  /// host-read scoped to session-history; only current session, limit 1..256.
  session-history: func(limit: u32) -> result<history-snapshot, host-error>;
}

/// One world, mandatory exports; unregistered ids return unknown-id errors.
world module {
  use types.{descriptor, mount-context, mounted, module-error, tool-input,
             tool-output, context-view, context-patch, tool-output-view,
             tool-output-patch, status-item};
  import host;
  export describe: func() -> descriptor;
  export mount: func(ctx: mount-context) -> result<mounted, module-error>;
  export call-tool: func(id: string, input: tool-input) -> result<tool-output, module-error>;
  export contribute-context: func(id: string, view: context-view) -> result<context-patch, module-error>;
  export process-tool-output: func(id: string, view: tool-output-view) -> result<tool-output-patch, module-error>;
  /// Host refreshes each registered id after mount and each tool/pipeline call;
  /// None removes the cached item. Rendering reads the cache, never the guest.
  export status: func(id: string) -> option<status-item>;
}
```

The block above is the exact `wit/smith-module.wit` package. Every export exists in
every module; a module that contributes no tools returns an error from
`call-tool`, which the host never calls because nothing was registered. This
is what removes nyx's world-per-combination problem.

- Tool parameters and results are JSON (a tool's schema is JSON Schema, so
  this is the native form). Everything else uses typed records.
- Adding a contribution kind or a host function is a new package version.
  The host keeps bindings for the current and previous version and adapts
  both to `smith_module::Module`.

### Version 0.1 projections and bounds

The WIT comments define explicit byte/list/depth limits; implementing their
validation is task 1.4, not part of this gate. The aggregate limit is 1 MiB;
status labels use Smith's existing 80 Unicode scalar value limit. Component
ordering copies `ComponentDescriptor` fields without inventing priorities.
`ToolSpec` carries every effect and the distinct typed permission upper bound;
tool arguments and the exact serialized `ToolOutcome` are JSON strings.

Native services, user paths and provider credentials in `ModuleContext` never
cross the ABI. Mount receives opaque plugin-data root aliases and host facts.
The existing `HostRead`/`HostWrite` capabilities are scoped to plugin-data for
filesystem/KV and `HostRead` to current-session history. Workspace file access
uses `WorkspaceRead`/`WorkspaceWrite`; HTTP uses exact-host `Network` and task
content transmission additionally uses `DataEgress`. Scopes are host-owned
manifest/grant evidence, not new `Capability` enum members. Context history
requires the same history grant; absence is an error rather than an implicit
history grant. No broker or grant enforcement is implemented in this gate.

Typed history preserves roles and content, including tool calls/results and
reasoning metadata. Native recursively nested tool results deeper than one
level are refused; no silent flattening occurs. Read-only component state uses
a typed, bounded value tree because WIT does not permit recursive types.
Usage provenance exposes the opaque `CacheIdentity` digest, and pipeline time
is elapsed monotonic time without the native timestamp's wall-clock origin.
Version 0.1 `ToolOutputPatch` replaces only the exact outcome: its future
adapter must set `state = None` and `events = []`. This withholds persistent
state mutation and event emission until a later version; component state may
still be read from its own namespace. Each status id is refreshed after mount
and each tool/pipeline call; status refresh does not recursively refresh itself.

### What a WASM module can contribute in the first version

| Kind | Why now |
|---|---|
| Tool | The most wanted extension point; maps to `Arc<dyn Tool>` |
| Context contributor | Lets a module add bounded context; patch type is already bounded |
| Tool-output processor | Lets a module post-process a tool's result |
| Status item | The declarative TUI surface |

History projector, tool-view resolver, model interceptor, and turn-commit
hook are held back: they can change what the provider sees or persist
component state, and should follow once the first four have real users.

### No ambient authority; capabilities are host functions

Version 0.1 links no WASI imports or `wasmtime-wasi` dependency: there are
no preopened directories, environment, sockets, or wall clock. The `host` import offers:

- `log(level, message)`: always available, bounded and redacted.
- `fs-read(root, path)`, `fs-write(root, path, bytes)`: only for roots named
  in a granted `workspace_read`, `workspace_write`, or plugin-data grant.
- `http(request)`: only to hosts named in a granted `network` capability;
  goes through Smith's existing HTTP transport and egress rules.
- `kv-get`, `kv-set`: a store in the plugin's data directory.
- `session-history(limit)`: the read-only service compiled-in modules get.

These map onto the existing `Capability` set and the
`requested_capabilities` / `granted_capabilities` fields of `ModuleSpec`. A
tool contributed by a WASM module still declares its effects and goes
through the ordinary approval path; the sandbox limits the module, approval
limits the tool.

### New trust tier

`ModuleTrust::Sandboxed`, provenance `UserManifest(plugin)`. This is the
first tier where "requested capability" is enforced by construction: an
ungranted host function returns `host-error.capability-denied`, naming the
existing harness capability. It performs no side effect.

### Limits

Per call: an epoch-based deadline (default 5 s for tools, 500 ms for
pipeline components and `mount`, 5 ms for `status`), a memory cap (default
64 MiB per instance), and payload bounds on every string and list. Three
consecutive failures disable the module for the session and mark it failed
in `/modules`. A pipeline component that fails contributes nothing for that
phase and the turn continues.

### Instances

One instance per module per session, created at mount and dropped at the
safe-boundary rebuild. Calls into one instance are serialized. The host calls
`status` after `mount` and after each call into the module and caches the
result, so rendering reads the cache and never enters the guest.

### Packaging through the plugin bundle

```json
{ "modules": [ { "wasm": "./module.wasm",
    "capabilities": { "network": ["api.example.com"],
                      "workspace_read": true } } ] }
```

in `.smith-plugin/plugin.json`, the overlay file `add-plugin-bundles`
reserves for Smith-specific keys. Nothing is added to the Claude Code
manifest. The plugin digest covers the `.wasm` bytes and the capability
request. Other tools reading the same plugin see an
ordinary content plugin.

### Dependency cost and the feature flag

`wasmtime` is the largest dependency Smith would have. nyx pins 46 and Zed
36; the gate selects `=36.0.17` (the patched Rust-1.88-compatible LTS line), with
default features off and only `component-model`, `cranelift`, `runtime`, and
`std`. Bindings and calls are synchronous; async is not needed for this gate.
No Winch, profiling, cache, WAT parser, or WASI is enabled. Task 1.2
measures release binary size and clean build time with and without
`wasm-modules`; the budget is +12 MB stripped and +90 s cold build. If the
measurement exceeds either, the feature ships default-off and the question
returns to the owner before the remaining tasks.

Compiled components are cached on disk under the plugin store, keyed by the
`.wasm` digest and the engine version, so startup does not recompile.

### Removing the subprocess and TypeScript requirements

Both are unbuilt. Keeping them would leave two specified-but-absent tiers
beside the one that exists. Out-of-process tools remain possible through
MCP, which is built, trusted, and already namespaced. If a subprocess tier
is wanted later it gets a new proposal against the then-current contract.

## Risks / Trade-offs

- Interface churn. Mitigated by versioned packages and two-version support,
  and by starting with four contribution kinds.
- Guest authoring friction outside Rust. Accepted; stated in the docs.
- A JS-based guest carries its own engine (nyx's Node example is 11.5 MB).
- Serialized calls mean one slow tool blocks that module's other calls.
- wasmtime security updates become Smith's release concern.

## Migration Plan

1. WIT package, host bindings, and the size and build-time measurement.
2. Adapter to `smith_module::Module` for tools and status items; limits.
3. Capability-gated host functions.
4. Pipeline contributions.
5. Plugin manifest field, install confirmation, listings.
6. Guest SDK, example module, CI build, docs.

## Found during implementation

- No WASI is linked. The host exposes only the `smith:module` imports, and a
  component importing any `wasi:*` interface is refused at load. Guests
  target `wasm32-unknown-unknown`.
- A sandboxed module's tools take their authority from the same rule as
  unreviewed MCP tools; what the guest declares about its own effects is
  ignored for approval.
- Module HTTP goes through a narrow `ModuleHttp` service on the module
  context, over the HTTP client Smith already has. Every redirect is
  refused, bodies are capped at 1 MiB, and a request needs both the network
  and the data-egress capability.
- Capability keys: `workspace_read`, `workspace_write`, `plugin_data_read`,
  `plugin_data_write`, `session_history`, `network`, `data_egress`. Grants
  are stored with the installed plugin and admitted only with the confirmed
  digest. Only the user's global plugin table can narrow them.
- A module's id is `wasm.` plus hashes of the plugin name and component
  path. It is stable across installs but not readable; a friendlier id is a
  follow-up.
- Tool schemas must be self-contained (no `$ref`). Module log text is
  withheld; only level and length are recorded. Calls into one module are
  serialised. Semantic-summary usage records cannot be represented in
  interface 0.1.
- Test guests and the example are committed as prebuilt components. Local
  rebuilds were byte-identical twice, but cross-machine reproducibility is
  unproven, so CI builds fresh components and tests against those.
- Three consecutive failures disable a module for the session; health is
  kept across rebuilds and tracked separately for child sessions.

## Open Questions

- Default limits (5 s tool deadline, 64 MiB): configurable per plugin by
  the user, or fixed in the first version? Proposed: fixed, with a user
  override table later.
- Should a WASM tool be allowed to declare itself read-only, or should all
  WASM tools start as unreviewed like MCP tools? Proposed: same
  conservative rule as MCP tools.

## Measurements

### Dependency gate (2026-10-11)

Newest means newest non-yanked stable release by semver; `50.0.0-rc.1` is a
prerelease and was not considered. Candidates were read from the crates.io API
and declared manifest MSRVs; `cargo info` additionally verified 49.0.2, 38.0.4,
and 36.0.17. No workspace MSRV or Agent Runtime git pin was changed.

| Candidate | Declared Rust MSRV | Decision |
|---|---|---|
| 49.0.2 | 1.96.0 | Reject: above workspace 1.88 |
| 48.0.5 | 1.95.0 | Reject: above workspace 1.88 |
| 47.0.4 | 1.94.0 | Reject: above workspace 1.88 |
| 46.0.3 | 1.94.0 | Reject: above workspace 1.88 |
| 45.0.3 | 1.93.0 | Reject: above workspace 1.88 |
| 44.0.3 | 1.92.0 | Reject: above workspace 1.88 |
| 43.0.2 | 1.91.0 | Reject: above workspace 1.88 |
| 42.0.2 | 1.91.0 | Reject: above workspace 1.88 |
| 41.0.4 | 1.90.0 | Reject: above workspace 1.88 |
| 40.0.4 | 1.89.0 | Reject: above workspace 1.88 |
| 39.0.2 | 1.89.0 | Reject: above workspace 1.88 |
| 38.0.4 | 1.88.0 | `cargo +1.88.0 check -p smith-wasm-host` passed; deny advisories failed |
| 37.0.3 | 1.87.0 | deny advisories failed; did not spend time compiling a rejected pin |
| **36.0.17** | **1.86.0** | **Chosen `=36.0.17`: Rust 1.88 check and all four deny checks passed** |

38.0.4 and 37.0.3 each fail the same 18 advisories:
`RUSTSEC-2026-0006`, `RUSTSEC-2026-0020`, `RUSTSEC-2026-0021`,
`RUSTSEC-2026-0085`, `RUSTSEC-2026-0086`, `RUSTSEC-2026-0087`,
`RUSTSEC-2026-0088`, `RUSTSEC-2026-0089`, `RUSTSEC-2026-0091`,
`RUSTSEC-2026-0092`, `RUSTSEC-2026-0093`, `RUSTSEC-2026-0094`,
`RUSTSEC-2026-0095`, `RUSTSEC-2026-0096`, `RUSTSEC-2026-0114`,
`RUSTSEC-2026-0222`, `RUSTSEC-2026-0269`, `RUSTSEC-2026-0316`.
Bans, licenses and sources pass for both rejected pins. No exceptions were
added. 36.0.17 is the maintained LTS release patched for these advisories
([upstream release policy](https://docs.wasmtime.dev/stability-release.html));
this gate therefore does not require an MSRV bump. The newest stable release
would require Rust 1.96. Feature-specific unreachability does not bypass deny.

Enabled features: `component-model`, `cranelift`, `runtime`, `std`, with
`default-features = false`. Epoch interruption is a runtime configuration,
not a Cargo feature. No WASI, async, Winch, profiling, cache or WAT feature.
Cargo.lock contains some inactive optional Wasmtime packages (including
Winch); they are not in the activated CLI dependency graph.

Commands used for each candidate (substitute the exact candidate in the root
workspace dependency before checking; the final manifest retains 36.0.17):

```sh
export CARGO_HOME="$PWD/target/wasm-gate/cargo-home"
export CARGO_DENY_HOME="$PWD/target/wasm-gate/deny-home"
cargo info wasmtime@49.0.2
cargo info wasmtime@38.0.4
cargo info wasmtime@36.0.17
cargo +1.88.0 check -p smith-wasm-host
cargo deny check
```

The isolated writable Cargo home was prepared inside this worktree because
the sandbox cannot write the shared home cache. Cache preparation/downloads
are excluded from release timing. Logs and scratch outputs are under ignored
`target/wasm-gate/`; no source or build output was written outside the worktree.

### Release comparison method

Host: aarch64 Apple Darwin, macOS 27.0 (26A5416b), `rustc +stable`
1.97.1 (8bab26f4f 2026-07-14), Cargo's default job count and default release
profile. This is a single-machine, single-run affected-package cold comparison,
not a statistical estimate or a completely empty-workspace build. All source
packages were already cached. No checks or other agent builds ran concurrently
with either timed build; other machine activity was not controlled.

To keep the main target directory intact, `cargo clean --release -p` removes
only affected packages. An untimed baseline build first prepares unrelated
release dependencies in the writable Cargo-home layout (Cargo reported
3m 41s; this preparation was not timed with `/usr/bin/time`). The timed
feature build cleans `smith-cli`, `smith-wasm-host`, and every additional
active transitive package. Cargo reported that `clean -p name@version` ignores
the version qualifier and removes all versions of that package. The initial
CLI-only timed baseline (28.16 s) was therefore discarded. The final timed
baseline applies the **same complete clean command** as the feature build,
including shared older versions affected by Cargo's cleaning behavior. Order
of final timed runs: with feature, then without feature. Shared unaffected
artifacts are reused in both. Feature-induced recompilation of
shared crates, if any, remains part of the measured feature cost. There is no
production component cache or precompiled-engine workaround in this comparison.

`smith --wasm-engine-diagnostic` actually compiles and instantiates the embedded
binary component, calls its exported function and verifies 42. WIT bindings
also compile, but adapters/brokers remain unimplemented. Stripping uses macOS
`strip -o`, identically for both binaries. Budgets are interpreted as decimal
MB (12,000,000 bytes) and elapsed wall-clock seconds; MiB is also reported.

### Results and decision

| Metric | Without WASM | With WASM | Added |
|---|---:|---:|---:|
| Stripped `smith`, bytes | 38,757,496 | 47,153,120 | **8,395,624** (8.40 MB / 8.01 MiB) |
| Unstripped `smith`, bytes | 51,001,328 | 62,845,728 | 11,844,400 |
| Cold affected-package wall time, seconds | 126.66 | 140.41 | **13.75** |
| Cargo.lock package entries added | — | **65** | 64 external packages + smith-wasm-host; 0 removed |

**Both budgets pass:** 8,395,624 < 12,000,000 bytes; 13.75 < 90 seconds.
The final `smith-cli` default retains `wasm-modules`. This completes the cost
and interface gate only; task 1.3 and all subsequent work remain unstarted.
Cargo parallelism overlaps engine compilation with shared recompilation, so
the time delta describes this paired build, not isolated compiler CPU cost.

The orchestrator's original default baseline was 50,906,528 unstripped and
38,658,432 stripped bytes. The measured feature-free binary here is
+99,064 stripped bytes relative to that reference; gate deltas use the
paired builds above, not the earlier reference.

Exact final clean command, applied before **each** timed build (package names
were obtained by diffing `cargo tree -p smith-cli --prefix none --format '{p}'`
with the same command adding `--no-default-features --features
module-image-generation,module-budget-notice`, then including smith-cli):

```sh
cargo +stable clean --release \
  -p arbitrary@1.5.0 \
  -p bumpalo@3.20.3 \
  -p cobs@0.3.0 \
  -p cranelift-assembler-x64@0.123.17 \
  -p cranelift-assembler-x64-meta@0.123.17 \
  -p cranelift-bforest@0.123.17 \
  -p cranelift-bitset@0.123.17 \
  -p cranelift-codegen@0.123.17 \
  -p cranelift-codegen-meta@0.123.17 \
  -p cranelift-codegen-shared@0.123.17 \
  -p cranelift-control@0.123.17 \
  -p cranelift-entity@0.123.17 \
  -p cranelift-frontend@0.123.17 \
  -p cranelift-isle@0.123.17 \
  -p cranelift-native@0.123.17 \
  -p cranelift-srcgen@0.123.17 \
  -p foldhash@0.1.5 \
  -p gimli@0.32.3 \
  -p hashbrown@0.15.5 \
  -p id-arena@2.3.0 \
  -p leb128fmt@0.1.0 \
  -p libm@0.2.16 \
  -p mach2@0.4.3 \
  -p object@0.37.3 \
  -p postcard@1.1.3 \
  -p pulley-interpreter@36.0.17 \
  -p pulley-macros@36.0.17 \
  -p regalloc2@0.12.2 \
  -p rustc-hash@2.1.3 \
  -p smith-wasm-host@0.3.10 \
  -p target-lexicon@0.13.5 \
  -p termcolor@1.4.1 \
  -p unicode-xid@0.2.6 \
  -p wasm-encoder@0.236.1 \
  -p wasmparser@0.236.1 \
  -p wasmprinter@0.236.1 \
  -p wasmtime@36.0.17 \
  -p wasmtime-environ@36.0.17 \
  -p wasmtime-internal-asm-macros@36.0.17 \
  -p wasmtime-internal-component-macro@36.0.17 \
  -p wasmtime-internal-component-util@36.0.17 \
  -p wasmtime-internal-cranelift@36.0.17 \
  -p wasmtime-internal-jit-icache-coherence@36.0.17 \
  -p wasmtime-internal-math@36.0.17 \
  -p wasmtime-internal-slab@36.0.17 \
  -p wasmtime-internal-unwinder@36.0.17 \
  -p wasmtime-internal-versioned-export-macros@36.0.17 \
  -p wasmtime-internal-wit-bindgen@36.0.17 \
  -p wit-parser@0.236.1 \
  -p smith-cli@0.3.10
```

Exact timed build and size commands, run from the workspace root after
exporting the Cargo-home variables above. The with-feature command ran while
`wasm-modules` was in default (as it remains in the final manifest):

```sh
# After the clean command: with feature.
/usr/bin/time -p -o target/wasm-gate/time-with.txt \
  cargo +stable build --release -p smith-cli
strip -o target/wasm-gate/smith-with.stripped target/release/smith
stat -f '%z' target/release/smith target/wasm-gate/smith-with.stripped
./target/release/smith --wasm-engine-diagnostic

# After the same clean command: without feature, keeping the other defaults.
/usr/bin/time -p -o target/wasm-gate/time-without.txt \
  cargo +stable build --release -p smith-cli --no-default-features \
  --features module-image-generation,module-budget-notice
strip -o target/wasm-gate/smith-without.stripped target/release/smith
stat -f '%z' target/release/smith target/wasm-gate/smith-without.stripped
./target/release/smith --wasm-engine-diagnostic
```

Build stdout/stderr were redirected respectively to
`target/wasm-gate/build-with.log` and `build-without.log`.
Both diagnostic branches ran: with feature reported `wasmtime 36.0.17;
smith:module@0.1.0; component-model; epoch interruption`; without feature
reported `WASM modules are not supported in this build`.

Lockfile count compared `(name, version, source)` tuples against an untouched
pre-change copy saved as `target/wasm-gate/Cargo.lock.before`; no preexisting
package versions were replaced. Inactive optional packages count in Cargo.lock
but do not count toward the 49 additional active CLI packages.

### Gate validation

All of the following passed in the final worktree (using the isolated Cargo
home above). Clippy is scoped to the new host and the affected CLI, including
all their targets; the whole workspace test suite was intentionally not run.
The Rust 1.88 host test compiles/instantiates/calls the binary fixture and
verifies its answer. Existing architecture tests and deny policy are untouched;
manifest inspection confirms the engine dependency exists only in the host.
No tasks.md checkbox was changed.

```sh
cargo +1.88.0 check -p smith-wasm-host
cargo +1.88.0 test -p smith-wasm-host
cargo +1.88.0 clippy -p smith-wasm-host -p smith-cli --all-targets -- -D warnings
cargo +stable clippy -p smith-wasm-host -p smith-cli --all-targets -- -D warnings
cargo +1.88.0 fmt --all --check
cargo +stable fmt --all --check
cargo deny check
wasm-tools component wit crates/smith-wasm-host/wit/smith-module.wit
wasm-tools validate crates/smith-wasm-host/tests/fixtures/diagnostic.wasm
python3 /Users/mai1015/.codex/skills/spec-toolkit/scripts/spec_toolkit.py validate add-wasm-modules-2026-10-11 --type change --strict
git diff --check
```

Fixture regeneration (no WAT feature or generator dependency in production):

```sh
wasm-tools parse crates/smith-wasm-host/tests/fixtures/diagnostic.wat \
  -o crates/smith-wasm-host/tests/fixtures/diagnostic.wasm
```
