---
created_at: 2026-10-16T23:20:00Z
updated_at: 2026-10-17T20:03:10Z
---

## Why

Three follow-ups to the shipped WASM module system were written on
2026-10-11 as task 2.4a of `add-wasm-modules`. They were left uncommitted in
the module-kernel worktree (preserved as branch
`wip/module-kernel-worktree-2026-10-11`, commit `663a5ed`), and main archived
that change without them. The owner chose on 2026-10-16 to land them before
any further runtime work.

- A module's id is `wasm.<hash>.<hash>`, which tells a user nothing in
  listings, status, or diagnostics.
- A context contributor that was not granted session history cannot run at
  all, though it may only need its own state.
- A view Smith itself cannot fit into the payload limits counts as a module
  failure, so a long session can disable a healthy module.

## What Changes

- A WASM module's id becomes `<plugin>/<component path without .wasm>` when
  that fits the module id alphabet and length. Otherwise it stays the hashed
  form.
- The old hashed id stays an alias for the same module. Install records,
  grants, and configuration that name it keep working without reinstalling,
  as the owner chose. Listings show the readable id.
- A context contributor without the session-history capability runs with
  an empty history instead of being refused.
- A view Smith cannot fit within payload limits skips that contribution for
  that call and does not count toward disabling the module.

## Impact

- Affected specs: `extension-system`
- Affected code: `crates/smith-plugin/src/modules.rs` (id derivation and
  alias), `crates/smith-wasm-host` (contributor views, failure counting),
  `docs/wasm-modules.md`
- Source to port: commit `663a5ed` on `wip/module-kernel-worktree-2026-10-11`
  (only the 2.4a parts, not the interface 0.2 work)
