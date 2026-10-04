---
created_at: 2026-10-04T00:00:00Z
updated_at: 2026-10-04T05:22:50Z
---

## Why

Three items deferred from `adopt-claude-code-grammar` (tasks 3.3b, 4.5b,
4.6c) are still visible:

- The agent inspector and `/agents` print debug values: `ReadOnlyView` for
  the child's workspace and `resumable false` for its checkpoint.
- First-run setup's review and configuration-collision preview bodies are cut
  off when they are taller than the frame, so the user confirms a change they
  cannot fully read.
- A Markdown table still arriving at the end of a streaming answer is
  re-measured on every delta, so its earlier rows jump as wider cells arrive.

## What Changes

- The inspector and `/agents` name the workspace in the words the spawn row
  already uses and say whether an exact resume is available in words.
- Setup's review and collision previews scroll with the arrow and page keys;
  the footer keys stay visible.
- A table still arriving at the end of a streaming answer shows its header
  and a dim "receiving table" line; it is drawn in full once it is complete.
  Earlier rows never move while it streams.
- OSC 8 links (also in 3.3b) stay deferred: the renderer cannot emit them
  without bypassing its cell buffer.

## Impact

- Affected specs: client-surfaces.
- Affected code: `smith-client` `agent_report.rs`; `smith-tui`
  `render/transcript.rs` (inspector), `setup.rs`, `render/markdown.rs`.
- Fixtures re-recorded for the agent inspector and `/agents` only.
