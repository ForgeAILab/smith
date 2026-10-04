---
created_at: 2026-10-03T00:00:00Z
updated_at: 2026-10-04T00:58:36Z
---

## Why

Every frame rebuilds every transcript block's lines, wraps all of them to
count rows, wraps them all again to draw, and re-renders the blocks before a
result to find where it starts. Nothing is cached, the work grows with the
whole conversation, and frames repeat every 100 ms while a turn runs. Scroll
offsets are `u16`, so a transcript past 65,535 rows cannot scroll to its
start. The interactive draw also writes scroll state into the application.
These are findings S9 and S7 and roadmap step 5 in the
[2026-10-02 audit](../../../qa/smith-structure-2026-10-02/report.md); the
streamed-Markdown part of step 5 already shipped in v0.3.1.

## What Changes

- Each block's wrapped rows are cached by the block's revision, the width,
  the fold state, and the theme. Only blocks that changed are rendered again;
  the open streaming block and speculative text render every frame.
- Drawing takes only the rows in view. A result's start row and the scroll
  limit come from cached row counts.
- Transcript scroll offsets become `usize`.
- Layout and scroll synchronisation become a separate step that runs before
  drawing; drawing reads the application without changing it.
- Output is unchanged: every existing render test and fixture stays as it is.

## Impact

- Affected specs: client-surfaces.
- Affected code: `smith-tui` `transcript.rs` (per-block revisions),
  `render/transcript.rs`, `render/layout.rs`, `render/wrap.rs`, `app/state.rs`
  and `app/input.rs` (scroll fields), the driver's draw call in `smith-cli`
  `tui_driver.rs`.
- No change to wording, layout, persistence, or provider behaviour.
