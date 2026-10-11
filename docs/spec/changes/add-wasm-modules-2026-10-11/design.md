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

interface types {
  record descriptor { id: string, revision: string, description: string,
                      requirements: list<string> }
  variant contribution {
    tool(tool-spec),
    context-contributor(component-spec),
    tool-output-processor(component-spec),
    status-item(string),
  }
  variant mounted { contributions(list<contribution>), inactive(string) }
}

world module {
  import host;                       // capability-gated functions
  export describe: func() -> descriptor;
  export mount: func(ctx: mount-context) -> result<mounted, string>;
  export call-tool: func(name: string, input: tool-input) -> result<tool-output, tool-error>;
  export contribute-context: func(id: string, view: context-view) -> result<context-patch, string>;
  export process-tool-output: func(id: string, view: tool-output-view) -> result<tool-output-patch, string>;
  export status: func(name: string) -> option<status-item>;
}
```

(Sketch; the exact records are settled in task 1.1.) Every export exists in
every module; a module that contributes no tools returns an error from
`call-tool`, which the host never calls because nothing was registered. This
is what removes nyx's world-per-combination problem.

- Tool parameters and results are JSON (a tool's schema is JSON Schema, so
  this is the native form). Everything else uses typed records.
- Adding a contribution kind or a host function is a new package version.
  The host keeps bindings for the current and previous version and adapts
  both to `smith_module::Module`.

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

The WASI context has no preopened directories, no environment, no sockets,
and no wall clock. The `host` import offers:

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
ungranted host function traps the call.

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
36; this change pins one current release with default features off and only
the component-model, Cranelift, and async features it needs. Task 1.2
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

## Open Questions

- Default limits (5 s tool deadline, 64 MiB): configurable per plugin by
  the user, or fixed in the first version? Proposed: fixed, with a user
  override table later.
- Should a WASM tool be allowed to declare itself read-only, or should all
  WASM tools start as unreviewed like MCP tools? Proposed: same
  conservative rule as MCP tools.
