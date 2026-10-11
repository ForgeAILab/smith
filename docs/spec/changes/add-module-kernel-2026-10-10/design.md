## Context

The owner wants dsh-style modularity: "everything is a plugin", features
selected at build time by cargo features and at run time by configuration.
dsh (github.com/deepseek-ai/deepseek-harness) gets this from Cordis, a
TypeScript framework where plugins claim named services, listen to typed
events with a fixed dispatch mode, and register reversible effects. Its
plugins run in-process with no isolation.

Three facts shape what carries over to Smith:

- Rust has no safe way to load third-party code into a running process, and
  `extension-system` already forbids native dynamic libraries.
- `extension-system` requires executable contributions to adapt to Agent
  Runtime's shared contracts "rather than a Smith-local parallel contract".
- Agent Runtime already has the interception seams. `EventObserver` is
  observe-only, but `harness/pipeline.rs` defines six ordered phases with
  typed, bounded patches: `HistoryProjector`, `ContextContributor`,
  `ToolViewResolver`, `ModelInterceptor`, `ToolOutputProcessor`, and
  `TurnCommitHook`. Each component carries a `ComponentDescriptor` (id,
  revision, ordering constraints) and the pipeline orders them
  topologically. Smith already uses these for `budget_notice`, `advisor`,
  `tool_output`, `prompt`, and `reasoning`.

## Goals / Non-Goals

- Goals:
  - One contract that a first-party feature, a compiled-in third-party
    crate, and later a WASM module can all implement.
  - Features a user can leave out of a build and switch off in configuration.
  - A list that shows every module and where it came from.
- Non-Goals:
  - Runtime loading of third-party code (follow-up `add-wasm-modules`).
  - Installable content bundles (follow-up `add-plugin-bundles`).
  - Sandboxing compiled-in modules. They are ordinary Rust in the user's
    binary.
  - Hot reload of code. dsh has it; a compiled binary cannot.
  - A general service locator shared between modules.

## Decisions

### A module returns values; it does not register side effects

`mount(&ModuleContext) -> Result<Mounted, ModuleError>`. `Mounted` is a list
of contributions. The composition root collects them and builds the runtime
once. Unmounting means rebuilding without that module, on the existing
safe-boundary reconfigure used by `/model` and `/agent`.

- Why: dsh needs reversible effects because plugins mutate a live context.
  Returning values gives the same guarantee with no disposer bookkeeping and
  no partially mounted state.
- Alternative considered: a registrar object with `register_*` methods and
  disposers. Rejected as more code for the same result.

### Contributions map to Agent Runtime contracts

| Contribution | Adapts to |
|---|---|
| Tool | `Arc<dyn Tool>` through the shared registry |
| Pipeline component | one of the six `harness::pipeline` traits |
| Observer | `EventObserver` (observe-only) |
| Command | Smith's single slash-command definition table |
| Status item | bounded declarative data rendered by the client |

`Contribution` in `harness.rs` gains `Pipeline { phase, component }` and
`StatusItem { name }`; `Tool`, `Command`, `Observer` exist. No Smith event
bus is added.

- Gap versus dsh: there is no pre-tool-execute or prompt-submit waterfall.
  Approval stays with `ApprovalPolicy`. Hooks on those points are part of
  `add-plugin-bundles`, where they are needed, and may need an Agent Runtime
  change on the compatibility branch. Nothing in this change needs one.

### Explicit compiled-in list

One function in `smith-cli` returns the compiled-in modules, each line gated
by `#[cfg(feature = "module-<id>")]`. Third-party crates are added as extra
lines.

- Why: readable, no new dependency, no link-time ordering surprises, and a
  build tool can generate the file later (`add-custom-builds`).
- Alternative considered: distributed registration (`inventory`/`linkme`).
  Rejected: hidden registration, platform linker quirks, and nothing a
  generated list cannot do.

### Contract lives in a new `smith-module` crate

Module crates depend on `smith-module` and Agent Runtime, not on
`smith-runtime`, so a module cannot reach factory internals and a
third-party crate has a small surface to track. `smith-runtime` consumes the
mounted contributions.

### The module context carries host services

A module crate cannot import `smith-runtime` or `smith-config`, so `mount`
receives what it needs as values: the module's own resolved settings, the
user directory, the session posture (read-only or not), an HTTP transport,
and the provider's image binding when one exists. The context grows one
named field at a time as ports need them; it is not a general service
locator.

`image-generation` shows the boundary. The tool (`GenerateImageTool`) is in
`smith-tools` and the backend (`image_api.rs`, `image_history.rs`) imports
only `smith-tools` and Agent Runtime, so both move to the module crate.
Resolving the provider's image binding stays in `factory/provider.rs`,
because it belongs to provider selection; the factory hands the result to
the context. There is no provider contribution kind in this change.

### Provenance and trust

`ModuleProvenance` gains `CompiledThirdParty(String)` (the crate name).
Trust tier stays `TrustedNative`. Smith makes no claim that such code is
mediated; the user's protection is that `/modules` and `smith config modules` name it. This matches the owner's position:
whoever compiles code in is responsible for what they put in.

### Configuration

