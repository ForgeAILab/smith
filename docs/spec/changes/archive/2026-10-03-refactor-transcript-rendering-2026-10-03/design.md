## Context

`draw_synced` (`render/layout.rs`) calls `transcript_lines`, which builds
lines for every block (`block_lines` in `render/transcript.rs`), then
`visual_scroll_limit` wraps them all to count rows, `block_start_row` builds
and wraps every block before a result, and `draw_transcript` wraps all lines
again for a `Paragraph` scrolled by a `u16` offset. Tests mostly call the
pure `draw`; the driver calls `draw_synced`, which takes `&mut App` and
writes `scroll_limit`, `scroll_back`, `following`, `scroll_to_block`, and
`result_scroll_revision`.

Blocks are private to `Transcript` and change through about 16 sites in
`transcript.rs`. A block's rows depend only on the block, the width, the
fold state (`work_details`), and the theme. Blank-line separators between
visible blocks, reasoning and suppressed-row omission, speculative text, the
turn summary, the getting-started screen, and the child inspector are
decided outside a single block.

## Goals / Non-Goals

- Goals: frame cost proportional to what changed and what is visible; no
  `u16` ceiling on transcript scrolling; a draw that does not write state;
  byte-identical output.
- Non-Goals: changing wording or layout; caching overlays, the composer, or
  the panels; incremental Markdown parsing inside a streaming block.

## Decisions

- Decision: `Transcript` keeps a revision per block, bumped by one mutation
  accessor that every change goes through, so a new mutation site cannot
  forget it. History replacement resets all revisions.
- Decision: the cache maps block position to (revision, width, fold state,
  theme) and the wrapped rows. It lives in `App` behind interior mutability
  as render-only state, so the pure `draw(&App)` path used by tests can use
  it too. A key mismatch re-renders that one block.
- Decision: assembly walks blocks in order, skips the same blocks
  `block_lines` skips, inserts the same separators, and appends the
  uncached tails (speculative text, turn summary). The child inspector and
  the getting-started screen stay uncached.
- Decision: the visible window is cut from the assembled rows by `usize`
  offsets and drawn without a `Paragraph` scroll offset. `block_start_row` is
  a prefix sum over cached row counts.
- Decision: a pure layout function computes the scroll limit and resolves
  follow and scroll-to-block from `&App` and the area; the driver applies its
  result to the app, then draws with `&App`. `draw_synced` becomes that
  sequence or is replaced by it.

## Risks / Trade-offs

- A mutation that bypasses the accessor would draw stale rows. Mitigation:
  blocks stay private, the accessor is the only `&mut` path, and a property
  test compares cached and uncached output after every mutation kind.
- The cache holds a second copy of the rendered transcript. Rows are already
  rebuilt every frame today; holding them is a trade of memory for time.
