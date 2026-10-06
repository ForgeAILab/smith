---
created_at: 2026-10-06T21:41:35Z
updated_at: 2026-10-06T21:41:39Z
---

## Why

A `registry.search` row was drawn while the call ran and removed the moment
it succeeded, about 70 ms later. The owner saw text appear and vanish on
every prompt and read it as a broken capability display.

## What Changes

- `write_todos` and `registry.search` draw no transcript row while running,
  matching their suppression on success. Failed, denied, and unreported calls
  still render. `agent` rows are unchanged, because a running `wait` is
  useful to see.

## Impact

- Affected specs: `tool-call-display`
- Affected code: `crates/smith-tui/src/render/transcript/blocks.rs`