`[modules.<id>] enabled = true|false`. A module's built-in default is
declared by the module. Ported features keep their existing key as the same
switch (`tools.image_generation.enabled` is `modules.image-generation.enabled`),
so no existing configuration breaks; setting both to different values in one
layer is an error naming both keys. Because resolution is the ordinary
layered one, a profile can carry its own module set, which is Smith's
equivalent of a dsh profile.

The alias follows the existing idle-compaction alias rule: same-layer
disagreement is an error, across layers normal precedence wins. This
reverses the removed proposal's "no second key" rule on purpose: a module
needs one uniform switch, and old keys must keep working.

A module that is switched on but not compiled in is reported, never silently
ignored.

### Module dependencies

A module names the module ids it requires. Mount order is topological. A
module whose requirement is absent or off does not mount and is reported
with the reason. Typed service sharing (dsh's `ctx.<key>` plus `inject`) is
left out until a ported feature needs it.

### First ports

| Module | Kind exercised | Why this one |
|---|---|---|
| `image-generation` | tool, host services through the context | already optional; backend has no `smith-runtime` imports |
| `budget-notice` | turn-commit hook, context contributor | already a pipeline component with no Smith imports |
| `budget-notice` status item | declarative TUI surface | new UI, not a port: shows the user the warning the model sees |

Idle compaction spans `cache_controller`, `cache_lifecycle`, `summary`, and
`resume_capsule`. It is the stress test and comes in a later change, after
the contract has survived these three.

### WASM (owner's question)

WASM removes the end user's compile step: authors build a `.wasm` component,
users install a file, and it is sandboxed by default. Zed ships extensions
this way on `wasmtime`. The existing "Multiple extension tiers" requirement
already allows it.

Costs: a `wasmtime`-class dependency in every build that enables it, and a
frozen interface (WIT) that must be versioned. Zed's tree carries five
versions of theirs (`since_v0.3.0` to `since_v0.8.0`). A WASM module gets
tools, pipeline components, commands, and declarative status items, not a
renderer handle, so rich TUI drawing stays with compiled-in modules.

Prior attempt: `../nyx/crates/nyx-plugin` (wasmtime 46). It was hard to
extend because of its interface shape, not because of WASM. Its world makes
one tool export mandatory, so a plugin is one tool; each optional interface
doubles the worlds (`plugin`, `plugin-hooks`, `plugin-ext`,
`plugin-hooks-ext`) and the host tries each in turn; hooks are five fixed
functions, so a new hook changes the interface for every plugin; payloads
are JSON strings; the package has no version; and the manifest declares no
capabilities. The contract in this change avoids those by construction: one
world, one `mount` that returns a list of contributions, and a call routed
by contribution id, so a new contribution kind is a new variant on a
versioned interface instead of a new world.

Recommendation, not decided here: make WASM the first runtime tier for
third-party code, ahead of the TypeScript subprocess host. It should wait
for this change so the interface is frozen against a contract that real
features already use. Consequence to weigh: `add-wasm-modules` would then
modify or remove the "Versioned subprocess extension protocol" and "Optional
TypeScript authoring host" requirements, unless both tiers are kept. `Mounted` is kept to plain data plus trait objects with
serializable inputs and outputs so a WASM adapter can implement it.

## Risks / Trade-offs

- The contract may be wrong for the harder features. Mitigation: three ports
  of different kinds now, idle compaction before any external freeze.
- More crates lengthen cold builds slightly and add manifest upkeep.
- A minimal build is a new supported configuration that CI must exercise.
- `/features` from the removed proposal becomes `/modules`; no shipped
  behavior changes because it was never built.

## Migration Plan

1. Land the contract and the composition path with no modules; behavior is
   unchanged.
2. Port one feature per step, each behind its existing key, with the
   existing fixtures passing unchanged.
3. Add `/modules` and the headless listing last, once there is something to
   list.

## Found during implementation

- The budget notice placed its warning in `ContextLane::TailContext` with
  `FragmentKind::DeveloperInstruction`. The pinned Agent Runtime driver
  (`agent/driver/mod.rs`, `validate_contributed_fragment`) accepts only
  `Continuation` in that lane, so the notice would have failed the first
  time it fired. The port uses `Continuation`. No production session was
  affected: the CLI never enables semantic summary, so the notice was, and
  still is, dormant there; `/modules` now shows it as inactive with that
  reason.
- Module crates import the six pipeline traits from the full Agent Runtime
  facade, the only place they are exported. The architecture test that
  limits facade use therefore names `smith-module` and each module crate.
  Follow-up: admit `crates/modules/*` structurally, or re-export the traits
  from `smith-module`.
- `smith config modules` needs a resolvable provider, like
  `smith config explain`; a fresh install with no provider cannot list
  modules yet.

## Open Questions

- WASM or the TypeScript subprocess host as the code tier for third-party
  modules. Not needed for this change; decide when `add-wasm-modules` is
  proposed, after `add-plugin-bundles` shows what bundles (command hooks plus
  MCP servers) already cover for script authors.

## Resolved

- `budget-notice` stays on whenever semantic summary is on (owner,
  2026-10-10).
- User-facing words: `module` for compiled-in pieces, `plugin` for
  installable bundles (owner, 2026-10-10).
