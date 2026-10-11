---
created_at: 2026-10-11T01:38:05Z
updated_at: 2026-10-11T01:38:05Z
---

## Why

After `add-module-kernel` a module needs a Rust toolchain and a custom
build, and after `add-plugin-bundles` a plugin can carry only content. There
is still no way for someone to ship code that a user installs as a file.
The owner chose WASM for that tier over the TypeScript subprocess host the
spec currently describes: the user installs nothing else, the code is
sandboxed by default, and calls stay in-process.

An earlier attempt (`../nyx/crates/nyx-plugin`) was hard to extend because
of its interface shape: one mandatory tool per plugin, a separate world for
every combination of optional interfaces, five fixed hook functions, and no
version. This change uses the module contract instead, which already avoids
those.

## What Changes

- Add a WASM component host. A WASM module implements the same contract as
  a compiled-in module: it describes itself, and `mount` returns a list of
  contributions. Smith calls a contribution by its id.
- One versioned interface package, `smith:module`, with one world. A new
  contribution kind is a new variant on the next interface version, not a
  new world. Smith supports the current and the previous interface version.
- Contribution kinds in the first version: tools, context contributors,
  tool-output processors, and status items. The other pipeline phases,
  observers, and commands follow in later interface versions.
- A WASM module has no ambient authority: no filesystem, no network, no
  environment, no clock beyond monotonic time. It gets host functions only
  for capabilities its manifest requests and the user granted: reading or
  writing under named roots, outbound HTTP to named hosts, a key-value
  store in the plugin's data directory, and read access to recent session
  history.
- A WASM module ships inside a plugin bundle. `.smith-plugin/plugin.json`
  names the `.wasm` file and the capabilities it requests. Install, trust,
  enable, update, and remove are the plugin's; the install confirmation
  shows the requested capabilities, and a new capability on update
  invalidates trust.
- Every call into a module is bounded in time, memory, and payload size. A
  trap, timeout, or oversized payload fails that call, disables the module
  for the session after repeated failures, and never ends the session.
- WASM modules appear in `/modules` and `/plugins` as sandboxed, with their
  granted capabilities, and are switched like any other module.
- The host is behind a `smith-cli` cargo feature, `wasm-modules`, on by
  default. A build without it reports WASM modules as not supported.
- A Rust guest SDK crate and one example module, built in CI.
- **BREAKING (spec only)**: remove the unbuilt "Versioned subprocess
  extension protocol" and "Optional TypeScript authoring host" requirements.
  MCP remains the way to attach an out-of-process tool server.

## Out of Scope

- Rendering in the TUI beyond status items. Rich rendering stays with
  compiled-in modules.
- Provider registration, permission-gate replacement, and replacing
  built-in tools from a WASM module.
- A JavaScript or Python guest SDK. Those languages can target the
  interface with their own component toolchains; Smith does not ship or
  test that path here.
- Hot reload of a module during a turn.
- A registry or signing scheme for WASM modules.

## Impact

- Affected specs: `extension-system`, `configuration`, `code-organization`.
- Affected code: new `crates/smith-wasm-host` (engine, linker, limits, WIT
  bindings, adapter to `smith_module::Module`), new
  `crates/smith-module-sdk` (guest side), `crates/smith-plugin` (manifest
  field and capability inventory), `crates/smith-runtime` (new trust tier,
  capability brokers), `crates/smith-cli` (feature, catalog), `examples/`.
- New dependencies: `wasmtime` and `wasmtime-wasi` (large; see `design.md`
  for the size budget and the measurement task), `wit-bindgen` for the SDK.
- Depends on `add-module-kernel` and `add-plugin-bundles`.
