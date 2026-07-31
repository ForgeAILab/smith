---
created_at: 2026-07-29T01:34:31Z
updated_at: 2026-07-29T01:57:16Z
---

## Why
Smith's protected tool rows reveal only argument key names, so a user can see
that `read` or `list` ran but not which file or directory it targeted. Opting
the runtime into raw tool arguments would fix the UX by weakening the event and
journal redaction boundary, which is not an acceptable trade.

## What Changes
- Add a bounded, tool-specific projector for safe invocation summaries of
  Smith's built-in tools.
- Enrich only the local TUI from the canonical in-process tool call, keyed by
  call ID; keep raw arguments disabled in runtime events, journals, and machine
  output.
- Render one Claude-style invocation row such as `Read(src/lib.rs) · ok` or
  `List(. · recursive) · ok`, while keeping tool-result content out of the
  transcript.
- Reconstruct the same safe summary when resuming canonical history and retain
  the existing protected-key fallback for unknown tools or unavailable calls.
- Keep arbitrary content-bearing values—including edit bodies, shell commands,
  and search patterns—out of the summary.

## Impact
- Affected specs: `tool-call-display`
- Affected code: `crates/smith-tools`, `crates/smith-runtime`,
  `crates/smith-cli`, and `crates/smith-tui`
- Shared Agent Runtime event schemas and persisted Smith session schemas remain
  unchanged.
