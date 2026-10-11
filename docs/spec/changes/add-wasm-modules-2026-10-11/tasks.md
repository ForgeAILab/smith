---
created_at: 2026-10-11T01:38:05Z
updated_at: 2026-10-11T08:26:59Z
completed_at: 2026-10-11T08:26:59Z
---

## 1. Interface and host

- [x] 1.1 Define the `smith:module@0.1.0` WIT package: types, the `host`
  import interface, and the single `module` world.
- [x] 1.2 Add `crates/smith-wasm-host` behind the `wasm-modules` feature
  with a pinned `wasmtime`; measure stripped release size and cold build
  time with and without the feature and record both in `design.md`. Stop
  and report if either budget is exceeded.
- [x] 1.3 Engine, linker, component loading, on-disk compiled-component
  cache keyed by content digest and engine version.
- [x] 1.4 Per-call deadline, memory cap, and payload bounds; failure
  counting and session-scoped disabling.

## 2. Contract adapter

- [x] 2.1 `WasmModule` implementing `smith_module::Module`: `describe`,
  `mount`, inactive and failed outcomes.
- [x] 2.2 Tool contributions as `Arc<dyn Tool>` with declared effects, under
  the conservative authority rule used for MCP tools.
- [x] 2.3 Status items refreshed after each call into the module and served
  to clients from a host-side cache.
- [x] 2.4 Context-contributor and tool-output-processor contributions with
  bounded patches; a failing component contributes nothing.
- [x] 2.5 `ModuleTrust::Sandboxed` in `harness.rs`, evidence, and listings.

## 3. Capabilities

- [x] 3.1 No WASI: the host exposes only the `smith:module` imports; test
  that filesystem, network, environment, and wall-clock imports are refused.
- [x] 3.2 Host functions: log, fs read and write under granted roots, HTTP
  to granted hosts through the existing transport, key-value store,
  session history.
- [x] 3.3 An ungranted host function fails the call; requested and granted
  capabilities recorded on the module.

## 4. Packaging and surfaces

- [x] 4.1 `modules` in the `.smith-plugin/plugin.json` overlay; the `.wasm` bytes and
  capability request in the plugin digest and install inventory.
- [x] 4.2 A capability added by an update invalidates trust and is shown.
- [x] 4.3 `/modules` and `/plugins` show sandboxed modules with granted
  capabilities and failure state; switching through the existing paths.
- [x] 4.4 A build without `wasm-modules` lists such modules as not
  supported in this build.

## 5. Authoring

- [x] 5.1 `crates/smith-module-sdk`: Rust guest bindings and helpers.
- [x] 5.2 `examples/wasm-module-hello`: one tool, one status item, one
  capability; built to a component in CI and installed in an end-to-end
  test.
- [x] 5.3 Interface versioning test: a module built against the previous
  package version still loads (added with the second version; until then a
  test that an unknown version is refused with a clear error).

## 6. Verification

- [x] 6.1 Conformance tests: trap, timeout, memory exhaustion, oversized
  payload, malformed contribution, each leaving the session running.
- [x] 6.2 Architecture tests for the new crates; `cargo deny` clean.
- [x] 6.3 `docs/wasm-modules.md` (authoring, capabilities, limits) and
  README.
- [x] 6.4 Full gate with `--no-fail-fast`, with and without the feature.
