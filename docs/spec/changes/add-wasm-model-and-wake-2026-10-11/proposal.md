---
created_at: 2026-10-11T21:09:05Z
updated_at: 2026-10-17T06:42:09Z
---

## Status (2026-10-16)

This change was written and partly implemented on 2026-10-11. The work was
never committed to main: it was preserved, unbuilt, as commit `663a5ed` on
branch `wip/module-kernel-worktree-2026-10-11`. It is re-filed here so it is
no longer stranded. The owner chose on 2026-10-16 to port it on top of the
agent-runtime one-shot call API, so that `complete` uses the same library
path as the advisor, the semantic summary, and the vision tool instead of
another hand-written provider loop. The task checkboxes below are reset,
because none of this code is on main. The WIP's tests are the acceptance
list for the port.

## Why

The first real WASM plugin, `examples/wasm-module-relay`, showed two limits of
interface 0.1. A module cannot ask a model anything, so an advisor, a
summariser, or a reviewer cannot be a plugin without carrying its own API
key. And a module only runs when the session calls it, so a message sent to
an idle session waits until its user types something.

## What Changes

- Add interface version `smith:module@0.2.0`. Smith keeps loading 0.1
  modules unchanged.
- Add a `complete` host function: one bounded, non-streaming, tool-free
  model request on the session's current route (provider/model or installed
  agent). Installed agents retain their own generation limits. The module
  never sees a credential. It needs a new `model` capability, requested in
  the manifest and confirmed at install like every other capability.
- Bound model spend per module: a cap on output tokens per call and on calls
  per turn, both narrowable by the user. Usage is recorded under the
  module's name and included in session totals.
- Add a `watcher` contribution kind and a `poll` export. While a session is
  live Smith polls each watcher at a bounded interval; the notices it
  returns are delivered through the session inbox that monitor
  notifications already use, so an idle session takes a turn and a busy one
  receives them at the next safe boundary. It needs a new `session_notify`
  capability.
- Update the relay example to deliver messages with a watcher, and add a
  `second-opinion` example that uses `complete` with `session_history`. It
  asks the session's own model; unlike the built-in advisor it cannot route
  to a stronger one.

## Out of Scope

- Replacing compaction. That needs a history-projector contribution and a
  model call from a pipeline phase, which this change refuses; both belong
  to a separate change.
- Streaming, tool use, or a model other than the session's current one.
- Blocking or rewriting a tool call before it runs.
- UI beyond the status item.
- Waking a session whose process is not running.

## Impact

- Affected specs: extension-system, usage-accounting, configuration
- Affected code: `crates/smith-wasm-host` (WIT 0.2, 0.1 adapter, broker,
  watcher scheduling), `crates/smith-module` (model service on
  `ModuleContext`, watcher contribution), `crates/smith-module-sdk`,
  `crates/smith-plugin` (capability keys), `crates/smith-runtime` (model
  service, inbox delivery, usage attribution), `crates/smith-cli`
  (capability review text, listings), `docs/wasm-modules.md`, `examples/`
