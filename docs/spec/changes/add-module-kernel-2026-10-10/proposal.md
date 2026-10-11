---
created_at: 2026-10-10T22:30:05Z
updated_at: 2026-10-11T00:41:52Z
---

## Why

Claude Code, Codex, OpenCode, pi, and DeepSeek Harness (dsh) all let people
extend the tool, and Smith does not. Without that it is hard to grow a
community. The owner prefers the dsh shape: every feature is a module mounted
beside the others, and a user composes the set they want.

Smith's optional features are wired straight into the runtime factory today,
so there is nothing a module could plug into. This change adds that seam and
proves it by moving real features onto it. It replaces
`add-feature-toggles-2026-10-05`, which only listed booleans over existing
keys and is removed with this proposal (0 of 13 tasks were done).

## What Changes

- Add a module contract: a module has an id, a revision, the modules it
  requires, and a `mount` step that returns its contributions as values.
  Contributions are tools, harness pipeline components, event observers,
  slash commands, and declarative status items.
- Contributions adapt to Agent Runtime's existing contracts (`Tool`, the six
  harness pipeline phases, `EventObserver`). Smith adds no second event bus.
- Mounting is reversible by construction: the runtime composition is rebuilt
  from the mounted set, so unmounting a module leaves nothing behind.
- Each first-party module is its own crate behind a cargo feature of
  `smith-cli`. Default features build all of them; a build without default
  features is a working minimal Smith.
- One explicit compiled-in module list in the composition root. A build that
  adds a third-party module crate to that list gets a trusted native module,
  shown as third-party wherever modules are listed.
- `[modules.<id>] enabled` selects which compiled-in modules mount, through
  the ordinary layered resolution (so a profile can carry its own set). A
  change applies at the next safe boundary, the same one `/model` uses.
- Port three features as proof, one per contribution kind:
  `image-generation` (tool), `budget-notice` (pipeline components), and a
  status item contributed by `budget-notice` (TUI surface).
- Add `/modules` and `smith config modules`: every known module with whether
  it is compiled in, mounted, where that choice came from, and its
  provenance. `/modules <id> on|off` edits the user configuration.
- Ported features keep their existing configuration key as an alias of the
  module switch, so no user configuration breaks.
- **BREAKING** for embedders only: `Contribution` and `ModuleProvenance` in
  `smith-runtime` gain variants, and a host that builds a `RuntimeRequest`
  directly gets neither image generation nor the budget notice unless it
  passes them in `RuntimeRequest.modules`.

## Out of Scope

Named follow-up changes, in the recommended order (see `design.md`):

- `add-plugin-bundles`: installable content bundles in the Claude Code
  layout (skills, commands, agents, MCP servers, hooks).
- `add-wasm-modules`: third-party modules loaded at runtime as WASM
  components, so end users need no Rust toolchain.
- `add-custom-builds`: a `smith build --with <crate>` helper that generates
  the compiled-in list.
- Porting the remaining features (idle compaction, resume capsule, handoff
  checkpoint, advisor, memory, MCP).
- Typed services shared between modules, and hot reload of code.

## Impact

- Affected specs: `extension-system`, `configuration`, `code-organization`,
  `client-surfaces`.
- Affected code: new `crates/smith-module` (contract) and
  `crates/modules/*` (one crate per module); `crates/smith-runtime`
  (`harness.rs`, `factory/`), `crates/smith-config` (`modules` table and
  resolution), `crates/smith-client` (`/modules`, status item data),
  `crates/smith-tui` (rendering), `crates/smith-cli` (compiled-in list,
  cargo features, `smith config modules`).
- New module crates fall under the existing "Crate boundaries are checked by
  structure" requirement; the architecture tests gain their rules.
- No Agent Runtime change is required. The pinned compatibility revision
  already exposes every contract used here.
