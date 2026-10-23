---
created_at: 2026-10-23T21:37:12Z
updated_at: 2026-10-23T21:37:12Z
---

## Why

A model can only learn an artifact ID from text in its context, and the most
important source is gone after one turn: a child-completion delivery names its
reply artifact but never enters canonical history. In `session-1878584b` a
gemini-4-argon root invented about sixty artifact IDs, and every
`artifact.read` failed with a bare `artifact not found` that offered no way to
find a real one, so it guessed again. Smith already keeps owner-checked
metadata for every artifact it stores, so it can list them itself; only a
stored child reply needs Agent Runtime to record which child wrote it.

## What Changes

- Add an `artifact.list` tool, registered beside `artifact.read` wherever
  Smith's own artifact store backs the session (root and child runtimes). It
  returns metadata only, never content, for the calling session's artifacts,
  newest first, in bounded pages, optionally filtered by purpose or child.
  Each row carries the ID, purpose, media type, byte length, originating turn
  and call, the child that produced it, and the source session of a copied
  child artifact.
- Agent Runtime names the child on a stored child reply
  (`add-artifact-child-attribution-2026-10-23`); Smith pins that change and
  resolves copied child artifacts through the session's own children.
- Listing reuses the `artifact.read` permission and the `session-artifact`
  resource kind, so it stays unattended exactly like reading.
- `artifact.read` keeps Runtime's ownership and integrity checks, but Smith
  rewrites its `artifact not found` error to say that IDs are never
  constructed and to name `artifact.list`, when listing is registered.
- The CLI bridge treats `artifact.list` like `artifact.read`: neither is
  bridged unless another supporting tool is.
- Root delegation guidance points a model that lost an ID to `agent result`.

## Impact

- Affected specs: `context-working-set`
- Affected code: `crates/smith-runtime/src/artifact.rs` (owner listing),
  `crates/smith-runtime/src/tool_output.rs` (lister, reader error),
  `crates/smith-runtime/src/factory/capabilities.rs`,
  `crates/smith-runtime/src/delegation.rs` (child registration),
  `crates/smith-runtime/src/host/startup.rs`,
  `crates/smith-runtime/src/abilities.rs`,
  `crates/smith-runtime/src/cli_bridge.rs`,
  `crates/smith-runtime/src/prompt.rs`
- Agent Runtime pin moves to the attribution change on the 0.7 line.
