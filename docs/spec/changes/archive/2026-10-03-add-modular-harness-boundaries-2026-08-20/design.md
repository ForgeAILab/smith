## Context

`smith-runtime` currently centralizes product composition correctly, but the
input is a large mutable `RuntimeRequest` with many optional services and a
public `Vec<Arc<dyn Tool>>`. The crate re-exports Agent Runtime's
`SessionHandle` and delegation types, while presentation code consumes upstream
events directly. This prevents duplicate construction but exposes upstream
schema and treats arbitrary in-process Rust as if it were an extension
boundary.

## Goals / Non-Goals

- Goals:
  - Create one immutable, explainable product composition record.
  - Separate what modules contribute from what authority they receive.
  - Give TUI, GPUI, headless, Forge, and tests a stable Smith client protocol.
  - Make trusted native embedding and untrusted extensions impossible to
    confuse in types or documentation.
- Non-Goals:
  - No second model/tool loop, event journal, usage accountant, or checkpoint
    mechanism.
  - No stable Rust dynamic-library ABI.
  - No claim that a subprocess protocol is safe before its broker is running.
  - No behavioral redesign of the current TUI.

## Decisions

### Decision: resolve before composition

```text
HarnessSpec -> Resolver -> ResolvedHarness -> Smith factory -> SmithSession
```

`HarnessSpec` contains declarations. The resolver loads provenance, validates
trust, computes revisions and grants, resolves services, and produces an
immutable `ResolvedHarness`. The public factory accepts only that resolved
value. Direct tests may use explicit test builders that still produce a
`ResolvedHarness`; they do not bypass the invariant.

Continuing to grow `RuntimeRequest` was considered. Optional trait objects are
convenient for tests but cannot prove that policy is complete, resolved, or
attributed before composition.

### Decision: contributions and grants are independent records

```rust
struct ResolvedModule {
    id: ModuleId,
    revision: ModuleRevision,
    provenance: ModuleProvenance,
    trust: ModuleTrust,
    contributions: Vec<Contribution>,
    requested_capabilities: CapabilitySet,
    granted_capabilities: CapabilitySet,
}
```

A tool, command, skill, observer, or panel declaration does not imply a grant.
Executable contribution adapters receive only broker handles covered by the
computed grant. Content-only skills receive no executable handle.

### Decision: Smith projects, Agent Runtime remains canonical

`SmithSession` delegates submission, steering, cancellation, and event
subscription to one Agent Runtime handle. `SmithEvent` is a versioned client
projection, not a replacement journal or execution event. Canonical Agent
Runtime events remain the persisted source and conformance target.

The adapter preserves stable Smith IDs and bounded payload references. Unknown
upstream events are either intentionally projected to a versioned generic
diagnostic or ignored by an explicit mapping test; clients never deserialize an
upstream enum directly.

### Decision: execution tier is part of module identity

| Tier | Execution | Default trust |
| --- | --- | --- |
| Smith built-in / Forge-owned | in-process Rust | trusted |
| User-installed extension | capability-brokered process | untrusted |
| MCP server | process or network connection | explicit server trust |
| Skill | parsed content | no executable authority |
| UI panel | declarative data | no runtime or renderer handles |

`Arc<dyn Tool>` remains possible only inside the trusted-native tier. A user
manifest cannot select that tier. WASI may later implement the same contribution
protocol without changing client or runtime semantics.

### Decision: retain one public factory, split private stages

Physical modules mirror the resolution and assembly stages, but only one public
build function constructs Agent Runtime. The split is accepted only with
equivalence tests over the existing composition record.

## Risks / Trade-offs

- The Smith client projection creates another versioned schema to maintain. It
  buys product stability but must remain a projection, not a competing source
  of truth.
- Migrating direct embedders is a source-breaking change. A time-bounded adapter
  and deprecation window reduce disruption without keeping two composition
  paths.
- Contribution/grant modeling may expose missing Agent Runtime primitives. New
  reusable mechanisms must land upstream first under the existing compatibility
  gate.
- The active MCP change touches the same capability; unsynchronized deltas
  could encode conflicting registration assumptions.

## Migration Plan

1. Land Smith client and resolved-harness types behind adapters while preserving
   current entry points.
2. Move standard hosts and tests to `ResolvedHarness`, then freeze the old
   request adapter.
3. Migrate presentation code to `SmithEvent`, preserving fixtures.
4. Rename and isolate trusted-native injection; publish trust-tier docs.
5. Remove Agent Runtime public re-exports and the compatibility adapter only
   after downstream callers have a released migration path.

## Open Questions

- Which Smith client types should deliberately reuse stable Agent Runtime wire
  records versus use Smith newtypes? The rule is to reuse only contracts already
  promised independently of the concrete facade API.
- Whether Forge needs an in-process `SmithSession` adapter only or also a framed
  Smith client protocol should be decided by Forge's deployment boundary, not
  by the TUI implementation.
